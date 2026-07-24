//! Global Input Monitor：全局键鼠事件监听，用于识别 dismiss 信号（见 brainstrom/plan.md §8.3）。
//!
//! 需要全局监听（不局限于 DM 自身窗口获得焦点），因为用户听写时通常在别的应用里输入。
//! 平台相关的具体实现见 `crate::platform::windows::input_hook`。

/// 全局键鼠事件的粗分类：DM 不关心具体按键/坐标，只关心"发生了任意一次操作"。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputEventKind {
    Keyboard,
    MouseLeft,
    MouseRight,
    MouseMiddle,
    /// 鼠标侧键（前进/后退键等）
    MouseSide,
}

/// 监听过程中可能发生的错误。
#[derive(Debug, Clone, PartialEq)]
pub struct MonitorError(pub String);

/// dismiss 事件回调类型。
pub type DismissCallback = Box<dyn Fn(InputEventKind) + Send>;

/// 全局键鼠监听接口。仅在 `State::Loading` / `State::Dictating` 期间启用
/// （对应 `TransitionEffect::StartGlobalInputMonitor` / `StopGlobalInputMonitor`）。
pub trait GlobalInputMonitor {
    /// 开始监听；任意一次键盘/鼠标事件都会触发回调（视为 dismiss）。
    fn start(&mut self, callback: DismissCallback) -> Result<(), MonitorError>;
    fn stop(&mut self);
    fn is_monitoring(&self) -> bool;
}
