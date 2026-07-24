//! Tray Manager（见 brainstrom/plan.md §3.3、§5、§7）。
//!
//! 创建/持有系统托盘图标（固定不变，不随状态改变）；
//! 左键点击 = 向 State Machine 发出"打开 MainWindow 请求"；右键菜单提供"退出"。
//! Tray、Runtime、SystemTray 三者共生共灭（见 plan.md §3.3 关键生命周期事实）。

/// 托盘操作错误。
#[derive(Debug, Clone, PartialEq)]
pub struct TrayError(pub String);

/// 左键点击回调：等价于向 State Machine 发出 `StateEvent::OpenMainWindowRequested`。
pub type OpenMainWindowCallback = Box<dyn Fn() + Send>;
/// 右键菜单"退出"回调：终止整个 Runtime 进程。
pub type ExitCallback = Box<dyn Fn() + Send>;

/// Tray Manager 接口。
pub trait TrayManager {
    /// 创建托盘图标（图标固定，不随状态变化，见 plan.md §7）。
    fn create(
        &mut self,
        on_open_main_window: OpenMainWindowCallback,
        on_exit: ExitCallback,
    ) -> Result<(), TrayError>;

    fn destroy(&mut self);
}
