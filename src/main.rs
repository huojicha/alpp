//! CLI 入口：参数 → 配置发现 → 文件收集 → 并行处理 → 聚合输出（diff/write/check）→ 退出码。

use alpp::cli::Args;
use alpp::config::{self, Config, Loaded};
use alpp::error::FileError;
use alpp::eval::{MacroTable, MacroValue};
use alpp::pipeline;
use alpp::profile::{self, Profile};
use alpp::rewriter;
use alpp::walker::{self, FileJob};
use clap::Parser;
use rayon::prelude::*;
use std::io::Write as _;
use std::path::{Path, PathBuf};

fn main() -> std::process::ExitCode {
    let args = Args::parse();
    match run(&args) {
        Ok(code) => code,
        Err(msg) => {
            eprintln!("alpp: {msg}");
            std::process::ExitCode::from(1)
        }
    }
}

/// 诊断输出上限：超出部分折叠为一行汇总（M2 诊断聚合增强，DESIGN §4.7）。
const MAX_PRINTED_ERRORS: usize = 50;

/// 宏表构建失败：用法错误（退出 1）。
enum TableError {
    Usage(String),
}

fn run(args: &Args) -> Result<std::process::ExitCode, String> {
    let paths = if args.paths.is_empty() {
        vec![PathBuf::from(".")]
    } else {
        args.paths.clone()
    };

    // `.alpp` 配置发现：以第一个输入路径为锚向上查找；--no-config 跳过（SPEC §10）
    let discovered: Option<Loaded> = if args.no_config {
        None
    } else {
        config::discover(&paths[0]).map_err(|e| format!("config: {e}"))?
    };

    let table = match build_macro_table(args, discovered.as_ref()) {
        Ok(t) => t,
        Err(TableError::Usage(m)) => return Err(m),
    };

    // --profile 提前校验，并强制应用到目录遍历的全部文件（含未知扩展名）
    let force = args
        .profile
        .as_ref()
        .map(|name| {
            profile::by_name(name)
                .ok_or_else(|| format!("unknown profile `{name}` (expected c_family|hash|lua)"))
        })
        .transpose()?;

    let col = walker::collect(&paths, args.max_size, args.no_ignore, force)
        .map_err(|e| format!("collect: {e}"))?;

    // 线程池：默认核数，-j 覆盖（DESIGN §4.7）
    let mut pool_builder = rayon::ThreadPoolBuilder::new();
    if let Some(n) = args.threads {
        pool_builder = pool_builder.num_threads(n);
    }
    pool_builder
        .build_global()
        .map_err(|e| format!("thread pool: {e}"))?;

    let results: Vec<(FileJob, Result<pipeline::Outcome, FileError>)> = col
        .jobs
        .par_iter()
        .map(|job| {
            let outcome = process_one(job, args, &table, discovered.as_ref());
            (job.clone(), outcome)
        })
        .collect();

    // 确定性输出：按路径排序（DESIGN §4.7）
    let mut errors: Vec<&FileError> = results
        .iter()
        .filter_map(|(_, r)| r.as_ref().err())
        .collect();
    errors.sort_by(|a, b| (&a.path, a.line).cmp(&(&b.path, b.line)));

    let mut changed: Vec<(&FileJob, &pipeline::Outcome)> = results
        .iter()
        .filter_map(|(j, r)| r.as_ref().ok().filter(|o| o.changed()).map(|o| (j, o)))
        .collect();
    changed.sort_by(|a, b| a.0.path.cmp(&b.0.path));

    let mut stdout = std::io::stdout().lock();
    let mut had_io_error = false;

    for e in errors.iter().take(MAX_PRINTED_ERRORS) {
        if e.code == 0 {
            had_io_error = true;
        }
        eprintln!("{e}");
    }
    if errors.len() > MAX_PRINTED_ERRORS {
        eprintln!(
            "alpp: ... and {} more errors",
            errors.len() - MAX_PRINTED_ERRORS
        );
    }

    if args.check || args.gate {
        // 校验/门禁模式：只报错，不改写不 diff（gate 残留检测在退出码处）
    } else if args.write {
        for (job, outcome) in results
            .iter()
            .filter_map(|(j, r)| r.as_ref().ok().map(|o| (j, o)))
        {
            // 镜像模式写出全部已处理文件（树完整性，未变更文件原样复制）；
            // 原地模式只写变更文件。
            if outcome.changed() || args.output.is_some() {
                if let Err(e) = write_back(&job.path, &outcome.output, args.output.as_deref()) {
                    had_io_error = true;
                    eprintln!("alpp: write {}: {e}", job.path.display());
                } else if args.verbose {
                    eprintln!(
                        "written: {} ({} lines removed)",
                        job.path.display(),
                        outcome.deleted_lines.len()
                    );
                }
            }
        }
    } else {
        for (job, outcome) in &changed {
            let old = std::fs::read(&job.path).unwrap_or_default();
            let _ = stdout.write_all(
                rewriter::unified_diff(&job.path, &old, &outcome.deleted_lines).as_bytes(),
            );
        }
        let _ = stdout.flush();
    }

    // 镜像模式补齐未处理文件：二进制/超限/未知扩展名原样复制，保证镜像树完整（M2）
    if let Some(root) = args.output.as_deref() {
        for p in col
            .passthrough
            .iter()
            .chain(col.skipped.iter().map(|(p, _)| p))
        {
            if let Err(e) = mirror_copy(p, root) {
                had_io_error = true;
                eprintln!("alpp: mirror {}: {e}", p.display());
            }
        }
    }

    if args.verbose {
        eprintln!(
            "alpp: processed={} changed={} errors={} skipped={}",
            col.jobs.len(),
            changed.len(),
            errors.len(),
            col.skipped.len()
        );
        for (p, why) in &col.skipped {
            eprintln!("alpp: skipped {}: {why}", p.display());
        }
    } else if args.write && !changed.is_empty() {
        // 写回成功也给人一行反馈（走 stderr，不污染管道输出）
        eprintln!(
            "alpp: processed={} changed={} errors={} skipped={}",
            col.jobs.len(),
            changed.len(),
            errors.len(),
            col.skipped.len()
        );
    }

    // --gate 残留门禁（M4）：发布树中不得残留任何条件编译指令，违规 → 退出 3
    let mut residual: Vec<String> = Vec::new();
    if args.gate {
        for (job, outcome) in results
            .iter()
            .filter_map(|(j, r)| r.as_ref().ok().map(|o| (j, o)))
        {
            if let Some(&line) = outcome.directive_lines.first() {
                residual.push(format!(
                    "{}:{}: G1: residual conditional-compilation directive",
                    job.path.display(),
                    line
                ));
            }
        }
        residual.sort();
        for r in &residual {
            eprintln!("{r}");
        }
    }

    if had_io_error {
        Ok(std::process::ExitCode::from(1))
    } else if !errors.is_empty() {
        Ok(std::process::ExitCode::from(2))
    } else if !residual.is_empty() {
        Ok(std::process::ExitCode::from(3))
    } else {
        Ok(std::process::ExitCode::from(0))
    }
}

/// 处理单个文件：读入 → 选择生效 Profile → 流水线处理。IO 错误记为 code 0。
fn process_one(
    job: &FileJob,
    args: &Args,
    table: &MacroTable,
    loaded: Option<&Loaded>,
) -> Result<pipeline::Outcome, FileError> {
    let bytes = std::fs::read(&job.path).map_err(|e| FileError {
        path: job.path.display().to_string(),
        line: 0,
        code: 0,
        message: format!("io: {e}"),
    })?;

    // Profile 优先级：--prefix > --profile（已烘焙进 job）> 按扩展名（DESIGN §4.2）
    let owned_custom;
    let profile: &Profile = if let Some(px) = &args.prefix {
        owned_custom = Profile {
            name: "custom",
            prefix: px.as_str(),
            extensions: &[],
        };
        &owned_custom
    } else {
        &job.profile
    };

    let empty = Config::default();
    let cfg = loaded.map(|l| &l.cfg).unwrap_or(&empty);
    pipeline::process_bytes(&bytes, &job.path, profile, table, args.strict, cfg)
}

/// 写回：原地用 tmp+rename 原子替换；-o DIR 镜像目录树（DESIGN §4.7）。
fn write_back(path: &Path, bytes: &[u8], mirror: Option<&Path>) -> std::io::Result<()> {
    match mirror {
        None => {
            let tmp = path.with_extension("alpp-tmp");
            std::fs::write(&tmp, bytes)?;
            std::fs::rename(&tmp, path)
        }
        Some(root) => {
            // 镜像模式：以 CWD 相对路径映射到 root 下
            let rel = path.strip_prefix(".").unwrap_or(path);
            let dest = root.join(rel);
            if let Some(parent) = dest.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(dest, bytes)
        }
    }
}

/// 镜像补齐：未处理文件原样复制到镜像树（DESIGN §4.7 树完整性）。
fn mirror_copy(src: &Path, root: &Path) -> std::io::Result<()> {
    let rel = src.strip_prefix(".").unwrap_or(src);
    let dest = root.join(rel);
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::copy(src, dest).map(|_| ())
}

/// 应用一条 `NAME[=VALUE]` 宏定义（CLI -D 与配置 define 共用）。
fn apply_define(t: &mut MacroTable, spec: &str) -> Result<(), String> {
    let (name, value) = match spec.split_once('=') {
        Some((n, v)) => (n, v),
        None => (spec, ""),
    };
    if !alpp::eval::is_valid_name(name.as_bytes()) {
        return Err(format!("invalid macro name in define `{spec}`"));
    }
    let v = if value.is_empty() && !spec.contains('=') {
        MacroValue::Defined
    } else {
        value
            .parse::<i64>()
            .map_or_else(|_| MacroValue::Str(value.to_string()), MacroValue::Num)
    };
    t.define(name, v);
    Ok(())
}

/// 构建初始宏表。优先级从低到高（SPEC §10）：
/// 配置 define → 配置 undefine → 命令行 -D → 命令行 -U（后写者胜）。
fn build_macro_table(args: &Args, loaded: Option<&Loaded>) -> Result<MacroTable, TableError> {
    let empty = Config::default();
    let cfg = loaded.map(|l| &l.cfg).unwrap_or(&empty);

    let mut t = MacroTable::new();

    // 1. 配置层（最低优先级）
    for spec in &cfg.macros.define {
        apply_define(&mut t, spec).map_err(TableError::Usage)?;
    }
    for u in &cfg.macros.undefine {
        t.undef(u);
    }

    // 2. 命令行 -D
    for d in &args.defines {
        let name = d.split_once('=').map_or(d.as_str(), |(n, _)| n);
        if !alpp::eval::is_valid_name(name.as_bytes()) {
            return Err(TableError::Usage(format!("invalid macro name in -D{d}")));
        }
        apply_define(&mut t, d).map_err(TableError::Usage)?;
    }
    // 3. 命令行 -U（后写者胜）
    for u in &args.undefines {
        if !alpp::eval::is_valid_name(u.as_bytes()) {
            return Err(TableError::Usage(format!("invalid macro name in -U{u}")));
        }
        t.undef(u);
    }
    Ok(t)
}
