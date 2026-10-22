# ALPP 宏语法全集参考 —— 对应 docs/MACRO_SPEC.md §3 指令集、§4 表达式、§5 块语义
# 场景：
#   default            无宏（看 else/ifndef 兜底分支）
#   -DFULL -DLEVEL=2   组合宏（看 ifdef/if/elif 数值分支）
#   -DNEED_DB          触发第 9 段 error（退出码 2，E5）
# 每段演示一类语法；用 alpp dry-run 看裁剪、-w 写回。
# hash Profile 前缀是 ##（双井号）：单 # 开头的普通注释不会被误判。

# --- 1. ifdef / else / endif：定义走 if 分支，否则走 else 分支 ---
##ifdef FULL
MODE = "full"
##else
MODE = "lite"
##endif

# --- 2. ifndef / endif：未定义时保留（典型：调试辅助代码） ---
##ifndef NDEBUG
DEBUG_BUILD = True
##endif

# --- 3. defined() 求值；未定义标识符按 0 ---
##if defined(FULL) && !defined(NDEBUG)
def trace_on():
    pass
##endif

# --- 4. 数值比较：-DLEVEL=2 传入数值；宏值不展开，保留行写字面值 ---
##if LEVEL >= 2
VERBOSE_LEVEL = 2
##endif

# --- 5. define/undef 顺序生效：只影响其后的表达式，文件尾丢弃 ---
##define LOCAL_FEATURE
##define RATIO 3
##if LOCAL_FEATURE && RATIO > 2
LOCAL_PATH = 30
##endif
##undef LOCAL_FEATURE
##if LOCAL_FEATURE
NEVER_COMPILED = 1
##endif

# --- 6. elif 链：至多生效一个分支，else 兜底 ---
##if LEVEL >= 3
TIER = "high"
##elif LEVEL == 2
TIER = "mid"
##elif LEVEL == 1
TIER = "low"
##else
TIER = "unset"
##endif

# --- 7. 逻辑运算与短路：|| 左真则不求值右操作数 ---
##if defined(FULL) || LEVEL > 5
FLAGS = 0xFF
##endif

# --- 8. 嵌套与外层抑制：外层未保留时，内层只配平登记（不求值、不触发 error） ---
##ifdef FULL
##ifdef NEVER_DEFINED_XYZ
INNER_NEVER = 1
##error "inner branch never active"
##endif
##endif

# --- 9. error：活跃分支中触发文件级致命错误 E5（退出码 2） ---
##ifdef NEED_DB
##error "NEED_DB build requires ALPP_DB_URL"
##endif

# --- 10. 字符串豁免：字符串字面量内的指令样文本不是指令，整行保留 ---
HINT = "run with ##ifdef FULL to enable"

# --- 11. 缩进指令：前缀前允许空白，函数/类体内照常使用 ---
def main():
    ##ifdef FULL
    print(MODE)
    ##endif

if __name__ == "__main__":
    main()
