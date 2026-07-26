//! Platform 模块：平台相关能力的具体实现层（见 brainstrom/plan.md §2）。
//!
//! Windows 优先实现；音频采集（`crate::audio`，基于 cpal）与
//! 托盘/窗口（`crate::tray` / `crate::window`，基于 Tauri）本身已跨平台，
//! 故这里只收纳"没有现成跨平台方案"的两类能力：全局键鼠监听、系统文本注入。
//! 未来扩展 macOS/Linux 时，在此新增对应的兄弟模块（如 `platform::macos`）。

pub mod windows;
