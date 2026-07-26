//! Window 模块：MainWindow ⇄ HudWindow 互斥显示管理（见 brainstrom/plan.md §3.2）。

pub mod hud_position;
pub mod window_manager;

pub use hud_position::{compute_hud_position, HudPosition, HUD_TOP_OFFSET_PX};
pub(crate) use window_manager::create_tauri_window_manager;
pub use window_manager::{WindowError, WindowManager};
