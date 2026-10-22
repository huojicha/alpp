//! 单文件处理流水线：行扫描 → 条件栈状态机 → 输出拼装。
//!
//! 状态机语义见 MACRO_SPEC §5：
//! - 每层条件一个 Frame（taken / keeping / else_seen）；
//! - 外层未保留时内层只配平不求值（外层抑制）；
//! - 指令行与未保留分支的行整行删除，保留分支字节级保真（DESIGN 不变量 I3）。

use crate::config::Config;
use crate::error::FileError;
use crate::eval::{self, MacroTable, MacroValue};
use crate::profile::Profile;
use crate::scanner::{classify, iter_lines, Directive};
use std::path::Path;

/// 单文件处理结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outcome {
    pub output: Vec<u8>,
    /// 被删除的行号（1-based，升序）。指令行与被裁剪代码行都计入。
    pub deleted_lines: Vec<usize>,
}

impl Outcome {
    pub fn changed(&self) -> bool {
        !self.deleted_lines.is_empty()
    }
}

struct Frame {
    taken: bool,
    keeping: bool,
    else_seen: bool,
    open_line: usize,
}

/// 处理一个完整文件字节缓冲（M1 整文件驻留；分块读入是 M3 优化，DESIGN §4.4.3）。
///
/// `config`：`.alpp` 指令语法管控（[directives] deny/allow），违规报 E8——无论指令所在分支
/// 是否活跃（管控语义，出现即违规；与 E5 的“活跃才触发”不同，SPEC §10）。
pub fn process_bytes(
    bytes: &[u8],
    path: &Path,
    profile: &Profile,
    macros: &MacroTable,
    strict: bool,
    config: &Config,
) -> Result<Outcome, FileError> {
    let err = |line: usize, code: u8, message: String| FileError {
        path: path.display().to_string(),
        line,
        code,
        message,
    };

    let mut out = Vec::with_capacity(bytes.len());
    let mut deleted_lines = Vec::new();
    let mut stack: Vec<Frame> = Vec::new();
    // 文件内 #define/#undef 只影响本文件（SPEC §1.3 / §4.2）
    let mut macros = macros.clone();

    // 空栈 = true；任一外层未保留即抑制（SPEC §5.3 外层抑制）
    fn all_keeping(stack: &[Frame]) -> bool {
        stack.iter().all(|f| f.keeping)
    }

    // Directive 变体 → 指令关键字名（配置管控与错误信息用）
    fn directive_kind(d: &Directive) -> &'static str {
        match d {
            Directive::If(_) => "if",
            Directive::Ifdef(_) => "ifdef",
            Directive::Ifndef(_) => "ifndef",
            Directive::Elif(_) => "elif",
            Directive::Else => "else",
            Directive::Endif => "endif",
            Directive::Define { .. } => "define",
            Directive::Undef(_) => "undef",
            Directive::Error(_) => "error",
        }
    }

    for line in iter_lines(bytes) {
        let dir =
            classify(line.content, profile, strict).map_err(|e| err(line.no, e.code, e.message))?;

        let dir = match dir {
            None => {
                if all_keeping(&stack) {
                    out.extend_from_slice(&bytes[line.start..line.total_end]);
                } else {
                    deleted_lines.push(line.no);
                }
                continue;
            }
            Some(d) => d,
        };

        // 指令行：先登记删除，再推进状态机（SPEC §6：指令行整行移除）
        deleted_lines.push(line.no);

        // 指令级语法管控（E8）：出现即违规，无论分支是否活跃（SPEC §10）
        config
            .check_directive(directive_kind(&dir))
            .map_err(|m| err(line.no, 8, m))?;

        match dir {
            Directive::Ifdef(name) => {
                let name = std::str::from_utf8(name).unwrap();
                let cond = macros.get(name).is_some();
                push_frame(&mut stack, cond, line.no);
            }
            Directive::Ifndef(name) => {
                let name = std::str::from_utf8(name).unwrap();
                let cond = macros.get(name).is_none();
                push_frame(&mut stack, cond, line.no);
            }
            Directive::If(expr) => {
                let cond = if all_keeping(&stack) {
                    eval::eval_expr(expr, &macros)
                        .map_err(|e| err(line.no, 7, format!("bad expression: {}", e.message)))?
                        != 0
                } else {
                    false // 外层抑制：不求值，本层永不保留
                };
                push_frame(&mut stack, cond, line.no);
            }
            Directive::Elif(expr) => {
                // 先计算外层保留态（不可变借用），再可变借用栈顶
                let outer = all_keeping(&stack[..stack.len().saturating_sub(1)]);
                let frame = stack
                    .last_mut()
                    .ok_or_else(|| err(line.no, 3, "#elif without matching #if".into()))?;
                if frame.taken {
                    frame.keeping = false; // 已有保留分支，elif 分支不再保留
                } else if outer {
                    let v = eval::eval_expr(expr, &macros)
                        .map_err(|e| err(line.no, 7, format!("bad expression: {}", e.message)))?
                        != 0;
                    frame.taken = v;
                    frame.keeping = v;
                }
                // 外层抑制时：taken=false 保持，keeping=false 保持，内层仍只配平
            }
            Directive::Else => {
                let outer = all_keeping(&stack[..stack.len().saturating_sub(1)]);
                let frame = stack
                    .last_mut()
                    .ok_or_else(|| err(line.no, 3, "#else without matching #if".into()))?;
                if frame.else_seen {
                    return Err(err(line.no, 2, "duplicate #else in the same block".into()));
                }
                frame.else_seen = true;
                if frame.taken {
                    frame.keeping = false;
                } else {
                    frame.taken = true;
                    frame.keeping = outer;
                }
            }
            Directive::Endif => {
                stack
                    .pop()
                    .ok_or_else(|| err(line.no, 3, "#endif without matching #if".into()))?;
            }
            Directive::Define { name, value } => {
                let name = std::str::from_utf8(name).unwrap();
                if all_keeping(&stack) {
                    let v = match value {
                        None => MacroValue::Defined,
                        Some(v) => {
                            let s = String::from_utf8_lossy(v);
                            match s.parse::<i64>() {
                                Ok(n) => MacroValue::Num(n),
                                Err(_) => MacroValue::Str(s.into_owned()),
                            }
                        }
                    };
                    macros.define(name, v);
                }
            }
            Directive::Undef(name) => {
                let name = std::str::from_utf8(name).unwrap();
                if all_keeping(&stack) {
                    macros.undef(name);
                }
            }
            Directive::Error(msg) => {
                if all_keeping(&stack) {
                    // 无附言时避免尾随空格："#error" 而非 "#error "
                    let detail = String::from_utf8_lossy(msg).trim().to_owned();
                    return Err(err(
                        line.no,
                        5,
                        if detail.is_empty() {
                            "#error".into()
                        } else {
                            format!("#error {detail}")
                        },
                    ));
                }
            }
        }
    }

    if let Some(frame) = stack.last() {
        return Err(err(
            frame.open_line,
            1,
            "unterminated conditional block (opened here)".to_string(),
        ));
    }

    Ok(Outcome {
        output: out,
        deleted_lines,
    })
}

fn push_frame(stack: &mut Vec<Frame>, cond: bool, open_line: usize) {
    if stack.iter().all(|f| f.keeping) {
        // 活跃层：按求值结果保留
        stack.push(Frame {
            taken: cond,
            keeping: cond,
            else_seen: false,
            open_line,
        });
    } else {
        // 外层抑制：taken=true 使本层 elif/else 永不翻转（SPEC §5.3）
        stack.push(Frame {
            taken: true,
            keeping: false,
            else_seen: false,
            open_line,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profile::{C_FAMILY, HASH};

    fn run(src: &str, defines: &[&str]) -> Result<String, FileError> {
        let mut t = MacroTable::new();
        for d in defines {
            // 支持 "NAME" 与 "NAME=VALUE" 两种形式，与 CLI -D 一致
            match d.split_once('=') {
                Some((n, v)) => {
                    t.define(n, MacroValue::Num(v.parse().unwrap()));
                }
                None => t.define(d, MacroValue::Defined),
            }
        }
        let out = process_bytes(
            src.as_bytes(),
            Path::new("t.js"),
            &C_FAMILY,
            &t,
            false,
            &Config::default(),
        )?;
        Ok(String::from_utf8(out.output).unwrap())
    }

    fn run_err(src: &str, defines: &[&str]) -> FileError {
        run(src, defines).unwrap_err()
    }

    #[test]
    fn basic_ifndef_strip() {
        let src = "a\n//#ifndef RELEASE\nlog(1)\n//#endif\nb\n";
        assert_eq!(run(src, &["RELEASE"]).unwrap(), "a\nb\n");
        // 未定义 RELEASE：块保留，指令行仍删除
        assert_eq!(run(src, &[]).unwrap(), "a\nlog(1)\nb\n");
    }

    #[test]
    fn ifdef_inverse() {
        let src = "//#ifdef RELEASE\nrel()\n//#endif\nx\n";
        assert_eq!(run(src, &["RELEASE"]).unwrap(), "rel()\nx\n");
        assert_eq!(run(src, &[]).unwrap(), "x\n");
    }

    #[test]
    fn else_branch() {
        let src = "//#ifdef R\nrel()\n//#else\ndev()\n//#endif\nz\n";
        assert_eq!(run(src, &[]).unwrap(), "dev()\nz\n");
        assert_eq!(run(src, &["R"]).unwrap(), "rel()\nz\n");
    }

    #[test]
    fn elif_chain() {
        let src = "//#if L == 1\none\n//#elif L == 2\ntwo\n//#elif L == 3\nthree\n//#else\nother\n//#endif\nend\n";
        assert_eq!(run(src, &["L=1"]).unwrap(), "one\nend\n");
        assert_eq!(run(src, &["L=2"]).unwrap(), "two\nend\n");
        assert_eq!(run(src, &["L=3"]).unwrap(), "three\nend\n");
        assert_eq!(run(src, &["L=9"]).unwrap(), "other\nend\n");
        assert_eq!(run(src, &[]).unwrap(), "other\nend\n"); // 未定义 L → 0
    }

    #[test]
    fn elif_first_taken_skips_rest() {
        let src = "//#if 1\na\n//#elif 1\nb\n//#else\nc\n//#endif\nd\n";
        assert_eq!(run(src, &[]).unwrap(), "a\nd\n");
    }

    #[test]
    fn nested_with_outer_suppression() {
        // 外层未保留时，内层指令只配平不求值、不触发 #error
        let src = "//#ifdef R\n//#if defined(NOTHING)\nx()\n//#endif\n//#error should not fire\n//#endif\nok\n";
        assert_eq!(run(src, &[]).unwrap(), "ok\n");
        // 外层保留时，#error 必须触发（R 定义，内层 #if false 不影响）
        let e = run_err(src, &["R"]);
        assert_eq!(e.code, 5);
        assert_eq!(e.line, 5);
    }

    #[test]
    fn nested_outer_skip_keeps_inner_code_hidden() {
        let src = "//#ifdef A\nouter()\n//#ifdef B\ninner()\n//#endif\n//#endif\nz\n";
        assert_eq!(run(src, &["A", "B"]).unwrap(), "outer()\ninner()\nz\n");
        assert_eq!(run(src, &["A"]).unwrap(), "outer()\nz\n");
        assert_eq!(run(src, &[]).unwrap(), "z\n");
    }

    #[test]
    fn file_local_define_and_undef() {
        let src = "//#define FLAG\n//#ifdef FLAG\nyes\n//#endif\n//#undef FLAG\n//#ifdef FLAG\nno\n//#endif\n";
        assert_eq!(run(src, &[]).unwrap(), "yes\n");
        // CLI 定义被文件内 #undef 覆盖（文件内顺序生效）
        assert_eq!(run(src, &["FLAG"]).unwrap(), "yes\n");
    }

    #[test]
    fn define_with_value_used_in_expr() {
        let src = "//#define LEVEL 2\n//#if LEVEL >= 2\nhi\n//#endif\n";
        assert_eq!(run(src, &[]).unwrap(), "hi\n");
    }

    #[test]
    fn error_fires_only_in_active_branch() {
        // 外层未保留 → #error 不触发（外层抑制，SPEC §5.3）
        assert_eq!(
            run(
                "//#ifdef X\n//#if defined(NOTHING)\nx()\n//#endif\n//#error boom\n//#endif\nok",
                &[]
            )
            .unwrap(),
            "ok"
        );
        // 活跃分支中触发 E5
        let e = run_err("//#error boom\n", &[]);
        assert_eq!(e.code, 5);
        assert!(e.message.contains("boom"));
        assert_eq!(e.line, 1);
    }

    #[test]
    fn unterminated_block_is_e1() {
        let e = run_err("//#ifdef A\nx\n", &[]);
        assert_eq!(e.code, 1);
        assert_eq!(e.line, 1); // 指向块开启处
    }

    #[test]
    fn orphan_and_duplicate_are_e3_e2() {
        assert_eq!(run_err("//#endif\n", &[]).code, 3);
        assert_eq!(run_err("//#else\n", &[]).code, 3);
        assert_eq!(
            run_err("//#if 1\n//#else\n//#else\n//#endif\n", &[]).code,
            2
        );
    }

    #[test]
    fn bad_expression_is_e7() {
        assert_eq!(run_err("//#if 1 +\n//#endif\n", &[]).code, 7);
    }

    #[test]
    fn strict_unknown_directive_is_e4() {
        let t = MacroTable::new();
        let e = process_bytes(
            b"//#include <x.h>\n",
            Path::new("t.js"),
            &C_FAMILY,
            &t,
            true,
            &Config::default(),
        )
        .unwrap_err();
        assert_eq!(e.code, 4);
    }

    #[test]
    fn indentation_preserved_and_bytes_fidelity() {
        let src = "func f() {\n\t//#ifndef R\n\tlog(x)\n\t//#endif\n}\n";
        assert_eq!(run(src, &["R"]).unwrap(), "func f() {\n}\n");
        // CRLF 保留分支
        let crlf = "a\r\n//#ifndef R\r\nlog\r\n//#endif\r\nb\r\n";
        let mut t = MacroTable::new();
        t.define("R", MacroValue::Defined);
        let out = process_bytes(
            crlf.as_bytes(),
            Path::new("t"),
            &C_FAMILY,
            &t,
            false,
            &Config::default(),
        )
        .unwrap();
        assert_eq!(String::from_utf8(out.output).unwrap(), "a\r\nb\r\n");
        assert_eq!(out.deleted_lines, vec![2, 3, 4]);
    }

    #[test]
    fn unchanged_file_is_identity() {
        let src = "a\nb\n";
        let t = MacroTable::new();
        let o = process_bytes(
            src.as_bytes(),
            Path::new("t"),
            &C_FAMILY,
            &t,
            false,
            &Config::default(),
        )
        .unwrap();
        assert!(!o.changed());
        assert_eq!(o.output.as_slice(), src.as_bytes());
        assert!(o.deleted_lines.is_empty());
    }

    #[test]
    fn hash_profile_end_to_end() {
        let src = "def f():\n    ##ifndef R\n    log(1)\n    ##endif\n    return 1\n";
        let mut t = MacroTable::new();
        t.define("R", MacroValue::Defined);
        let o = process_bytes(
            src.as_bytes(),
            Path::new("t.py"),
            &HASH,
            &t,
            false,
            &Config::default(),
        )
        .unwrap();
        assert_eq!(
            String::from_utf8(o.output).unwrap(),
            "def f():\n    return 1\n"
        );
        assert_eq!(o.deleted_lines, vec![2, 3, 4]);
    }
}
