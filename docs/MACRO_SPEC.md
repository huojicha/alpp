# ALPP 宏规范（MACRO_SPEC v1）

- 本规范定义 ALPP 识别的指令语法、求值语义与输出规则。实现必须与本规范逐条对应；规范变更需升版本号。
- 本规范刻意是 **C 预处理器条件编译的严格子集** + 面向任意语言的嵌入方式调整，差异全部列于 §8。

## 1. 设计原则

1. **注释内嵌**：指令必须位于宿主语言的行注释内。任何不含 ALPP 处理器的工具链（编译器、IDE、lint、测试）看到的都是普通注释，开发树语法合法性不受影响。
2. **显式标记**：代码作者显式决定哪些块参与条件编译；ALPP 不做任何基于命名约定的推断。
3. **文件内封闭**：一个文件内的指令集合与 `-D/-U` 完全决定输出；文件之间无宏传播。
4. **保守解析**：默认只认 §3 指令表中的**精确关键字**；其余内容永不触碰（宽松模式）。

## 2. 词法结构

### 2.1 指令行构成

```
directive-line  ::= indent prefix keyword [sp args] line-ending
indent          ::= [ \t]*                       // 允许缩进
prefix          ::= profile.prefix               // "//#" | "##" | "--#"
keyword         ::= §3 指令表中的精确关键字
line-ending     ::= "\n" | "\r\n"
```

- 指令必须**独占一行**：前缀之前只允许空白，行尾不允许任何额外内容。
- 关键字与前缀之间**不允许空白**（`//# if` 不是指令）；关键字与参数之间要求 ≥1 个空白。
- 判定为指令后，**整行从输出中移除**（§6）。

### 2.2 Profile 与宿主注释对照

| Profile | prefix | 宿主语言 |
|---|---|---|
| `c_family` | `//#` | Java/JS/TS/Kotlin/Dart/PHP/Scala/Solidity |
| `hash` | `##` | Python/Ruby/Shell/TOML |
| `lua` | `--#` | Lua（`--` 行注释内嵌指令） |

**选型原则**：只收录**没有原生条件编译**的语言。有原生机制的（C/C++ 的 cpp 宏、Rust 的 cfg/feature、Go 的 build tag、Swift/C# 的 `#if`）请直接使用原生能力；标记性语言（HTML/XML 及其派生）不提供支持。

Profile 按扩展名自动选择（映射表见 `src/profile.rs`），可被 `--profile`/`--prefix` 覆盖。

### 2.3 字符串字面量豁免

若前缀的起始位置位于本行一个**未闭合的字符串字面量**内（`'…'` 或 `"…"`，转义 `\'`/`\"` 正确处理），该行不是指令行。约定上**禁止在字符串字面量内书写指令形式文本**（见 §7 错误码 E6 与 §8 差异 3）。

## 3. 指令集

| 指令 | 参数 | 作用 |
|---|---|---|
| `#ifdef NAME` | 宏名 | NAME 已定义 → 保留本块 |
| `#ifndef NAME` | 宏名 | NAME 未定义 → 保留本块 |
| `#if EXPR` | 表达式（§4） | EXPR 非零/为真 → 保留本块 |
| `#elif EXPR` | 表达式 | 前面分支都未保留且 EXPR 为真 → 保留（每层至多生效一次，可与 `#else` 共存） |
| `#else` | 无 | 前面分支都未保留 → 保留本块（每层至多一个） |
| `#endif` | 无 | 关闭最近一层条件块 |
| `#define NAME [VALUE]` | 宏名 [值] | 定义宏；VALUE 省略则为 `Defined`；本行永远从输出移除 |
| `#undef NAME` | 宏名 | 取消定义；本行永远从输出移除 |
| `#error MSG` | 任意文本 | 在**活跃**分支中出现 → 文件级致命错误 E5；本行从输出移除 |

- `NAME` 文法：`[A-Za-z_][A-Za-z0-9_]*`。**约定宏名全大写下划线**（`RELEASE`、`VERBOSE_LOG`），工具不强制。
- V1 关键字表**固定**，不支持用户扩展关键字。`#if` 之外的 cpp 指令（`#include`/`#pragma`…）一律不识别——在 `c_family` 中它们是普通注释。

## 4. 表达式文法与求值

### 4.1 EBNF

```ebnf
expr    ::= or
or      ::= and { "||" and }
and     ::= eq { "&&" eq }
eq      ::= rel { ("==" | "!=") rel }
rel     ::= unary { ("<" | ">" | "<=" | ">=") unary }
unary   ::= "!" unary | primary
primary ::= "(" expr ")"
          | "defined" "(" NAME ")"
          | NAME            // 未定义 → 0；已定义 → 其数值（Defined 视为 1，Str 视为 0）
          | integer         // [0-9]+，十进制
```

- 优先级从低到高：`||` < `&&` < `==`/`!=` < `< > <= >=` < `!` < 括号。短路求值。
- 整数比较为 `i64` 语义；布尔上下文中非零为真。
- 非法表达式 → E7（不回退为注释处理，因为关键字已精确匹配）。

### 4.2 宏表与作用域

- 初始宏表由 CLI 构造：`-DNAME`、`-DNAME=VALUE` 定义；`-UNAME` 删除（后写者胜，`-D A -U A` 结果为未定义）。
- 文件内 `#define/#undef` **顺序生效**：仅影响其后求值的表达式。文件结束时宏表丢弃。
- V1 不支持宏展开后再求值（`-DA=BB` 不会展开 `BB`）；`NAME` 直接绑定其定义值。

## 5. 条件块语义

1. `#if*` 入栈一层 Frame；`#endif` 出栈。**必须配平**：栈非空到文件尾 → E1；`#else/#elif/#endif` 无对应层 → E3。
2. 每层 Frame 维护 `taken: bool`（是否已有分支被保留）：
   - `#elif` 仅当 `!taken` 时求值，为真则 `taken = true` 并保留本分支至下一个 `#elif/#else/#endif`；
   - `#else` 仅当 `!taken` 时保留，并置 `taken = true`；重复 `#else` → E2。
3. **外层抑制**：只要任一外层 Frame 处于「未保留」状态，内层一切指令只做配平登记，不求值、不产生 `#error`。由此保证语义与 cpp 一致。
4. 指令行与被裁剪区间都生成删除编辑；保留区间的字节零修改。

## 6. 输出规则

| 内容 | 输出行为 |
|---|---|
| 指令行（含 `#define/#undef/#error`） | 整行删除（含行尾符） |
| 处于「未保留」分支的普通代码行 | 整行删除（含行尾符） |
| 处于「保留」分支的普通代码行 | 原样保留（字节级保真：缩进、tab、`\r\n`、BOM、UTF-8） |
| 文件中无任何指令 | 输出 = 输入（不产生 diff、不写盘） |

删除粒度是**整行**；ALPP 不做行内拼接或改写。

## 7. 错误码

| 码 | 名称 | 触发 |
|---|---|---|
| E1 | Unterminated | 文件结束仍有未配平的条件层 |
| E2 | DuplicateElse | 同层出现第二个 `#else` |
| E3 | OrphanDirective | `#else/#elif/#endif` 无对应开层 |
| E4 | UnknownDirective | `--strict` 下，前缀后跟非关键字表的 `#` 指令样文本 |
| E5 | UserError | 活跃分支中的 `#error MSG` |
| E6 | StringEmbedded | 前缀出现在字符串字面量内且 `--strict`（默认宽松模式 降级为警告） |
| E7 | BadExpression | 表达式语法/类型错误 |
| E8 | ForbiddenDirective | 指令关键字被 `.alpp` 配置 `[directives]` 节管控（deny 名单或 allow 名单外）；无论分支是否活跃均报错（§10） |

## 8. 与 C 预处理器的差异对照

| # | cpp | ALPP v1 | 理由 |
|---|---|---|---|
| 1 | 指令必须 `#` 顶格（或空白后） | 前缀为语言注释 + `#`，允许缩进 | 非 C 语言中裸 `#` 非法 |
| 2 | 关键字与 `#` 间允许空白（`#  ifdef`） | 不允许 | 消除 hash 语言普通注释误判 |
| 3 | 字符串字面量中的 `#ifdef` 文本会被解析 | 字符串字面量豁免 + E6 | 修复实测误伤（filepp/`"// #ifndef"` 案例） |
| 4 | `#define` 支持函数宏、`#`/`##` 拼接 | 仅对象宏 | 覆盖场景足够，解析成本大幅下降 |
| 5 | 宏展开递归多遍 | 无展开，直接绑定值 | 可预测性 |
| 6 | 未定义标识符在 `#if` 中替换为 `0` | 相同 | 语义对齐 |
| 7 | `#include` 跨文件 | 不支持 | 文件级并行与封闭性（DESIGN §4.7） |
| 8 | `#error` 使编译失败 | E5 文件级错误，聚合后统一退出 | 批处理友好 |

## 9. 多语言示例

Java（`c_family`，`alpp -DRELEASE`）：

```java
void process(String userId, String action) {
    //#ifndef RELEASE
    log.debug("user {} did {}", userId, action);
    //#endif
    //#if DEBUG && !RELEASE
    dumpState(userId);
    //#elif VERBOSE
    trace.println(process);
    //#endif
}
```

`-DRELEASE` 输出：`log.debug` 行、`dumpState` 行被删除，`trace` 分支删除（DEBUG 未定义），其余原样；所有指令行消失。

Python（`hash`，`alpp -DVERBOSE_LOG`）：

```python
##ifndef VERBOSE_LOG
logger.debug("user %s did %s", user_id, action)
##else
pass  ## release 占位，保持块非空
##endif
```

注意 Python 缩进块的**非空约束**：被裁剪后块内必须仍有语句，作者需用 `#else pass` 或 `##if` 反转（写 `##ifndef` 使日志分支为保留侧）规避——这是行级删除模型的固有限制，写入编码规范。
```

## 10. 配置文件 `.alpp`

业务仓库可在根目录放置 `.alpp`（TOML）约束宏用法、提供默认宏：

```toml
[macros]
# 默认宏初值（`NAME` 或 `NAME=VALUE`），优先级最低，被命令行 -D/-U 覆盖
define = ["DEBUG", "LEVEL=1"]
undefine = []

[directives]
# 指令级语法管控：限制项目允许使用哪些宏语句（如只许 ifdef/ifndef/else/endif）
# 名单只能取九个关键字：if/ifdef/ifndef/elif/else/endif/define/undef/error
deny = ["if", "elif", "define", "undef", "error"]
# 或用白名单（可选）：名单外指令一律违规；deny 命中优先
# allow = ["ifdef", "ifndef", "else", "endif"]
```

- **发现规则**：从第一个输入路径（目录本身；文件则取其父目录）向上逐级查找，找到即停；未找到视为空配置。多输入路径以第一个为锚，全进程一份。
- **优先级**（从低到高）：配置 define → 配置 undefine → 命令行 -D → 命令行 -U（后写者胜）。
- **E8 管控语义**：`[directives]` deny/allow 是硬约束，对文件内指令无论所在分支是否活跃均报错（与 E5 的“活跃才触发”不同）；错误格式 `<文件>:<行>: E8: <详情>`，退出码 2。宏名不受配置管控（CLI -D/-U 任意宏均可）；需要禁写 `#define` 时用 `[directives] deny` 拦指令本身。
- **配置解析失败**（坏 TOML、未知字段、非法宏名/指令名）→ 启动错误，退出码 1。
- `--no-config` 跳过发现（逃生门）。

## 11. 版本化

- 规范版本写入 ALPP `--version` 输出；不兼容变更升主版本。
- V1 冻结指令集；后续候选特性（块注释指令、`.alpp.toml`、宏展开）在 DESIGN §7 跟踪。
