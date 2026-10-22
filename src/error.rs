//! 错误与诊断（MACRO_SPEC §7 错误码；DESIGN §4.10 退出码）。

use std::fmt;
use std::path::Path;

/// 单个文件的处理错误。`code` 为 E1–E7 的数字部分。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileError {
    pub path: String,
    pub line: usize,
    pub code: u8,
    pub message: String,
}

impl fmt::Display for FileError {
    /// 诊断格式：`<path>:<line>: E<n>: <message>`（DESIGN §4.10）
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}:{}: E{}: {}",
            self.path, self.line, self.code, self.message
        )
    }
}

impl std::error::Error for FileError {}

/// 构造指令类错误的便捷函数。
pub fn directive_error(path: &Path, line: usize, code: u8, message: String) -> FileError {
    FileError {
        path: path.display().to_string(),
        line,
        code,
        message,
    }
}
