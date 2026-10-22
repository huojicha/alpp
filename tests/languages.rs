//! 语言专项端到端测试：每种语言一个全语法参考 fixture（golden file 方案）。
//!
//! `tests/fixtures/<lang>/all.<ext>` 覆盖 MACRO_SPEC 全部语法：
//! §3 指令全集（ifdef/ifndef/if/elif/else/endif/define/undef/error）、
//! §4 表达式全集（defined/数值比较/逻辑短路/括号优先级）、
//! §5 块语义（嵌套/外层抑制）、§6 输出规则（字符串豁免、缩进指令）。fixture 头部注释即使用说明，可直接作为写代码时
//! 的语法参考。
//!
//! 每语言三场景，全部断言为完整字符串逐字节对比：
//! - `default`（无宏）→ dry-run diff 全文 + write 全文；
//! - `-DFULL -DLEVEL=2` → write 全文；
//! - `-DNEED_DB` → 第 9 段 #error 活跃 → E5，退出码 2。
//!
//! 期望文件：`<lang>/diff-all-default.diff`、
//! `<lang>/write-all-default.<ext>`、`<lang>/write-all-macro.<ext>`。
//!
//! 覆盖 5 种语言 × 3 Profile（只收录无原生条件编译的语言）：
//! java、ts（c_family），py、sh（hash），lua（lua）。
//!
//! 注意：处理始终发生在临时目录，fixtures 本身永不改写；但若对 alpp
//! 仓库自身执行 `alpp -w .`，walker 会捡到 fixtures 里的源码文件——
//! 对本仓库跑 alpp 时请显式排除 tests/。

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const BIN: &str = env!("CARGO_BIN_EXE_alpp");
const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures");

struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "alpp-lang-{tag}-{}-{:?}",
            std::process::id(),
            std::time::SystemTime::now(),
        ));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        TempDir(dir)
    }

    fn path(&self) -> &Path {
        &self.0
    }

    fn write(&self, rel: &str, content: &str) {
        let p = self.0.join(rel);
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent).expect("create dir");
        }
        std::fs::write(p, content).expect("write file");
    }

    fn read(&self, rel: &str) -> String {
        std::fs::read_to_string(self.0.join(rel)).expect("read file")
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// 读取 golden file（期望文件缺失时给出明确 panic 信息）。
fn fixture(rel: &str) -> String {
    std::fs::read_to_string(Path::new(FIXTURES).join(rel))
        .unwrap_or_else(|e| panic!("missing fixture `{rel}`: {e}"))
}

fn run(cwd: &Path, args: &[&str]) -> Output {
    Command::new(BIN)
        .args(args)
        .current_dir(cwd)
        .output()
        .expect("spawn alpp")
}

fn stderr(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}

/// dry-run（默认模式）：退出码 0 且完整 diff 与 golden file 逐字节一致。
fn assert_diff(t: &TempDir, rel: &str, source: &str, extra: &[&str], expected: &str) {
    t.write(rel, source);
    let mut args: Vec<&str> = extra.to_vec();
    args.push(rel);
    let o = run(t.path(), &args);
    assert_eq!(
        o.status.code(),
        Some(0),
        "args={args:?} stderr={}",
        stderr(&o)
    );
    assert_eq!(
        String::from_utf8_lossy(&o.stdout),
        expected,
        "dry-run diff mismatch for {rel} (args={args:?})"
    );
}

/// write 模式：退出码 0 且写回文件与 golden file 逐字节一致。
fn assert_written(t: &TempDir, rel: &str, source: &str, extra: &[&str], expected: &str) {
    t.write(rel, source);
    let mut args: Vec<&str> = extra.to_vec();
    args.push("-w");
    args.push(rel);
    let o = run(t.path(), &args);
    assert_eq!(
        o.status.code(),
        Some(0),
        "args={args:?} stderr={}",
        stderr(&o)
    );
    assert_eq!(
        t.read(rel),
        expected,
        "written content mismatch for {rel} (args={args:?})"
    );
}

/// 三场景全断言：dry-run diff、无宏写回、组合宏写回、#error 退出码。
fn assert_all_reference(lang: &str, input: &str, ext: &str) {
    let t = TempDir::new(lang);
    let src = fixture(&format!("{lang}/{input}"));
    assert_diff(
        &t,
        input,
        &src,
        &[],
        &fixture(&format!("{lang}/diff-all-default.diff")),
    );
    assert_written(
        &t,
        input,
        &src,
        &[],
        &fixture(&format!("{lang}/write-all-default.{ext}")),
    );
    assert_written(
        &t,
        input,
        &src,
        &["-DFULL", "-DLEVEL=2"],
        &fixture(&format!("{lang}/write-all-macro.{ext}")),
    );
    // -DNEED_DB：第 9 段 #error 处于活跃分支 → E5，退出码 2，无 diff 输出
    t.write(input, &src);
    let o = run(t.path(), &["-DNEED_DB", input]);
    assert_eq!(
        o.status.code(),
        Some(2),
        "{lang}: E5 must exit 2, stderr={}",
        stderr(&o)
    );
    assert!(
        o.stdout.is_empty(),
        "{lang}: fatal error must not emit diff"
    );
    assert!(
        String::from_utf8_lossy(&o.stderr).contains("ALPP_DB_URL"),
        "{lang}: E5 message must name the requirement"
    );
}

#[test]
fn java_all_reference() {
    assert_all_reference("java", "All.java", "java");
}

#[test]
fn typescript_all_reference() {
    assert_all_reference("ts", "all.ts", "ts");
}

#[test]
fn python_all_reference() {
    assert_all_reference("python", "all.py", "py");
}

#[test]
fn shell_all_reference() {
    assert_all_reference("shell", "all.sh", "sh");
}

#[test]
fn lua_all_reference() {
    assert_all_reference("lua", "all.lua", "lua");
}
