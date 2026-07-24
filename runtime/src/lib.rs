//! DictatingMe Runtime crate 根：见 brainstrom/plan.md 全文，
//! 尤其 §5 模块设计表 与 §2 技术栈选型。

pub mod state_machine;
pub mod audio;
pub mod models;
pub mod text;
pub mod input_monitor;
pub mod storage;
pub mod tray;
pub mod window;
pub mod platform;
pub mod runtime;
pub mod commands;
pub mod events;
