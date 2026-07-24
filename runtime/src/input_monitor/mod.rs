//! Input Monitor 模块：全局键鼠监听与 dismiss 判定（见 brainstrom/plan.md §8.3）。

pub mod dismiss;

pub use dismiss::{DismissCallback, GlobalInputMonitor, InputEventKind, MonitorError};
