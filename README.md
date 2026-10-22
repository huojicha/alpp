<div align="center">

# alpp

**给任何语言加上 `#ifdef` —— 注释内嵌条件编译，发布构建时一键裁剪**

[![CI](https://github.com/myzhan/alpp/actions/workflows/ci.yml/badge.svg)](https://github.com/myzhan/alpp/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/rust-stable-orange.svg)](https://www.rust-lang.org)

日志、埋点、实验特性想从发布版里干净剥离，但 Java / Python / Shell / Lua 没有条件编译？
**alpp** 把 `#ifdef` 藏进语言注释里——平时对编译器、lint、IDE 完全不可见；发布构建时一条命令裁出干净的生产代码。

</div>

---

## 它解决什么问题

调试日志想在开发时留着、发布时删掉？传统的做法（日志级别开关）运行时仍有开销，且代码永远留在产物里。alpp 让你用 C 程序员的方式管理它们：

```java
import java.util.logging.Logger;

public class UserService {
    //#ifdef TRACE_SQL
    private static final Logger SQL_LOG = Logger.getLogger("sql");
    //#endif

    public User find(long id) {
        User u = db.query(id);

        //#if TRACE_SQL && SLOW_QUERY_MS > 0
        SQL_LOG.fine("find(" + id + ") took " + slowMs() + "ms");
        //#endif
        //#ifdef TRACE_SQL
        SQL_LOG.fine("find(" + id + ") done");
        //#endif

        return u;
    }
}
```

发布构建跑一条命令：

```bash
alpp -w -DTRACE_SQL ./src
```

输出（`TRACE_SQL` 开 → 日志保留；`SLOW_QUERY_MS` 未定义按 0 → `0 > 0` 为假 → 慢查询计时块被裁）：

```java
import java.util.logging.Logger;

public class UserService {
    private static final Logger SQL_LOG = Logger.getLogger("sql");

    public User find(long id) {
        User u = db.query(id);

        SQL_LOG.fine("find(" + id + ") done");

        return u;
    }
}
```

指令藏在注释里，所以**开发树不需要任何工具**就能通过编译、lint 与 IDE 检查——alpp 只在发布构建那一刻出场。

## 30 秒上手

```bash
# 安装（Rust 工具链）
cargo install --git https://github.com/myzhan/alpp

# dry-run：预览裁剪 diff（默认模式，不改任何文件）
alpp -DRELEASE ./src

# 落盘写回
alpp -w -DRELEASE ./src

# CI 校验：指令是否全部合法
alpp --check ./src

# 发布门禁：发布树中不允许残留任何指令
alpp --gate ./dist
```

退出码：`0` 成功 · `1` IO/用法错误 · `2` 指令校验失败 · `3` `--gate` 发现残留。

## 语法速览

九条指令，与 C 预处理器同形：

| 指令 | 示例 |
|---|---|
| `#ifdef` / `#ifndef` | `//#ifdef TRACE_SQL` |
| `#if` / `#elif` | `//#if TRACE && LEVEL > 1` |
| `#else` / `#endif` | `//#endif` |
| `#define` / `#undef` | `//#define RETRY=3`（仅当前文件生效） |
| `#error` | `//#error RELEASE 下必须定义 NEED_DB` |

表达式支持 `defined(X)`、数值比较（`> >= < <= == !=`）、逻辑运算（`&& || !`）、括号优先级——**没有** `and` / `or` / `not` 文字运算符。未定义标识符按 `0` 参与求值。前缀行允许任意缩进；字符串字面量里的指令样文本自动豁免（不会误伤 `"// #ifdef"` 这类内容）。

## 支持的语言

按注释形态分三组 Profile，按扩展名自动识别；也可用 `-P` / `--prefix` 强制或自定义：

| Profile | 前缀 | 语言 |
|---|---|---|
| `c_family` | `//#` | Java · JavaScript · TypeScript · Kotlin · Dart · PHP · Scala · Solidity |
| `hash` | `##` | Python · Ruby · Shell（sh/bash/zsh）· TOML |
| `lua` | `--#` | Lua |

> 选型原则：只收录**没有原生条件编译**的语言。C/C++/Rust/Go/Swift/C# 自带预处理器，请用原生机制。

## 为性能与安全设计

- **快**：SIMD 行扫描（memchr，SSE2/AVX2/NEON 运行时派发）+ rayon 文件级无锁并行 + thin-LTO。单线程全处理吞吐 **2.2–2.6 GiB/s**（M1 芯片实测，10 万行语料 criterion 100 样本）；4 性能核并行 **3.70×**。基准可复现：`cargo bench`
- **零破坏**：除被裁剪的区间外，任何字节原样保留——`\r\n`、tab、BOM、UTF-8 多字节字符不归一化；写回走临时文件 + 原子 rename
- **确定**：dry-run 的 unified diff 可直接管道给 `git apply`；并行度不影响输出顺序；写回摘要走 stderr 不污染管道
- **不越界**：宏作用域限单文件（换来无锁并行）；`--write` 之外的一切模式都不碰文件系统

## 项目级管控：`.alpp`

业务仓库根目录放一个 `.alpp`（TOML），约束团队宏用法、提供默认宏：

```toml
[macros]
define = ["DEBUG", "LEVEL=1"]   # 默认宏，优先级低于命令行 -D/-U

[directives]
deny = ["if", "elif", "define", "undef", "error"]  # 只许写 ifdef/ifndef/else/endif
# allow = ["ifdef", "ifndef", "else", "endif"]     # 或用白名单（deny 命中优先）
```

被禁指令无论在哪个分支（包括被抑制的分支）都会报 `E8` 退出——防滥用是硬约束。`--no-config` 可应急跳过。

## 文档

- [宏规范 v1](docs/MACRO_SPEC.md) — 指令语法、表达式文法、错误码 E1–E8、与 cpp 的差异对照
- [设计文档](docs/DESIGN.md) — 架构、并行模型、SIMD/跨平台策略、基准数据、里程碑
- [AI Agent 技能](skills/alpp-macros/SKILL.md) — 宏语法速查与批量改代码工作流，供 AI agent 按规范书写条件编译
- [AGENTS.md](AGENTS.md) — 本仓库的 AI 维护规范

## License

[MIT](LICENSE)
