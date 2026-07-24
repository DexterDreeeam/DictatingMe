//! Tray 模块：系统托盘的创建与事件转发（见 brainstrom/plan.md §3.3、§7）。

pub mod tray_manager;

pub use tray_manager::{ExitCallback, OpenMainWindowCallback, TrayError, TrayManager};
