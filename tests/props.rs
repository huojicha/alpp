//! 属性测试（M4，DESIGN §6）：seed 扫描式随机指令树，替代 proptest 框架
//! （seed 失败可精确复现，无需收缩）。构造器保证指令配平，因此断言三条不变量：
//! I-P1 平衡树永不产生 E1/E2/E3；I-P2 strip 完备（输出无任何指令行）；
//! I-P3 幂等（对输出再处理一遍零变更）。

use alpp::config::Config;
use alpp::eval::{MacroTable, MacroValue};
use alpp::pipeline;
use alpp::profile::{self, Profile};
use std::path::Path;

/// xorshift64*（benches 同款），避免引入 rand 依赖。
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

const NAMES: [&str; 4] = ["M0", "M1", "M2", "M3"];
const EXPRS: [&str; 5] = [
    "defined(M0)",
    "!defined(M1)",
    "M2 == 0",
    "M3 > 2 && 1",
    "M0 || defined(M2)",
];
const CODE_LINES: [&str; 4] = ["x = 1;", "f();", "y += 2;", ""];

/// 生成一个构造性配平的指令块（open + 内容 + [嵌套] + [else] + endif）。
fn gen_block(rng: &mut Rng, prefix: &str, depth: u64, out: &mut Vec<String>) {
    let name = NAMES[rng.below(4) as usize];
    out.push(match rng.below(3) {
        0 => format!("{prefix}ifdef {name}"),
        1 => format!("{prefix}ifndef {name}"),
        _ => format!(
            "{prefix}if {}",
            EXPRS[rng.below(EXPRS.len() as u64) as usize]
        ),
    });
    for _ in 0..rng.below(3) {
        out.push(CODE_LINES[rng.below(4) as usize].to_string());
    }
    if depth > 0 && rng.below(2) == 0 {
        gen_block(rng, prefix, depth - 1, out);
    }
    if rng.below(10) < 3 {
        out.push(format!("{prefix}else"));
        for _ in 0..rng.below(2) {
            out.push(CODE_LINES[rng.below(4) as usize].to_string());
        }
    }
    out.push(format!("{prefix}endif"));
}

/// 生成一个完整源文件：顶层代码行与指令块交错（整体平衡）。
fn gen_file(rng: &mut Rng, prefix: &str) -> String {
    let mut lines: Vec<String> = Vec::new();
    for _ in 0..rng.below(3) {
        lines.push(CODE_LINES[rng.below(4) as usize].to_string());
    }
    for _ in 0..1 + rng.below(3) {
        gen_block(rng, prefix, 3, &mut lines);
        lines.push("mid();".to_string());
    }
    for _ in 0..rng.below(2) {
        lines.push(CODE_LINES[rng.below(4) as usize].to_string());
    }
    let mut text = lines.join("\n");
    if !text.is_empty() {
        text.push('\n');
    }
    text
}

fn profile_for(i: u64) -> Profile<'static> {
    [profile::C_FAMILY, profile::HASH, profile::LUA][(i % 3) as usize]
}

/// 输出中不得残留任何指令行（I-P2）。
fn assert_no_directives(out: &[u8], prefix: &str) {
    let text = String::from_utf8_lossy(out);
    for (idx, line) in text.lines().enumerate() {
        let t = line.trim_start();
        let rest = t.strip_prefix(prefix).unwrap_or("");
        let kw = rest.split_whitespace().next().unwrap_or("");
        assert!(
            !["if", "ifdef", "ifndef", "elif", "else", "endif", "define", "undef", "error"]
                .contains(&kw),
            "残留指令行 L{}: {line:?}",
            idx + 1
        );
    }
}

#[test]
fn invariants_hold_across_random_balanced_trees() {
    const SEEDS: u64 = 600;
    for seed in 0..SEEDS {
        let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
        let i = seed % 3;
        let prefix = ["//#", "##", "--#"][i as usize];
        let text = gen_file(&mut rng, prefix);

        let table = MacroTable::new();
        let first = pipeline::process_bytes(
            text.as_bytes(),
            Path::new(&format!("p{seed}.{}", ["js", "py", "lua"][i as usize])),
            &profile_for(i),
            &table,
            false,
            &Config::default(),
        );

        // I-P1：构造性配平的树不产生结构/求值错误
        let first = match first {
            Ok(o) => o,
            Err(e) => panic!(
                "seed {seed} ({prefix}): 平衡树报错 {}:{} E{}: {}",
                e.path, e.line, e.code, e.message
            ),
        };

        // I-P2：strip 完备
        assert_no_directives(&first.output, prefix);

        // I-P3：幂等——对输出再处理一遍零变更
        let second = pipeline::process_bytes(
            &first.output,
            Path::new("second"),
            &profile_for(i),
            &table,
            false,
            &Config::default(),
        )
        .unwrap_or_else(|e| panic!("seed {seed}: 二次处理报错 E{}: {}", e.code, e.message));
        assert!(
            second.deleted_lines.is_empty(),
            "seed {seed}: 输出不是幂等的，仍删 {} 行",
            second.deleted_lines.len()
        );
        assert_eq!(second.output, first.output, "seed {seed}: 二次输出漂移");
    }
}

/// 宏表非空（-D 全开/全关两种极端）时同一组不变量仍成立。
#[test]
fn invariants_hold_with_populated_macro_table() {
    for seed in 0..300u64 {
        let mut rng = Rng(seed.wrapping_mul(0xBF58_476D_1CE4_E5B9) | 1);
        let i = seed % 3;
        let prefix = ["//#", "##", "--#"][i as usize];
        let text = gen_file(&mut rng, prefix);

        let mut table = MacroTable::new();
        for name in NAMES {
            table.define(
                name,
                if seed % 2 == 0 {
                    MacroValue::Num(1)
                } else {
                    MacroValue::Num(0)
                },
            );
        }

        let out = pipeline::process_bytes(
            text.as_bytes(),
            Path::new("q.js"),
            &profile_for(i),
            &table,
            false,
            &Config::default(),
        )
        .unwrap_or_else(|e| panic!("seed {seed} ({prefix}): E{}: {}", e.code, e.message));

        assert_no_directives(&out.output, prefix);
    }
}
