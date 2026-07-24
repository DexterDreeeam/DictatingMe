//! Runtime：DM 的核心，持有并编排所有模块（见 brainstrom/plan.md §3.3、§5.1 数据流）。

use std::sync::Arc;

use crate::audio::{AudioCapture, AudioRingBuffer};
use crate::input_monitor::GlobalInputMonitor;
use crate::models::{DictationModelEngine, EvokeModelEngine};
use crate::state_machine::{State, StateEvent, StateMachine};
use crate::storage::{ConfigStore, Database, HistoryStore};
use crate::text::{TextDiffEngine, TextInjector};
use crate::tray::TrayManager;
use crate::window::WindowManager;

/// Runtime 级别错误。
#[derive(Debug, Clone, PartialEq)]
pub struct RuntimeError(pub String);

/// Runtime：编排 state_machine / audio / models / text / input_monitor / storage / tray / window
/// 各模块，是 `main.rs` 中 Tauri `.manage()` 的被管理状态。
///
/// 字段均为占位类型/trait 对象，具体的平台实现（Windows）在 `crate::platform::windows` 中，
/// 由外部在构造 `Runtime` 时注入（依赖注入，便于未来替换/mock 测试）。
pub struct Runtime {
    state_machine: StateMachine,

    audio_capture: Box<dyn AudioCapture + Send>,
    ring_buffer: AudioRingBuffer,

    evoke_model: EvokeModelEngine,
    dictation_model: DictationModelEngine,

    text_diff: TextDiffEngine,
    text_injector: Box<dyn TextInjector + Send>,

    input_monitor: Box<dyn GlobalInputMonitor + Send>,

    db: Arc<Database>,
    history_store: HistoryStore,
    config_store: ConfigStore,

    tray_manager: Box<dyn TrayManager + Send>,
    window_manager: Box<dyn WindowManager + Send>,
}

impl Runtime {
    /// 依赖注入式构造：由 `main.rs` 在启动时组装具体平台实现后传入。
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        audio_capture: Box<dyn AudioCapture + Send>,
        evoke_model: EvokeModelEngine,
        dictation_model: DictationModelEngine,
        text_injector: Box<dyn TextInjector + Send>,
        input_monitor: Box<dyn GlobalInputMonitor + Send>,
        db: Arc<Database>,
        tray_manager: Box<dyn TrayManager + Send>,
        window_manager: Box<dyn WindowManager + Send>,
    ) -> Self {
        todo!()
    }

    /// 启动 Runtime：创建托盘、迁移数据库、进入 `State::Listening`（见 plan.md §4.2 `[*] --> Listening`）。
    pub fn start(&mut self) -> Result<(), RuntimeError> {
        todo!()
    }

    /// 统一事件入口：所有外部输入（唤醒词检测、dismiss、托盘点击等）都通过此方法驱动状态机，
    /// 并据此执行状态机计算出的副作用（加载/卸载模型、显示/隐藏窗口等，见 plan.md §5.1）。
    pub fn handle_event(&mut self, event: StateEvent) -> Result<State, RuntimeError> {
        todo!()
    }

    pub fn current_state(&self) -> State {
        todo!()
    }

    pub fn history(&self) -> &HistoryStore {
        todo!()
    }

    pub fn config(&self) -> &ConfigStore {
        todo!()
    }

    /// 退出：由系统托盘菜单"退出"或 MainWindow 标题栏电源按钮触发，两者走同一路径，
    /// 无需二次确认，直接终止整个 Runtime 进程（见 plan.md §3.3、§6）。
    pub fn shutdown(&mut self) {
        todo!()
    }
}
