//! 端到端集成测试：真实二进制 × 真实文件树（DESIGN §7 M1 验收）。
//!
//! 覆盖三模式（dry-run / --write / --check）、-D/-U、--profile、--strict、
//! walker 行为（gitignore/二进制/未知扩展名）、退出码 0/1/2、CRLF 保真。

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const BIN: &str = env!("CARGO_BIN_EXE_alpp");

/// 生命周期受管的临时目录，测试结束自动清理。
struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> Self {
        let p = std::env::temp_dir().join(format!("alpp-it-{}-{}", tag, std::process::id()));
        let _ = fs::remove_dir_all(&p);
        fs::create_dir_all(&p).unwrap();
        TempDir(p)
    }

    fn path(&self) -> &Path {
        &self.0
    }

    fn write(&self, rel: &str, content: &str) -> PathBuf {
        let p = self.0.join(rel);
        if let Some(parent) = p.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(&p, content).unwrap();
        p
    }

    fn read(&self, rel: &str) -> String {
        fs::read_to_string(self.0.join(rel)).unwrap()
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// 在 cwd 下运行 alpp。
fn run(cwd: &Path, args: &[&str]) -> Output {
    Command::new(BIN)
        .args(args)
        .current_dir(cwd)
        .output()
        .expect("failed to spawn alpp")
}

fn stdout(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}

fn stderr(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}

// ---------------------------------------------------------------------------
// dry-run：unified diff 与文件不被修改
// ---------------------------------------------------------------------------

#[test]
fn dry_run_exact_diff_and_files_untouched() {
    let t = TempDir::new("dryrun");
    t.write("mini.js", "a\n//#ifdef X\nb\n//#endif\nc\n");

    // X 未定义：删 2..4。ctx [1..5] → @@ -1,5 +1,2 @@
    let o = run(t.path(), &["mini.js"]);
    assert_eq!(o.status.code(), Some(0));
    let expect =
        "--- a/mini.js\n+++ b/mini.js\n@@ -1,5 +1,2 @@\n a\n-//#ifdef X\n-b\n-//#endif\n c\n";
    assert_eq!(stdout(&o), expect);

    // dry-run 不落盘
    assert_eq!(t.read("mini.js"), "a\n//#ifdef X\nb\n//#endif\nc\n");

    // X 定义后：块保留但指令行仍从输出剥离（SPEC §6：指令行无条件删除）
    let o = run(t.path(), &["-DX", "mini.js"]);
    assert_eq!(o.status.code(), Some(0));
    let expect =
        "--- a/mini.js\n+++ b/mini.js\n@@ -1,5 +1,3 @@\n a\n-//#ifdef X\n b\n-//#endif\n c\n";
    assert_eq!(stdout(&o), expect);
    assert_eq!(t.read("mini.js"), "a\n//#ifdef X\nb\n//#endif\nc\n");
}

#[test]
fn dry_run_mixed_profiles_js_py_lua() {
    let t = TempDir::new("mixed");
    t.write("a.js", "x\n//#ifndef R\nLOG(1)\n//#endif\ny\n");
    t.write("b.py", "##ifdef R\nimport logging\n##endif\nprint('hi')\n");
    t.write("c.lua", "--#ifdef R\nlog('dbg')\n--#endif\nprint('ok')\n");

    // -DR：ifndef 块被剔除；ifdef 块保留但指令行剥离（SPEC §6）
    let o = run(t.path(), &["-DR", "."]);
    assert_eq!(o.status.code(), Some(0), "stderr: {}", stderr(&o));
    let out = stdout(&o);
    assert!(
        out.contains("-//#ifndef R") && out.contains("-LOG(1)"),
        "a.js 块应被剔除:\n{out}"
    );
    assert!(
        out.contains("-##ifdef R") && out.contains(" import logging"),
        "b.py 保留内容:\n{out}"
    );
    assert!(
        out.contains("--#ifdef R") && out.contains(" log('dbg')"),
        "c.lua 保留内容（diff 删除前缀叠加 -- 呈三连字符）:\n{out}"
    );

    // 不带 -DR：ifndef 块保留（R 未定义）、两个 ifdef 块剔除
    let o = run(t.path(), &["."]);
    assert_eq!(o.status.code(), Some(0));
    let out = stdout(&o);
    assert!(out.contains(" LOG(1)"), "ifndef 未定义 → 保留:\n{out}");
    assert!(!out.contains("-LOG(1)"));
    assert!(out.contains("-import logging"));
    assert!(out.contains("-log('dbg')"), "lua 前缀行应被识别:\n{out}");
}

// ---------------------------------------------------------------------------
// --write：原地改写、CRLF 保真、镜像树
// ---------------------------------------------------------------------------

#[test]
fn write_in_place_exact_output() {
    let t = TempDir::new("write");
    t.write(
        "s.js",
        "package main\n\n//#if LEVEL >= 2\nheavyLog(\"x\", 1)\n//#endif\nfunc f() {}\n",
    );

    // LEVEL 未定义 → 0，0 >= 2 假 → 块整体删除
    let o = run(t.path(), &["-w", "."]);
    assert_eq!(o.status.code(), Some(0), "stderr: {}", stderr(&o));
    assert_eq!(
        t.read("s.js"),
        "package main\n\nfunc f() {}\n",
        "指令行与被抑制内容都应删除"
    );

    // 再跑一次：已是干净树，无变更
    let o = run(t.path(), &["-w", "."]);
    assert_eq!(o.status.code(), Some(0));
}

#[test]
fn write_preserves_crlf_and_tabs() {
    let t = TempDir::new("crlf");
    t.write("w.js", "l1\r\n//#ifdef X\r\nl2\r\n//#endif\r\nl3\r\n");
    let o = run(t.path(), &["-w", "w.js"]);
    assert_eq!(o.status.code(), Some(0));
    assert_eq!(
        t.read("w.js"),
        "l1\r\nl3\r\n",
        "删除块后剩余行的 CRLF 必须原样保留"
    );
}

#[test]
fn write_mirror_tree_includes_unchanged_files() {
    let t = TempDir::new("mirror");
    t.write("a.js", "x\n//#ifdef R\nLOG(1)\n//#endif\ny\n");
    t.write("sub/b.js", "clean()\n");
    t.write("notes.txt", "not source\n");

    let out = t.path().join("out");
    let o = run(t.path(), &["-w", "-o", "out", "."]);
    assert_eq!(o.status.code(), Some(0), "stderr: {}", stderr(&o));

    // 变更文件被改写进镜像（块剔除，不留空行）
    assert!(t.path().join("out/a.js").exists());
    assert_eq!(fs::read_to_string(out.join("a.js")).unwrap(), "x\ny\n");
    // 未变更的已处理文件也进镜像（树完整性）
    assert_eq!(
        fs::read_to_string(out.join("sub/b.js")).unwrap(),
        "clean()\n"
    );
    // 未识别扩展名不属于 jobs，不进镜像（M1 已知边界）
    assert!(!out.join("notes.txt").exists());
    // 原树完全未动
    assert_eq!(t.read("a.js"), "x\n//#ifdef R\nLOG(1)\n//#endif\ny\n");
    assert_eq!(t.read("sub/b.js"), "clean()\n");
}

// ---------------------------------------------------------------------------
// --check：CI 门禁语义（只校验，退出码驱动）
// ---------------------------------------------------------------------------

#[test]
fn check_mode_exit_codes() {
    let t = TempDir::new("check");

    // 干净树（含合法指令）→ 0
    t.write("ok.js", "//#ifdef A\nx\n//#endif\n");
    let o = run(t.path(), &["--check", "."]);
    assert_eq!(o.status.code(), Some(0), "stderr: {}", stderr(&o));

    // 未闭合块 → 2 + E1
    t.write("bad.js", "//#ifdef A\nx\n");
    let o = run(t.path(), &["--check", "."]);
    assert_eq!(o.status.code(), Some(2));
    let err = stderr(&o);
    assert!(err.contains("bad.js"), "错误应带文件名: {err}");
    assert!(err.contains("E1"), "应有错误码: {err}");

    // check 模式不落盘、不输出 diff
    assert_eq!(t.read("bad.js"), "//#ifdef A\nx\n");
    assert_eq!(stdout(&o), "");
}

// ---------------------------------------------------------------------------
// 错误指令与宏语义
// ---------------------------------------------------------------------------

#[test]
fn error_directive_active_vs_suppressed() {
    let t = TempDir::new("errdir");

    // 活跃分支中的 #error → E5，退出码 2
    t.write(
        "a.js",
        "//#if !defined(PROD)\n//#error PROD required for release\n//#endif\nx\n",
    );
    let o = run(t.path(), &["."]);
    assert_eq!(o.status.code(), Some(2));
    assert!(stderr(&o).contains("E5"));

    // 定义 PROD 后该分支被抑制，#error 不触发：剔除后只剩 x
    let o = run(t.path(), &["-DPROD", "-w", "."]);
    assert_eq!(o.status.code(), Some(0), "stderr: {}", stderr(&o));
    assert_eq!(t.read("a.js"), "x\n");
}

#[test]
fn nested_blocks_and_else_branch() {
    let t = TempDir::new("nested");
    t.write(
        "n.js",
        "1\n//#ifdef A\n2a\n//#ifdef B\n3ab\n//#endif\n4a\n//#else\n2else\n//#endif\nend\n",
    );

    // A、B 都未定义：保留外层 else 分支
    let o = run(t.path(), &["-w", "."]);
    assert_eq!(o.status.code(), Some(0));
    assert_eq!(t.read("n.js"), "1\n2else\nend\n");

    // 恢复文件，A 定义 B 未定义
    t.write(
        "n.js",
        "1\n//#ifdef A\n2a\n//#ifdef B\n3ab\n//#endif\n4a\n//#else\n2else\n//#endif\nend\n",
    );
    let o = run(t.path(), &["-w", "-DA", "."]);
    assert_eq!(o.status.code(), Some(0));
    assert_eq!(t.read("n.js"), "1\n2a\n4a\nend\n");

    // 外层抑制时内层 #error 不触发（抑制分支只配平不求值，SPEC §5.3）
    t.write(
        "n.js",
        "1\n//#ifdef A\n//#if B\n//#error boom\n//#endif\n//#endif\nx\n",
    );
    let o = run(t.path(), &["-DA", "-w", "."]);
    assert_eq!(o.status.code(), Some(0), "外层抑制时内层 #error 不触发");
    assert_eq!(t.read("n.js"), "1\nx\n");
}

#[test]
fn define_in_file_scopes_to_file_only() {
    let t = TempDir::new("filescope");
    t.write("d1.js", "//#define LOCAL\n//#ifdef LOCAL\nkept\n//#endif\n");
    t.write(
        "d2.js",
        "//#ifdef LOCAL\nshould-not-appear\n//#endif\nafter\n",
    );

    let o = run(t.path(), &["-w", "."]);
    assert_eq!(o.status.code(), Some(0), "stderr: {}", stderr(&o));
    assert_eq!(t.read("d1.js"), "kept\n");
    assert_eq!(t.read("d2.js"), "after\n", "宏不得跨文件泄漏");
}

#[test]
fn cli_define_value_and_undef_precedence() {
    let t = TempDir::new("clival");

    // 数值宏参与比较
    t.write("v.js", "//#if LEVEL >= 2\nkept\n//#endif\ntail\n");
    let o = run(t.path(), &["-w", "-DLEVEL=3", "."]);
    assert_eq!(o.status.code(), Some(0));
    assert_eq!(t.read("v.js"), "kept\ntail\n", "LEVEL=3 满足 >= 2，保留");

    t.write("v.js", "//#if LEVEL >= 2\nkept\n//#endif\ntail\n");
    let o = run(t.path(), &["-w", "-DLEVEL=1", "."]);
    assert_eq!(o.status.code(), Some(0));
    assert_eq!(t.read("v.js"), "tail\n", "LEVEL=1 不满足，剔除");

    // -U 压制 -D（后写者胜）
    t.write("u.js", "//#ifdef X\nkept\n//#endif\ntail\n");
    let o = run(t.path(), &["-DX", "-UX", "-w", "."]);
    assert_eq!(o.status.code(), Some(0));
    assert_eq!(t.read("u.js"), "tail\n", "-UX 应压制 -DX");

    // 非法宏名 → 用法错误，退出码 1
    let o = run(t.path(), &["-D2BAD", "."]);
    assert_eq!(o.status.code(), Some(1));
    assert!(stderr(&o).contains("invalid macro name"));
}

// ---------------------------------------------------------------------------
// 严格模式与字符串豁免
// ---------------------------------------------------------------------------

#[test]
fn strict_unknown_directive_vs_loose() {
    let t = TempDir::new("strict");
    t.write("s.js", "a\n//#include <evil.h>\nb\n");

    // 宽松（默认）：当普通注释保留
    let o = run(t.path(), &["."]);
    assert_eq!(o.status.code(), Some(0));
    assert_eq!(stdout(&o), "");

    // 严格：E4
    let o = run(t.path(), &["--strict", "."]);
    assert_eq!(o.status.code(), Some(2));
    assert!(stderr(&o).contains("E4"));

    // 同样规则作用于 hash profile
    t.write("p.py", "##pragma weird\n");
    let o = run(t.path(), &["--strict", "."]);
    assert_eq!(o.status.code(), Some(2));
}

#[test]
fn string_literal_lines_are_not_directives() {
    let t = TempDir::new("strlit");
    // "//#if ..." 出现在字符串内（引号未闭合）→ 豁免
    t.write("g.js", "s := \"//#if WEIRD\"\ntail\n");
    let o = run(t.path(), &["--strict", "."]);
    assert_eq!(o.status.code(), Some(0), "stderr: {}", stderr(&o));
    assert_eq!(stdout(&o), "");
    assert_eq!(t.read("g.js"), "s := \"//#if WEIRD\"\ntail\n");
}

// ---------------------------------------------------------------------------
// walker：gitignore、二进制、未知扩展名、显式文件回退
// ---------------------------------------------------------------------------

#[test]
fn walker_respects_gitignore_and_skips_binary() {
    let t = TempDir::new("walker");
    // ignore crate 默认 require_git：造一个最小 .git 目录使 .gitignore 生效
    fs::create_dir_all(t.path().join(".git")).unwrap();
    t.write(".gitignore", "ignored.js\n");
    t.write("ignored.js", "//#ifdef X\ni\n//#endif\n");
    t.write("keep.js", "//#ifdef X\nk\n//#endif\n");
    // 含 NUL 的伪二进制
    let mut bin_content = String::from("//#ifdef X\n");
    bin_content.push('\0');
    bin_content.push_str("//#endif\n");
    t.write("blob.js", &bin_content);
    t.write("readme.txt", "no directives\n");

    // gitignore：ignored.js 被忽略；blob.js 二进制跳过；readme.txt 非源码
    let o = run(t.path(), &["-w", "."]);
    assert_eq!(o.status.code(), Some(0), "stderr: {}", stderr(&o));
    assert_eq!(
        t.read("ignored.js"),
        "//#ifdef X\ni\n//#endif\n",
        "gitignore 文件不应被改写"
    );
    assert_eq!(t.read("blob.js"), bin_content, "二进制不应被改写");
    assert_eq!(t.read("keep.js"), "", "未定义 X → 整块剔除，文件为空");

    // --no-ignore：ignored.js 参与处理
    let o = run(t.path(), &["-w", "--no-ignore", "."]);
    assert_eq!(o.status.code(), Some(0));
    assert_eq!(t.read("ignored.js"), "", "--no-ignore 应处理被忽略文件");
}

#[test]
fn explicit_file_unknown_extension_forced_c_family() {
    let t = TempDir::new("explicit");
    t.write(
        "Makefile",
        "//#ifdef DEBUG\necho dbg\n//#endif\nall:\n\techo all\n",
    );

    // 显式文件 + 未知扩展名 → 回退 c_family 并继续处理
    let o = run(t.path(), &["-w", "Makefile"]);
    assert_eq!(o.status.code(), Some(0), "stderr: {}", stderr(&o));
    assert_eq!(t.read("Makefile"), "all:\n\techo all\n");
}

#[test]
fn profile_override_for_hash_syntax() {
    let t = TempDir::new("profileovr");
    t.write("data.conf", "##ifdef X\n## mismatch\n##endif\nplain\n");

    // data.conf 未知扩展名 → 强制 c_family 时 ## 不算指令（宽松下无错无 diff）
    let o = run(t.path(), &["."]);
    assert_eq!(o.status.code(), Some(0));

    // -P hash 对未知扩展名生效：未定义 X → 整块剔除（含内部的未知指令行）
    let o = run(t.path(), &["-P", "hash", "."]);
    assert_eq!(o.status.code(), Some(0), "stderr: {}", stderr(&o));
    let out = stdout(&o);
    assert!(
        out.contains("-##ifdef X") && out.contains("-## mismatch"),
        "-P 应使 ## 被识别:\n{out}"
    );

    // 未定义 X：ifdef 块删除；-P 对目录遍历的未知扩展名文件同样生效
    t.write("data.conf", "##ifdef X\nhidden\n##endif\nplain\n");
    let o = run(t.path(), &["-w", "-P", "hash", "."]);
    assert_eq!(o.status.code(), Some(0));
    assert_eq!(t.read("data.conf"), "plain\n");

    // 未知 profile 名 → 用法错误，退出码 1
    let o = run(t.path(), &["-P", "nope", "."]);
    assert_eq!(o.status.code(), Some(1));
    assert!(stderr(&o).contains("unknown profile"));
}

// ---------------------------------------------------------------------------
// 退出码与 IO
// ---------------------------------------------------------------------------

#[test]
fn missing_path_is_exit_1() {
    let t = TempDir::new("missing");
    let o = run(t.path(), &["no/such/path.js"]);
    assert_eq!(o.status.code(), Some(1));
    assert!(stderr(&o).contains("no/such/path.js"));
}

#[test]
fn io_error_and_directive_error_both_reported() {
    let t = TempDir::new("botherr");
    t.write("bad.js", "//#ifdef A\nx\n");
    let o = run(t.path(), &["--check", ".", "no/such.js"]);
    assert_eq!(o.status.code(), Some(1), "IO 错误优先于指令错误");
    assert!(stderr(&o).contains("bad.js") || stderr(&o).contains("no/such.js"));
}

// ---------------------------------------------------------------------------
// `.alpp` 配置文件：指令语法管控（E8）与默认宏，优先级低于命令行（SPEC §10）
// ---------------------------------------------------------------------------

#[test]
fn config_define_lower_priority_than_cli() {
    let t = TempDir::new("cfgprio");
    t.write(".alpp", "[macros]\ndefine = [\"MODE=1\"]\n");
    t.write("a.js", "//#if MODE == 2\nkept\n//#endif\ntail\n");

    // 配置 MODE=1 → 不满足 → 删
    let o = run(t.path(), &["-w", "a.js"]);
    assert_eq!(o.status.code(), Some(0), "stderr={}", stderr(&o));
    assert_eq!(t.read("a.js"), "tail\n");

    // 命令行 -DMODE=2 覆盖配置 → 保留
    t.write("a.js", "//#if MODE == 2\nkept\n//#endif\ntail\n");
    let o = run(t.path(), &["-w", "-DMODE=2", "a.js"]);
    assert_eq!(o.status.code(), Some(0), "stderr={}", stderr(&o));
    assert_eq!(t.read("a.js"), "kept\ntail\n");
}

#[test]
fn config_no_config_skips_discovery() {
    let t = TempDir::new("cfgskip");
    t.write(".alpp", "[directives]\ndeny = [\"define\"]\n");
    t.write("a.js", "//#ifdef NEVER\n//#define BAD\n//#endif\ntail\n");

    // 有配置：define 被禁（抑制分支也报）→ E8 退出 2
    let o = run(t.path(), &["a.js"]);
    assert_eq!(o.status.code(), Some(2), "stderr={}", stderr(&o));

    // --no-config：跳过发现 → NEVER 未定义块正常裁剪
    let o = run(t.path(), &["--no-config", "-w", "a.js"]);
    assert_eq!(o.status.code(), Some(0), "stderr={}", stderr(&o));
    assert_eq!(
        t.read("a.js"),
        "tail\n",
        "--no-config 时指令不受管控 → 正常裁剪"
    );
}

#[test]
fn config_directive_deny_is_e8() {
    let t = TempDir::new("cfgdir");
    t.write(".alpp", "[directives]\ndeny = [\"define\", \"undef\"]\n");
    t.write("d.js", "//#define LOG 1\nx\n");

    let o = run(t.path(), &["d.js"]);
    assert_eq!(o.status.code(), Some(2), "stderr={}", stderr(&o));
    let err = stderr(&o);
    assert!(err.contains("d.js:1"), "应带文件行号: {err}");
    assert!(err.contains("E8"), "应有错误码: {err}");
    assert!(err.contains("directive `define` is denied"), "{err}");
    // 未写回：dry-run 模式出错文件不产出 diff
    assert_eq!(t.read("d.js"), "//#define LOG 1\nx\n");
}

#[test]
fn config_directive_hits_suppressed_branch_too() {
    let t = TempDir::new("cfgdirsup");
    t.write(".alpp", "[directives]\ndeny = [\"define\"]\n");
    t.write("s.js", "//#ifdef NEVER\n//#define BAD\n//#endif\ntail\n");

    // 外层 NEVER 未定义 → 块被裁剪，但 BAD 的 define 仍违规（管控语义）
    let o = run(t.path(), &["s.js"]);
    assert_eq!(o.status.code(), Some(2), "stderr={}", stderr(&o));
    assert!(stderr(&o).contains("E8"), "抑制分支内也报: {}", stderr(&o));
}

#[test]
fn config_directive_allow_whitelist() {
    let t = TempDir::new("cfgdirallow");
    t.write(".alpp", "[directives]\nallow = [\"ifdef\", \"endif\"]\n");
    t.write("w.js", "//#ifdef A\nx\n//#endif\n//#if 1\ny\n//#endif\n");

    let o = run(t.path(), &["w.js"]);
    assert_eq!(o.status.code(), Some(2), "stderr={}", stderr(&o));
    let err = stderr(&o);
    assert!(err.contains("w.js:4"), "应指向 #if 行: {err}");
    assert!(
        err.contains("directive `if` is not in the allow list"),
        "{err}"
    );

    // 全部用白名单内指令 → 正常裁剪（-DA 保留块，指令行剥离）
    t.write("ok.js", "//#ifdef A\nx\n//#endif\n");
    let o = run(t.path(), &["-w", "-DA", "ok.js"]);
    assert_eq!(o.status.code(), Some(0), "stderr={}", stderr(&o));
    assert_eq!(t.read("ok.js"), "x\n");
}

#[test]
fn config_directive_bad_name_is_config_error() {
    let t = TempDir::new("cfgdirbad");
    t.write(".alpp", "[directives]\ndeny = [\"ifeq\"]\n");
    t.write("a.js", "x\n");

    // 非法指令名 → 配置解析失败，启动错误退出 1
    let o = run(t.path(), &["a.js"]);
    assert_eq!(o.status.code(), Some(1), "stderr={}", stderr(&o));
    assert!(
        stderr(&o).contains("invalid directive name"),
        "{}",
        stderr(&o)
    );
}

#[test]
fn config_discovery_walks_up_from_nested_dir() {
    let t = TempDir::new("cfgup");
    t.write(".alpp", "[directives]\ndeny = [\"error\"]\n");
    t.write("deep/nested/a.js", "//#error\ntail\n");

    let o = run(t.path(), &["deep/nested"]);
    assert_eq!(
        o.status.code(),
        Some(2),
        "从嵌套目录向上找到配置, stderr={}",
        stderr(&o)
    );
}

#[test]
fn config_bad_toml_is_config_error() {
    let t = TempDir::new("cfgbad");
    t.write(".alpp", "not toml [");
    t.write("a.js", "ok\n");

    let o = run(t.path(), &["a.js"]);
    assert_eq!(
        o.status.code(),
        Some(1),
        "配置解析失败 → 启动错误, stderr={}",
        stderr(&o)
    );
    assert!(stderr(&o).contains(".alpp"), "{}", stderr(&o));
}

#[test]
fn config_without_config_file_behaves_as_before() {
    let t = TempDir::new("cfgnone");
    t.write("a.js", "//#ifdef X\nx\n//#endif\ntail\n");

    let o = run(t.path(), &["-w", "-DX", "a.js"]);
    assert_eq!(o.status.code(), Some(0), "stderr={}", stderr(&o));
    assert_eq!(t.read("a.js"), "x\ntail\n");
}
