//! Window Manager（见 brainstrom/plan.md §3.2、§5）。
//!
//! 根据 State Machine 广播的状态互斥显示 MainWindow 或 HudWindow：
//! 同一时刻有且只有一个可见。`Configure` 显示 MainWindow，其余四态显示 HudWindow。

use crate::state_machine::{HudLight, WindowKind};

/// 窗口操作错误。
#[derive(Debug, Clone, PartialEq)]
pub struct WindowError(pub String);

/// Window Manager 接口。
pub trait WindowManager {
    /// 依据 State Machine 给出的目标窗口做互斥切换（显示其一、隐藏另一个）。
    fn apply_visible_window(&mut self, visible: WindowKind) -> Result<(), WindowError>;

    fn show_main_window(&mut self) -> Result<(), WindowError>;
    fn hide_main_window(&mut self) -> Result<(), WindowError>;

    fn show_hud_window(&mut self) -> Result<(), WindowError>;
    fn hide_hud_window(&mut self) -> Result<(), WindowError>;

    /// 更新 HUD 灯光颜色（Listening=黄，Loading/Dictating=绿，Unloading=灭）。
    fn set_hud_light(&mut self, light: HudLight) -> Result<(), WindowError>;
}
