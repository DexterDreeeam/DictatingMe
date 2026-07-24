//! HUD 悬浮窗定位（见 brainstrom/plan.md §7）。
//!
//! 固定在主屏幕顶部居中，距离屏幕顶部约 80px；不跟随鼠标，多屏时只在主屏幕显示。

/// 屏幕坐标（像素，主屏幕坐标系）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HudPosition {
    pub x: i32,
    pub y: i32,
}

/// HUD 距屏幕顶部的固定偏移（像素），见 plan.md §7。
pub const HUD_TOP_OFFSET_PX: i32 = 80;

/// 根据主屏幕宽度与 HUD 自身宽度，计算"顶部居中"的坐标。
pub fn compute_hud_position(primary_monitor_width: i32, hud_width: i32) -> HudPosition {
    todo!()
}
