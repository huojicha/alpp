// ALPP 宏语法全集参考 —— 对应 docs/MACRO_SPEC.md §3 指令集、§4 表达式、§5 块语义
// 场景：
//   default            无宏（看 else/ifndef 兜底分支）
//   -DFULL -DLEVEL=2   组合宏（看 ifdef/if/elif 数值分支）
//   -DNEED_DB          触发第 9 段 error（退出码 2，E5）
// 每段演示一类语法；用 alpp dry-run 看裁剪、-w 写回。

// --- 1. ifdef / else / endif：定义走 if 分支，否则走 else 分支 ---
export const MODE = "full";

// --- 2. ifndef / endif：未定义时保留（典型：调试辅助代码） ---
export const DEBUG_BUILD = true;

// --- 3. defined() 求值；未定义标识符按 0 ---
export function traceOn(): void {}

// --- 4. 数值比较：-DLEVEL=2 传入数值；宏值不展开，保留行写字面值 ---
export const VERBOSE_LEVEL = 2;

// --- 5. define/undef 顺序生效：只影响其后的表达式，文件尾丢弃 ---
export const LOCAL_PATH = 30;

// --- 6. elif 链：至多生效一个分支，else 兜底 ---
export const TIER = "mid";

// --- 7. 逻辑运算与短路：|| 左真则不求值右操作数 ---
export const FLAGS = 0xFF;

// --- 8. 嵌套与外层抑制：外层未保留时，内层只配平登记（不求值、不触发 error） ---

// --- 9. error：活跃分支中触发文件级致命错误 E5（退出码 2） ---

// --- 10. 字符串豁免：字符串字面量内的指令样文本不是指令，整行保留 ---
export const HINT = "run with //#ifdef FULL to enable";

// --- 11. 缩进指令：前缀前允许空白，函数体内照常使用 ---
export function main(): void {
    console.log(MODE);
}
