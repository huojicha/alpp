//! 语言 Profile：扩展名 → 指令前缀（MACRO_SPEC §2.2，DESIGN §4.2）。
//!
//! 前缀合法性的设计依据（调研实测）：
//! - `c_family` 用 `//#`：指令天然是行注释，开发树无需工具即合法；
//! - `hash` 用 `##` 而非 `#`：`#` 前缀会把 Python 普通注释误判为指令；
//! - `lua` 用 `--#`：Lua 行注释是 `--`，指令自然落在行注释内。
//!
//! 选型原则：只收录**没有原生条件编译**的语言。有原生机制的（C/C++ 的 cpp 宏、
//! Rust 的 cfg/feature、Go 的 build tag、Swift/C# 的 #if 等）与标记性语言
//! （HTML/XML 及其派生）不在支持范围内。

/// 语言 Profile。生命周期参数支持运行时自定义前缀（--prefix）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Profile<'a> {
    pub name: &'a str,
    /// 打开指令的字节序列，如 "//#"、"##"。匹配时允许行首缩进。
    pub prefix: &'a str,
    pub extensions: &'a [&'a str],
}

pub const C_FAMILY: Profile<'static> = Profile {
    name: "c_family",
    prefix: "//#",
    extensions: &[
        "java", "js", "jsx", "mjs", "cjs", "ts", "tsx", "kt", "kts", "dart", "php", "scala", "sol",
    ],
};

pub const HASH: Profile<'static> = Profile {
    name: "hash",
    prefix: "##",
    extensions: &["py", "pyi", "sh", "bash", "zsh", "rb", "toml"],
};

pub const LUA: Profile<'static> = Profile {
    name: "lua",
    prefix: "--#",
    extensions: &["lua"],
};

pub const ALL: [Profile<'static>; 3] = [C_FAMILY, HASH, LUA];

/// 按扩展名返回 Profile（拷贝，生命周期 'static）。
pub fn by_extension(ext: &str) -> Option<Profile<'static>> {
    ALL.iter().copied().find(|p| p.extensions.contains(&ext))
}

/// 按名称返回 Profile（--profile 覆盖用）。
pub fn by_name(name: &str) -> Option<Profile<'static>> {
    ALL.iter().copied().find(|p| p.name == name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extension_mapping() {
        assert_eq!(by_extension("java").unwrap().name, "c_family");
        assert_eq!(by_extension("ts").unwrap().name, "c_family");
        assert_eq!(by_extension("py").unwrap().name, "hash");
        assert_eq!(by_extension("lua").unwrap().name, "lua");
        // 有原生条件编译的语言与标记性语言不支持
        assert!(by_extension("c").is_none());
        assert!(by_extension("rs").is_none());
        assert!(by_extension("go").is_none());
        assert!(by_extension("swift").is_none());
        assert!(by_extension("cs").is_none());
        assert!(by_extension("html").is_none());
        assert!(by_extension("xml").is_none());
        assert!(by_extension("vue").is_none());
        assert!(by_extension("txt").is_none());
    }

    #[test]
    fn name_lookup_and_custom_prefix() {
        assert_eq!(by_name("hash").unwrap().prefix, "##");
        // 运行时自定义前缀（--prefix）
        let custom = Profile {
            name: "custom",
            prefix: "//@",
            extensions: &[],
        };
        assert_eq!(custom.prefix, "//@");
    }
}
