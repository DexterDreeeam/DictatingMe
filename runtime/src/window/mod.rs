//! Window 模块：MainWindow ⇄ HudWindow 互斥显示管理（见 brainstrom/plan.md §3.2）。

pub mod window_manager;
pub mod hud_position;

pub use window_manager::{WindowError, WindowManager};
pub use hud_position::{compute_hud_position, HudPosition, HUD_TOP_OFFSET_PX};
