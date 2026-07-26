//! 驱动状态转移的事件类型（见 brainstrom/plan.md §4.2 状态转移表）。

/// 触发 `StateMachine::apply` 的输入事件。
#[derive(Debug, Clone, PartialEq)]
pub enum StateEvent {
    /// 用户请求打开 MainWindow（点击托盘图标左键，等价于一次 dismiss）。
    OpenMainWindowRequested,
    /// 用户请求进入后台运行（点击 MainWindow 标题栏播放按钮），语义等同于关闭 MainWindow。
    MainWindowClosed,
    /// EvokeModel 检测到唤醒词（仅 Listening 态可能产生）。
    WakeWordDetected,
    /// DictationModel 异步加载完成。附带 session_id 用于丢弃过期加载。
    DictationModelLoaded { session_id: u64 },
    /// Global Input Monitor 检测到任意键盘/鼠标事件（dismiss 信号）。
    DismissDetected,
    /// Unloading 收尾流程（丢弃内容、卸载模型、写入 History）已完成。附带 session_id。
    CleanupFinished { session_id: u64 },
}
