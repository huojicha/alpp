# ALPP 设计文档

- 版本：0.1（对应 MACRO_SPEC v1）
- 状态：Draft
- 术语约定：本文中「指令」指 `MACRO_SPEC.md` 定义的注释内嵌条件编译指令；「宏」指指令操作的条件变量。

## 1. 目标与非目标

### 1.1 目标

1. **语言无关的条件编译**：开发者用注释形式在源码中显式写 `#ifdef` 风格指令，ALPP 在发布构建时按宏定义裁剪代码块。不改动开发树的语法合法性（指令永远位于注释内）。
2. **多平台**：纯 Rust 实现，覆盖 `x86_64/aarch64 × linux/darwin/windows`，Linux 提供 musl 静态链接产物。
3. **多线程**：文件级数据并行，吞吐随核数近线性扩展；文件间零共享状态。
4. **SIMD 加速**：行扫描的热路径（定位指令行）使用向量化字节搜索（SSE2/AVX2/NEON，运行时派发），目标是单线程扫描吞吐 ≥ 1 GB/s。
5. **安全默认**：默认 dry-run 输出 diff，显式 `--write` 才落盘；`--check` 供 CI 门禁。

### 1.2 非目标（V1）

- 不做完整词法/语法解析（不引入 tree-sitter；行级状态机 + 字符串字面量启发式即可满足需求）。
- 不支持跨文件的宏作用域（保证文件级并行与增量正确性）。
- 不做宏函数、字符串拼接等 C 预处理器全量特性。
- 不处理块注释内嵌指令（`/* #ifdef */`），V1 仅行注释/`#` 注释/XML 注释（见 §7 开放问题）。

### 1.3 与既有方案的差异

| 方案 | 缺陷 | ALPP 的改进 |
|---|---|---|
| filepp（Perl） | `#` 注释语言（Python）中普通注释易误判为指令；字符串字面量中的指令文本会被误解析；无 dry-run/diff | 关键字精确匹配 + 宽松默认模式（§4.5）；字符串字面量启发式（§4.6）；内置 diff/check |
| gpp | 默认模式条件语义与 cpp 不一致；cpp 模式要求指令顶格（非 C 语言非法）；tab 转空格 | 语义严格对齐 cpp；指令允许缩进；逐字节保留原文 |
| cpp 直接跑非 C 文件 | 撇号/引号导致解析错误、行标记污染 | 无 cpp 兼容负担，面向任意文本 |
| ast-grep 模式剥离 | 需维护每语言规则，无显式标记 | ALPP 显式标记，零规则维护；二者可互补使用 |

## 2. 总体架构

```
                    ┌────────────────────────────────────────────┐
 CLI (clap) ───────►│ Pipeline                                   │
  -D/-U 宏表        │                                            │
  --profile/--prefix│  Walker (ignore::WalkBuilder, 并行遍历,     │
                    │   gitignore 感知)                          │
                    │    │  按扩展名 → Profile (src/profile.rs)   │
                    │    ▼                                       │
                    │  每文件任务 (rayon worker, 文件间无共享)     │
                    │    Scanner  ── SIMD 行分类 (src/scanner.rs) │
                    │    Parser   ── 指令/块状态机 (src/parser.rs)│
                    │    Evaluator── 表达式求值    (src/eval.rs)  │
                    │    Rewriter ── 区间删除/写回 (src/rewriter) │
                    │    ▼                                       │
                    │  结果聚合: diff 打印 / 落盘 / 错误汇总       │
                    └────────────────────────────────────────────┘
```

模块职责：

| 模块 | 职责 | 关键数据结构 |
|---|---|---|
| `cli.rs` | 参数解析与校验（已实现） | `Args` |
| `profile.rs` | 扩展名 → 指令前缀/后缀映射（已实现） | `Profile` |
| `walker.rs` | 并行文件发现、二进制剔除、Profile 绑定 | `FileJob { path, profile }` |
| `scanner.rs` | SIMD 行分类：快速判定每行是否可能是指令行 | `LineKind { Code, MaybeDirective }` |
| `parser.rs` | 指令解析 + 块状态机（嵌套栈）+ 字符串字面量跳过 | `Directive`, `BlockState` |
| `eval.rs` | 布尔表达式求值、宏表管理 | `MacroTable`, `Expr` |
| `rewriter.rs` | 依据「保留区间」拼装输出；in-place/stdout/diff | `Edit { byte_range, action }` |
| `report.rs` | 诊断信息（文件:行:列）、退出码、统计 | `Diagnostic` |

## 3. 数据流与不变量

1. **输入**：`Vec<PathBuf>` → walker 展开为 `FileJob` 流。
2. **每文件处理是纯函数**：`(bytes, profile, macro_table, flags) → Result<TextEdit list, FileError>`。除诊断收集外无共享可变状态，这是并行的根基（不变量 I1）。
3. **重写是区间替换**：解析阶段产出 `Vec<Edit>`（删除指令行、删除被裁剪块的整行区间），rewriter 一次性拼装，保证单遍 O(n)（不变量 I2）。
4. **行尾与编码保真**：除被删除区间外，任何字节（含 `\r\n`、tab、BOM、UTF-8 多字节字符）原样保留（不变量 I3）。

## 4. 关键设计

### 4.1 CLI 与运行模式

三种模式互斥：

| 模式 | 行为 |
|---|---|
| 默认（dry-run） | 对每个发生改动的文件输出 unified diff 到 stdout；无改动文件沉默 |
| `--write` | 原地写回（写临时文件 + rename，保证原子性）；`-o <dir>` 可选镜像输出到新目录树 |
| `--check` | 只验证指令合法性（块配平、strict 检查），不改写；任何错误 → 退出码 2。用于 CI 门禁与「源码不得含字符串内指令」守卫 |

全局宏来源优先级：文件内 `#define` > `--undefine (-U)` > `--define (-D)`（后两者都作用于文件起始时刻的宏表初值，`-U` 用于压制 `-D` 或内置宏）。文件内指令的生命周期仅限该文件（§4.7）。

### 4.2 语言 Profile 与前缀

- 内置三个 profile：`c_family`（`//#`）、`hash`（`##`）、`lua`（`--#`），映射表见 `src/profile.rs`。
- 选型原则：只收录**没有原生条件编译**的语言；有原生机制的（C/C++/Rust/Go/Swift/C# 等）与标记性语言（HTML/XML）不支持。
- 覆盖手段（优先级从高到低）：`--prefix`（全局强制）> `--profile`（强制某 profile）> 按扩展名自动 > 跳过（未知扩展名不处理，除非显式传入该文件——显式文件使用 `c_family` 兜底并告警）。
- 选 `##` 作为 hash 语言前缀的依据：调研期实测，`#` 单字符前缀会把普通注释（如 `# 注释`）误判为指令；`##` 在注释正文中出现概率趋近于零。
- 前缀匹配允许行首缩进（任意水平空白），兼容代码内缩进的块。

### 4.3 文件发现（walker）

- 基于 `ignore` crate：尊重 `.gitignore`/`.ignore`，隐藏文件默认跳过，支持 `--no-ignore` 逃生门。
- 二进制剔除：读头部 8KB，含 NUL 字节即跳过（与 git 判定一致）。
- 单文件上限：默认 64MB，超出跳过并告警（防御性；可通过 `--max-size` 调整）。

### 4.4 行扫描（SIMD 热路径）

源文件中指令行占比通常 <1%，扫描层的唯一任务是**廉价地排除非指令行**：

1. **第一级（SIMD 字节过滤）**：对每个 profile 预计算前缀锚字节（`//#` → `/`，`##` → `#`，`--#` → `-`）。用 `memchr` 在整块缓冲区上搜索锚字节——memchr 内部按平台派发 SSE2/AVX2（x86_64，`is_x86_feature_detected` 运行时检测）或 NEON（aarch64 基线指令），一次处理 16~64 字节。锚字节都不存在的行直接标记 `Code`，成本摊薄为每行一次向量比较。
2. **第二级（精确前缀确认）**：对命中锚字节的行做行内定位：跳过行首水平空白后 `memcmp` 完整前缀；再校验后续字符构成合法指令关键字（`MACRO_SPEC.md` §3 精确匹配表）。
3. **分块处理**：大文件按 1MB 块读入，以最后一个 `\n` 为块边界对齐（不变量 I3），避免整文件驻留内存。
4. **字符串字面量启发式**：对 `MaybeDirective` 行，从行首到前缀位置做一次引号奇偶扫描（`'`/`"`，含转义 `\"`）。若前缀落在未闭合引号内，则该行不是指令（消除 filepp 实测的字符串误伤缺陷）。已知局限：行内跨行的原始字符串不在 V1 覆盖范围，记入 §7。

SIMD 策略说明：不手写 intrinsics，依赖 `memchr` 的向量化实现 + 运行时特性检测，天然获得多平台覆盖；单字节锚搜索是本问题形态下收益/复杂度比最高的 SIMD 落点（分支预测友好的两级过滤，而非对整行做正则）。

### 4.5 指令解析与块状态机

- 逐行喂入 `MaybeDirective`/`Code`，状态机维护**条件栈**：

```
enum Frame { Active, InactiveSkip }   // 每层 if/ifdef 一个 Frame
state = Vec<Frame> + 当前层是否已取过 else
```

- 关键规则（详见 MACRO_SPEC §5）：
  - 栈顶任一外层为 `InactiveSkip` 时，内层指令只参与配平计数、不求值；
  - `#else/#elif` 翻转仅当同层未翻转过；
  - 指令行本身、被裁剪区间的行 → 生成 `Edit::Delete`；保留区间不动（不生成 Edit）。
- **宽松默认模式**：以 `#` 开头但不匹配指令关键字表精确字段的行一律视为普通注释（filepp 误伤的根治）；`--strict` 下则报错（错误码 E4）。
- `#error MSG`：在活跃分支中出现即产生该文件的致命错误（E5）。

### 4.6 表达式求值（eval）

- 文法与求值顺序见 MACRO_SPEC §4。实现为递归下降，输入是前缀后的字节切片，输出 `bool`/`i64`。
- `defined(NAME)` 查询宏表；`#if` 中未定义标识符按 `0` 处理（与 cpp 一致），`--strict` 下改为警告。
- 宏表：`HashMap<String, Value>`；`Value = Defined | Num(i64) | Str`（`-DX=1`、`-DX=prod`）。

### 4.7 并行模型

- **文件级并行**：`ignore::WalkBuilder::build_parallel` 的 visitor 在每线程内独立完成「读 → 扫 → 解析 → 求值 → 重写 → 写」，无跨文件宏传播（设计取舍：牺牲 `#include` 类全局配置，换取 I1 无锁并行）。
- 线程数：默认 = 可用核数；`-j N` 覆盖。
- 诊断聚合：每文件返回 `Result<(), FileError>`，主线程经 channel 收集，**按文件路径排序后**统一输出（保证 diff/错误输出确定性，不随调度顺序抖动）。
- 输出契约：stdout 只承载 dry-run 的 unified diff（可安全管道）；错误、写回 IO 错误与摘要均走 stderr。`--write` 有变更时默认输出一行摘要 `alpp: processed=N changed=N errors=N skipped=N`（无变更静默）；`--verbose` 另加逐文件 written 行与 skipped 明细。
- 写回安全：`--write` 时先写 `<path>.alpp-tmp` 再 `rename`；检测到输出与输入相同则跳过写盘。

### 4.8 配置文件 `.alpp`（M2 提前落地 ✅）

- 业务仓库可放置 `.alpp`（TOML）约束宏用法：`[macros] define/undefine` + `[directives] deny/allow`（格式与语义见 MACRO_SPEC §10）；发现规则：从第一个输入路径向上逐级查找，找到即停。
- **与并行模型的关系**：配置只做校验（`[directives]` deny/allow → E8）与宏表初值，不参与条件求值状态机，发现一次后全进程共享只读，不破坏 4.7 的无锁模型。
- 优先级（低→高）：配置 define → 配置 undefine → `-D` → `-U`；`--no-config` 逃生门。
- E8 管控对抑制分支同样生效（出现即违规），与 E5 的“活跃才触发”不同。
- `[directives]` 节做指令级语法管控（E8 ForbiddenDirective）：限制项目可用的宏语句（如只许 ifdef/endif）；名单值校验限定九个关键字，非法名 → 配置解析失败退出 1。`[macros]` 节只保留 define/undefine（默认宏，优先级低于 CLI）；宏名不做配置管控。
- 新增依赖：`toml` + `serde`（配置解析）。

### 4.9 跨平台策略

- 依赖全部为纯 Rust crate（clap/ignore/memchr/rayon/anyhow/thiserror/serde/toml），无 C 工具链依赖，`cross` 即可交叉编译。
- 产物矩阵：`x86_64-unknown-linux-gnu/musl`、`aarch64-unknown-linux-gnu/musl`、`x86_64-apple-darwin`、`aarch64-apple-darwin`、`x86_64-pc-windows-msvc`。
- SIMD 派发：x86_64 运行时检测 SSE2/AVX2；aarch64 NEON 为基线无条件启用；其他平台回退标量 memchr（纯 Rust 实现，仍有不错的吞吐）。
- 行尾：`\n` 与 `\r\n` 都识别为行界，删除区间时连同行尾符一起删除，不引入行尾归一化。
- 路径：一律 `std::path`，不拼接字符串路径。

### 4.10 性能目标与基准

| 指标 | 目标 | M1 实测（M1 芯片，6P+6E，release+lto） |
|---|---|---|
| 单线程全处理吞吐（d0 纯代码） | ≥ 1 GB/s | **2.21 GiB/s** ✅ |
| 单线程全处理吞吐（d1 典型发布树） | ≥ 1 GB/s | **2.58 GiB/s** ✅ |
| 单线程全处理吞吐（d10 重标注） | ≥ 1 GB/s | 1.93 GiB/s ✅ |
| 仅扫描（line_iter） | — | 3.70 GiB/s |
| 仅扫描（锚字节预筛） | — | 2.44 GiB/s |
| 并行扩展 t4/t8（8 文件） | 8 核 ≥ 6× | t4 = **3.70×**，t8 = 2.67× |

并行说明：实测机为异构核（6 性能核 + 6 能效核）。t4 全部落在性能核上近线性扩展；
t8 的 2 个线程被调度到能效核成为长尾，加速比反而回落。同构 8 核平台（Linux CI）
预计可达 ≥6×；`-j` 参数已支持用户按核型自选线程数。10 万行（~3.6 MB）确定性语料，
criterion 100 样本；数据可由 `cargo bench` 复现。

基准用 criterion（`benches/process.rs`）：确定性合成语料（xorshift64 种子固定，跨机器可比）× {0%, 1%, 10% 指令密度} × {全处理, 仅扫描, 多线程}。回归门槛进 CI。

### 4.10 错误处理与退出码

| 码 | 含义 |
|---|---|
| 0 | 成功（含 dry-run 有改动） |
| 1 | 运行错误（IO、路径不存在、未知 profile/宏名） |
| 2 | 指令校验失败（未闭合 E1、else 重复 E2、外层失配 E3、strict 未知指令 E4、#error E5、字符串内指令 E6、表达式错误 E7） |
| 3 | `--check` 发现未剥离残留（门禁语义，M4 实现；当前 `--check` 仅校验指令合法性，失败时退出码 2） |

诊断格式：`<path>:<line>:<col>: E<n>: <message>`，全部汇总在 stderr，多错误不中断其他文件的处理（最终以最高严重级退出码退出）。

## 5. 测试策略

1. **单元测试**：parser 状态机表驱动用例（嵌套/else/未闭合/字符串内前缀/缩进/`\r\n`）；eval 文法用例（优先级、defined、未定义标识符）。— M1 已落地，39 个。
2. **集成测试**：`tests/integration.rs` 以真实二进制端到端验证（临时文件树 + `CARGO_BIN_EXE`），覆盖三模式、`-D/-U`、`--profile`、`--strict`、退出码 0/1/2、gitignore/二进制跳过、CRLF 保真、镜像树完整性；`tests/languages.rs` 按语言专项验证（5 种语言 × 3 Profile，只收录无原生条件编译的语言），采用 golden file：每种语言一个全语法参考 `tests/fixtures/<lang>/all.<ext>`（java 为 All.java；共 20 个文件 = 5 输入 + 15 期望），Rust 侧只保留测试逻辑，全部断言为完整字符串逐字节对比。all fixture 覆盖 §3 指令全集（ifdef/ifndef/if/elif/else/endif/define/undef/error）+ §4 表达式全集（defined/数值比较/逻辑短路/括号优先级）+ §5 块语义（嵌套/外层抑制）+ §6 输出规则（字符串豁免、缩进指令），头部注释即使用说明，可直接作为写代码时的语法参考；三场景（default、`-DFULL -DLEVEL=2`、`-DNEED_DB` 触发 E5）。— M1 已落地，17 + 5 个。
3. **属性测试**（proptest，后续）：随机合法指令嵌套 → 输出必须语法合法且与手算裁剪结果一致。
4. **模糊测试**（cargo-fuzz，后续）：fuzz parser 的行输入，要求无 panic、无不配平漏报。
5. **对拍**：以 filepp 在同一样本上的输出做交叉验证（语义子集内必须一致），已在调研期验证过 filepp 行为基线。
6. **CI**（GitHub Actions，`.github/workflows/ci.yml`）：lint job（`cargo fmt --check` + `cargo clippy --all-targets -- -D warnings`）+ test job（ubuntu/macos/windows 矩阵跑 `cargo test` 与 `cargo bench --no-run` 编译检查）；本地提交前四步均可本地预验证。— M1 已落地。

## 6. 里程碑

| 里程碑 | 内容 | 验收 |
|---|---|---|
| M1 ✅ | scanner + parser + eval + rewriter 闭环（含 rayon 文件级并行）；三模式 CLI | 单元 45 + 集成 27 + 语言专项 5 全绿；多语言（java/ts/py/sh/lua）完整字符串对比通过，每语言一个全语法参考 fixture；基准数据见 §4.10 |
| M2 | 诊断聚合增强、`-o` 镜像补齐未处理文件、loom 并发测试；~~`.alpp` 配置文件~~ ✅ 已提前落地（§4.8） | 8 核扩展 ≥6×（已有基准）；并发竞争测试 |
| M3 | SIMD 基准与调优、release profile、跨平台产物 | 吞吐 ≥1GB/s；五平台产物出包 |
| M4 | `--check` 残留门禁（退出码 3）、proptest/fuzz、对拍 filepp | CI 齐备 |

## 7. 开放问题

1. 块注释内嵌指令（`/* #ifdef */`）是否值得支持？当前判定：不值得（行注释已覆盖主流语言，块注释引入跨行词法复杂度）。
2. 配置文件（`.alpp.toml`：自定义扩展名映射、项目级宏默认值）放在 M4 之后。
3. `#include` 宏表共享：与无锁并行冲突，除非引入「两阶段：先收集全局 define 文件再并行」；暂不做成核心特性。
4. Go 反引号原始字符串（可跨行）内的指令前缀：V1 已知局限，由编码规范规避（指令不得出现在字符串内）。
