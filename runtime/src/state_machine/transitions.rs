//! 状态转移逻辑（见 brainstrom/plan.md §4.2）。

use super::event::StateEvent;
use super::state::State;

/// 状态转移错误：当前状态不接受该事件（非法转移组合）。
#[derive(Debug, Clone, PartialEq)]
pub struct InvalidTransition {
    pub from: State,
    pub event: StateEvent,
}

/// Runtime 需要响应执行的副作用意图（由 StateMachine 计算"应该做什么"，
/// 具体执行留给 Runtime 编排各模块，见 plan.md §5.1 数据流）。
#[derive(Debug, Clone, PartialEq)]
pub enum TransitionEffect {
    StartEvokeModel,
    StopEvokeModel,
    /// 触发 DictationModel 异步加载（唯一入口：Evoke 检测到唤醒词之后）
    StartLoadingDictationModel,
    /// 立即开始把麦克风音频写入 Audio Ring Buffer（进入 Loading 时）
    StartAudioBuffering,
    /// Ring Buffer 内容作为第一批数据喂给 DictationModel，随后无缝接实时流（进入 Dictating 时）
    DrainRingBufferIntoDictationModel,
    /// 停止喂音频、丢弃尚未转换完成的内容（dismiss 后立即执行，不等模型收尾）
    StopDictationModelAndDiscardPending,
    UnloadDictationModel,
    /// 写入本次听写的最终文本 + 录音 + 时间戳
    WriteHistoryEntry,
    ShowMainWindow,
    HideMainWindow,
    ShowHudWindow,
    HideHudWindow,
    StartGlobalInputMonitor,
    StopGlobalInputMonitor,
}

/// 一次状态转移的结果：新状态 + Runtime 需要执行的副作用意图列表。
#[derive(Debug, Clone, PartialEq)]
pub struct Transition {
    pub new_state: State,
    pub effects: Vec<TransitionEffect>,
}

/// Runtime 状态机核心：持有当前状态，根据事件计算下一状态与副作用。
/// `Unloading` 是所有非待机状态退出到 `Listening` 的统一出口（见 plan.md §4.2 要点）。
pub struct StateMachine {
    current: State,
    /// 在 Loading/Dictating 期间收到"打开 MainWindow 请求"时置位；
    /// Unloading -> Listening 完成后，若此标记为真，自动继续转入 Configure，
    /// 全程无需用户二次操作（见 plan.md §4.2 要点第 2 条）。
    pending_open_main_window: bool,
}

impl StateMachine {
    /// 初始状态固定为 `Listening`（见 plan.md：`[*] --> Listening`）。
    pub fn new() -> Self {
        todo!()
    }

    pub fn current(&self) -> State {
        todo!()
    }

    /// 应用一个事件，返回新状态与副作用列表，或转移错误（非法事件/状态组合）。
    /// 完整转移表见 plan.md §4.2；关键分支：
    ///   - Listening --WakeWordDetected--> Loading
    ///   - Loading --DictationModelLoaded--> Dictating
    ///   - Loading --OpenMainWindowRequested--> Unloading（中断加载）
    ///   - Dictating --DismissDetected | OpenMainWindowRequested--> Unloading
    ///   - Unloading --CleanupFinished--> Listening（若 pending_open_main_window，继续 --> Configure）
    ///   - Listening <--> Configure（OpenMainWindowRequested / MainWindowClosed，双向）
    pub fn apply(&mut self, event: StateEvent) -> Result<Transition, InvalidTransition> {
        todo!("见上方 doc comment 与 plan.md §4.2 完整转移表")
    }
}

impl Default for StateMachine {
    fn default() -> Self {
        Self::new()
    }
}
