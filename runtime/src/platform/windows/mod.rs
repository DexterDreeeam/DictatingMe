//! Windows 平台相关实现（见 brainstrom/plan.md §2：Windows 优先，架构预留 macOS/Linux）。

pub mod input_hook;
pub mod send_input;

pub use input_hook::WindowsInputMonitor;
pub use send_input::WindowsTextInjector;
