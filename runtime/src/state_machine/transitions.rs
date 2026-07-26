//! 状态转移逻辑（见 brainstrom/plan.md §4.2）。

use super::event::StateEvent;
use super::state::State;

/// 状态转移结果枚举
#[derive(Debug, Clone, PartialEq)]
pub enum TransitionError {
    /// 当前状态不接受该事件（非法转移组合）
    Invalid(State, StateEvent),
    /// 事件被忽略（如重复 dismiss，过期 session_id 等）
    Ignored,
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
    next_session_id: u64,
    active_session_id: Option<u64>,
}

impl StateMachine {
    /// 初始状态固定为 `Configure`，Runtime 启动后立即显示 MainWindow。
    pub fn new() -> Self {
        Self {
            current: State::Configure,
            pending_open_main_window: false,
            next_session_id: 1,
            active_session_id: None,
        }
    }

    pub fn current(&self) -> State {
        self.current
    }

    pub(crate) fn active_session_id(&self) -> Option<u64> {
        self.active_session_id
    }

    /// 应用一个事件，返回新状态与副作用列表，或转移错误（非法事件/状态组合）。
    /// 完整转移表见 plan.md §4.2；关键分支：
    ///   - Listening --WakeWordDetected--> Loading
    ///   - Loading --DictationModelLoaded--> Dictating
    ///   - Loading --OpenMainWindowRequested--> Unloading（中断加载）
    ///   - Dictating --DismissDetected | OpenMainWindowRequested--> Unloading
    ///   - Unloading --CleanupFinished--> Listening（若 pending_open_main_window，继续 --> Configure）
    ///   - Listening <--> Configure（OpenMainWindowRequested / MainWindowClosed，双向）
    pub fn apply(&mut self, event: StateEvent) -> Result<Transition, TransitionError> {
        if let Some(session_id) = event.session_id() {
            if self.active_session_id != Some(session_id) {
                return Err(TransitionError::Ignored);
            }
        }

        let (new_state, effects) = match (self.current, &event) {
            (State::Listening, StateEvent::OpenMainWindowRequested) => (
                State::Configure,
                vec![
                    TransitionEffect::StopEvokeModel,
                    TransitionEffect::HideHudWindow,
                    TransitionEffect::ShowMainWindow,
                ],
            ),
            (State::Configure, StateEvent::MainWindowClosed) => (
                State::Listening,
                vec![
                    TransitionEffect::HideMainWindow,
                    TransitionEffect::ShowHudWindow,
                    TransitionEffect::StartEvokeModel,
                ],
            ),
            (State::Listening, StateEvent::WakeWordDetected) => {
                let session_id = self.next_session_id;
                self.next_session_id = self.next_session_id.wrapping_add(1);
                if self.next_session_id == 0 {
                    self.next_session_id = 1;
                }
                self.active_session_id = Some(session_id);
                (
                    State::Loading,
                    vec![
                        TransitionEffect::StopEvokeModel,
                        TransitionEffect::StartLoadingDictationModel,
                        TransitionEffect::StartAudioBuffering,
                        TransitionEffect::StartGlobalInputMonitor,
                    ],
                )
            }
            (State::Loading, StateEvent::DictationModelLoaded { .. }) => (
                State::Dictating,
                vec![TransitionEffect::DrainRingBufferIntoDictationModel],
            ),
            (
                State::Loading | State::Dictating,
                StateEvent::DismissDetected | StateEvent::OpenMainWindowRequested,
            ) => {
                self.pending_open_main_window =
                    matches!(&event, StateEvent::OpenMainWindowRequested);
                (
                    State::Unloading,
                    vec![
                        TransitionEffect::StopGlobalInputMonitor,
                        TransitionEffect::StopDictationModelAndDiscardPending,
                        TransitionEffect::UnloadDictationModel,
                        TransitionEffect::WriteHistoryEntry,
                    ],
                )
            }
            (State::Unloading, StateEvent::OpenMainWindowRequested) => {
                self.pending_open_main_window = true;
                return Err(TransitionError::Ignored);
            }
            (State::Unloading, StateEvent::CleanupFinished { .. }) => {
                self.active_session_id = None;
                if self.pending_open_main_window {
                    self.pending_open_main_window = false;
                    (
                        State::Configure,
                        vec![
                            TransitionEffect::HideHudWindow,
                            TransitionEffect::ShowMainWindow,
                        ],
                    )
                } else {
                    (State::Listening, vec![TransitionEffect::StartEvokeModel])
                }
            }
            (State::Configure, StateEvent::OpenMainWindowRequested)
            | (State::Listening, StateEvent::MainWindowClosed)
            | (State::Listening | State::Configure, StateEvent::DismissDetected)
            | (
                State::Loading | State::Dictating | State::Unloading,
                StateEvent::WakeWordDetected,
            )
            | (State::Dictating | State::Unloading, StateEvent::DictationModelLoaded { .. })
            | (State::Unloading, StateEvent::DismissDetected) => {
                return Err(TransitionError::Ignored);
            }
            _ => return Err(TransitionError::Invalid(self.current, event)),
        };

        self.current = new_state;
        Ok(Transition { new_state, effects })
    }
}

impl Default for StateMachine {
    fn default() -> Self {
        Self::new()
    }
}

impl StateEvent {
    fn session_id(&self) -> Option<u64> {
        match self {
            Self::DictationModelLoaded { session_id } | Self::CleanupFinished { session_id } => {
                Some(*session_id)
            }
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wake(machine: &mut StateMachine) -> u64 {
        if machine.current() == State::Configure {
            machine.apply(StateEvent::MainWindowClosed).unwrap();
        }
        let transition = machine.apply(StateEvent::WakeWordDetected).unwrap();
        assert_eq!(transition.new_state, State::Loading);
        assert_eq!(
            transition.effects,
            vec![
                TransitionEffect::StopEvokeModel,
                TransitionEffect::StartLoadingDictationModel,
                TransitionEffect::StartAudioBuffering,
                TransitionEffect::StartGlobalInputMonitor,
            ]
        );
        machine.active_session_id().unwrap()
    }

    #[test]
    fn completes_a_dictation_cycle_with_exact_effects() {
        let mut machine = StateMachine::new();
        let session_id = wake(&mut machine);

        let loaded = machine
            .apply(StateEvent::DictationModelLoaded { session_id })
            .unwrap();
        assert_eq!(loaded.new_state, State::Dictating);
        assert_eq!(
            loaded.effects,
            vec![TransitionEffect::DrainRingBufferIntoDictationModel]
        );

        let unloading = machine.apply(StateEvent::DismissDetected).unwrap();
        assert_eq!(unloading.new_state, State::Unloading);
        assert_eq!(
            unloading.effects,
            vec![
                TransitionEffect::StopGlobalInputMonitor,
                TransitionEffect::StopDictationModelAndDiscardPending,
                TransitionEffect::UnloadDictationModel,
                TransitionEffect::WriteHistoryEntry,
            ]
        );

        let listening = machine
            .apply(StateEvent::CleanupFinished { session_id })
            .unwrap();
        assert_eq!(listening.new_state, State::Listening);
        assert_eq!(listening.effects, vec![TransitionEffect::StartEvokeModel]);
    }

    #[test]
    fn preserves_an_open_request_until_cleanup_finishes() {
        let mut machine = StateMachine::new();
        let session_id = wake(&mut machine);
        machine.apply(StateEvent::DismissDetected).unwrap();

        assert_eq!(
            machine.apply(StateEvent::OpenMainWindowRequested),
            Err(TransitionError::Ignored)
        );
        let configure = machine
            .apply(StateEvent::CleanupFinished { session_id })
            .unwrap();
        assert_eq!(configure.new_state, State::Configure);
        assert_eq!(
            configure.effects,
            vec![
                TransitionEffect::HideHudWindow,
                TransitionEffect::ShowMainWindow,
            ]
        );
    }

    #[test]
    fn ignores_stale_session_and_repeated_external_events() {
        let mut machine = StateMachine::new();
        let session_id = wake(&mut machine);
        assert_eq!(
            machine.apply(StateEvent::DictationModelLoaded {
                session_id: session_id + 1,
            }),
            Err(TransitionError::Ignored)
        );
        assert_eq!(machine.current(), State::Loading);

        machine
            .apply(StateEvent::DictationModelLoaded { session_id })
            .unwrap();
        assert_eq!(
            machine.apply(StateEvent::DictationModelLoaded { session_id }),
            Err(TransitionError::Ignored)
        );
        machine.apply(StateEvent::DismissDetected).unwrap();
        assert_eq!(
            machine.apply(StateEvent::DismissDetected),
            Err(TransitionError::Ignored)
        );
        machine
            .apply(StateEvent::CleanupFinished { session_id })
            .unwrap();
        assert_eq!(
            machine.apply(StateEvent::CleanupFinished { session_id }),
            Err(TransitionError::Ignored)
        );
    }

    #[test]
    fn switches_between_listening_and_configure() {
        let mut machine = StateMachine::new();
        assert_eq!(machine.current(), State::Configure);
        assert_eq!(
            machine.apply(StateEvent::OpenMainWindowRequested),
            Err(TransitionError::Ignored)
        );
        let listening = machine.apply(StateEvent::MainWindowClosed).unwrap();
        assert_eq!(listening.new_state, State::Listening);
        assert_eq!(
            machine.apply(StateEvent::MainWindowClosed),
            Err(TransitionError::Ignored)
        );
        let configure = machine.apply(StateEvent::OpenMainWindowRequested).unwrap();
        assert_eq!(configure.new_state, State::Configure);
    }
}
