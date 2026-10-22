//! ALPP — Any Language PreProcessor.
//!
//! 库入口：各模块与 docs/MACRO_SPEC.md 的章节一一对应。

pub mod cli;
pub mod config;
pub mod error;
pub mod eval;
pub mod pipeline;
pub mod profile;
pub mod rewriter;
pub mod scanner;
pub mod walker;
