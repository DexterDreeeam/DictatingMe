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
    let monitor_width = i64::from(primary_monitor_width.max(0));
    let hud_width = i64::from(hud_width.max(0));
    let available_width = monitor_width.saturating_sub(hud_width).max(0);
    let centered_x = available_width / 2;

    HudPosition {
        x: i32::try_from(centered_x).unwrap_or(i32::MAX),
        y: HUD_TOP_OFFSET_PX.max(0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn centers_hud_on_primary_monitor() {
        assert_eq!(
            compute_hud_position(1_920, 320),
            HudPosition { x: 800, y: 80 }
        );
    }

    #[test]
    fn clamps_oversized_or_invalid_dimensions() {
        assert_eq!(compute_hud_position(200, 400), HudPosition { x: 0, y: 80 });
        assert_eq!(compute_hud_position(-1, 100), HudPosition { x: 0, y: 80 });
        assert_eq!(
            compute_hud_position(1_000, -100),
            HudPosition { x: 500, y: 80 }
        );
    }
}
