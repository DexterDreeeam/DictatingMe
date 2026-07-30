//! Runtime：DM 的核心，持有并编排所有模块（见 brainstrom/plan.md §3.3、§5.1 数据流）。

use std::fs::{self, File};
use std::io::{self, BufWriter, Cursor};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use tauri::AppHandle;
use tokio::sync::{mpsc, oneshot, watch};
use tokio::task::JoinHandle;
use uuid::Uuid;

use crate::audio::{
    AudioBus, AudioCapture, AudioDeviceInfo, AudioDeviceProvider, AudioFrame, AudioRingBuffer,
    CpalAudioCapture,
};
use crate::events::{emit_evoke_score, emit_history_updated, emit_input_level, emit_state_changed};
use crate::input_monitor::GlobalInputMonitor;
use crate::models::{DictationModelEngine, EvokeModelEngine};
use crate::scoring::ScoringSystem;
use crate::settings::{RuntimeBundle, SettingsHandle};
use crate::state_machine::{State, StateEvent, StateMachine, TransitionEffect, TransitionError};
use crate::storage::{AppConfig, ConfigStore, Database, HistoryEntry, HistoryStore};
use crate::text::{TextDiffEngine, TextInjector};
use crate::tray::TrayManager;
use crate::window::WindowManager;

const AUDIO_QUEUE_CAPACITY: usize = 64;
const UNLOADING_AUDIO_DRAIN_LIMIT: usize = AUDIO_QUEUE_CAPACITY;
const AUDIO_DROP_LOG_INTERVAL_MS: u64 = 5_000;
const RING_BUFFER_CAPACITY_MS: u64 = 30_000;
const INPUT_LEVEL_INTERVAL: Duration = Duration::from_millis(50);
const WAKE_CUE_WAV: &[u8] = include_bytes!("../assets/sounds/wake.wav");
const END_CUE_WAV: &[u8] = include_bytes!("../assets/sounds/end.wav");

/// Runtime 级别错误。
#[derive(Debug, Clone, PartialEq)]
pub struct RuntimeError(pub String);

impl std::fmt::Display for RuntimeError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for RuntimeError {}

/// 供 Tauri 命令调用的句柄（可安全克隆并跨线程调用）。
/// 封装了对 `RuntimeCore` Actor 的通道和方法。
#[derive(Clone)]
pub struct RuntimeHandle {
    sender: mpsc::UnboundedSender<RuntimeMessage>,
}

impl RuntimeHandle {
    pub async fn get_state(&self) -> Result<State, RuntimeError> {
        self.request(RuntimeMessage::GetState).await
    }

    pub async fn get_config(&self) -> Result<AppConfig, RuntimeError> {
        self.request(RuntimeMessage::GetConfig).await
    }

    pub async fn get_history(&self) -> Result<Vec<HistoryEntry>, RuntimeError> {
        self.request(RuntimeMessage::GetHistory).await
    }

    pub(crate) async fn list_devices(&self) -> Result<Vec<AudioDeviceInfo>, RuntimeError> {
        self.request(RuntimeMessage::ListDevices).await
    }

    pub(crate) async fn set_input_device(
        &self,
        device_id: String,
    ) -> Result<AppConfig, RuntimeError> {
        self.request(|reply| RuntimeMessage::SetInputDevice { device_id, reply })
            .await
    }

    pub(crate) async fn get_history_entry(&self, id: String) -> Result<HistoryEntry, RuntimeError> {
        self.request(|reply| RuntimeMessage::GetHistoryEntry { id, reply })
            .await
    }

    pub(crate) async fn handle_event(&self, event: StateEvent) -> Result<State, RuntimeError> {
        self.request(|reply| RuntimeMessage::StateEvent {
            event,
            reply: Some(reply),
        })
        .await
    }

    pub(crate) fn notify_event(&self, event: StateEvent, context: &'static str) {
        if self
            .sender
            .send(RuntimeMessage::StateEvent { event, reply: None })
            .is_err()
        {
            tracing::error!(context, "runtime callback failed: actor is unavailable");
        }
    }

    pub(crate) async fn shutdown(&self) -> Result<(), RuntimeError> {
        self.request(|reply| RuntimeMessage::Shutdown { reply: Some(reply) })
            .await
    }

    pub(crate) async fn frontend_ready(&self) -> Result<(), RuntimeError> {
        self.request(|reply| RuntimeMessage::ActivateInitialWindow { reply })
            .await
    }

    pub(crate) fn spawn(
        mut core: RuntimeCore,
        app: AppHandle,
        recordings_dir: PathBuf,
        audio_bus: AudioBus,
    ) -> Result<Self, RuntimeError> {
        tracing::info!(
            recordings_directory = %recordings_dir.display(),
            "RuntimeHandle spawn begin"
        );
        let (sender, receiver) = mpsc::unbounded_channel();
        let (audio_sender, audio_receiver) = mpsc::channel(AUDIO_QUEUE_CAPACITY);
        let audio_drop_reporter = Arc::new(AudioDropReporter::default());

        core.sender = Some(sender.clone());
        core.app = Some(app);
        core.recordings_dir = recordings_dir;
        core.install_audio_callback(audio_sender, audio_drop_reporter, audio_bus);
        if let Err(error) = core.start() {
            tracing::error!(%error, "RuntimeHandle spawn error");
            return Err(error);
        }

        tauri::async_runtime::spawn(core.run(receiver, audio_receiver));
        tracing::info!("RuntimeHandle spawn success");
        Ok(Self { sender })
    }

    async fn request<T, F>(&self, make_message: F) -> Result<T, RuntimeError>
    where
        T: Send + 'static,
        F: FnOnce(oneshot::Sender<Result<T, RuntimeError>>) -> RuntimeMessage,
    {
        let (reply, receiver) = oneshot::channel();
        self.sender.send(make_message(reply)).map_err(|_| {
            tracing::error!("runtime request failed: actor is unavailable");
            RuntimeError("runtime actor is unavailable".to_owned())
        })?;
        receiver.await.map_err(|_| {
            tracing::error!("runtime request failed: actor stopped before replying");
            RuntimeError("runtime actor stopped before replying".to_owned())
        })?
    }
}

#[derive(Default)]
struct AudioDropReporter {
    last_report_ms: AtomicU64,
    dropped_since_report: AtomicU64,
}

impl AudioDropReporter {
    fn report(&self, reason: &'static str) {
        self.dropped_since_report.fetch_add(1, Ordering::Relaxed);
        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_millis().min(u128::from(u64::MAX)) as u64)
            .unwrap_or_default();
        let last_report_ms = self.last_report_ms.load(Ordering::Relaxed);
        if last_report_ms != 0 && now_ms.saturating_sub(last_report_ms) < AUDIO_DROP_LOG_INTERVAL_MS
        {
            return;
        }
        if self
            .last_report_ms
            .compare_exchange(
                last_report_ms,
                now_ms.max(1),
                Ordering::Relaxed,
                Ordering::Relaxed,
            )
            .is_ok()
        {
            let dropped_frames = self.dropped_since_report.swap(0, Ordering::Relaxed);
            tracing::warn!(
                reason,
                dropped_frames,
                rate_limit_ms = AUDIO_DROP_LOG_INTERVAL_MS,
                "audio queue dropped frames"
            );
        }
    }
}

enum RuntimeMessage {
    GetState(oneshot::Sender<Result<State, RuntimeError>>),
    GetConfig(oneshot::Sender<Result<AppConfig, RuntimeError>>),
    GetHistory(oneshot::Sender<Result<Vec<HistoryEntry>, RuntimeError>>),
    ListDevices(oneshot::Sender<Result<Vec<AudioDeviceInfo>, RuntimeError>>),
    SetInputDevice {
        device_id: String,
        reply: oneshot::Sender<Result<AppConfig, RuntimeError>>,
    },
    GetHistoryEntry {
        id: String,
        reply: oneshot::Sender<Result<HistoryEntry, RuntimeError>>,
    },
    StateEvent {
        event: StateEvent,
        reply: Option<oneshot::Sender<Result<State, RuntimeError>>>,
    },
    DictationModelLoaded {
        session_id: u64,
        result: Result<DictationModelEngine, RuntimeError>,
    },
    ActivateInitialWindow {
        reply: oneshot::Sender<Result<(), RuntimeError>>,
    },
    Shutdown {
        reply: Option<oneshot::Sender<Result<(), RuntimeError>>>,
    },
}

struct RuntimeLifecycle {
    started: bool,
    shutting_down: bool,
    surfaces_activated: bool,
    model_load: Option<ModelLoadTask>,
}

struct ModelLoadTask {
    handle: JoinHandle<()>,
    cancelled: Arc<AtomicBool>,
}

impl ModelLoadTask {
    fn cancel(self) {
        self.cancelled.store(true, Ordering::Release);
        self.handle.abort();
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RuntimeCue {
    Wake,
    End,
}

struct CuePlayer {
    output: rodio::MixerDeviceSink,
}

impl CuePlayer {
    fn new() -> Result<Self, RuntimeError> {
        let mut output = rodio::DeviceSinkBuilder::open_default_sink()
            .map_err(|error| RuntimeError(format!("failed to open cue output device: {error}")))?;
        output.log_on_drop(false);
        Ok(Self { output })
    }

    fn play(&self, cue: RuntimeCue) -> Result<(), RuntimeError> {
        let bytes = match cue {
            RuntimeCue::Wake => WAKE_CUE_WAV,
            RuntimeCue::End => END_CUE_WAV,
        };
        let source = rodio::Decoder::try_from(Cursor::new(bytes)).map_err(|error| {
            RuntimeError(format!("failed to decode embedded {cue:?} cue: {error}"))
        })?;
        self.output.mixer().add(source);
        Ok(())
    }
}

impl Default for RuntimeLifecycle {
    fn default() -> Self {
        Self {
            started: false,
            shutting_down: false,
            surfaces_activated: false,
            model_load: None,
        }
    }
}

/// RuntimeCore：编排 state_machine / audio / models / text / input_monitor / storage / tray / window
/// 各模块，作为后台 Actor 运行。
pub struct RuntimeCore {
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

    sender: Option<mpsc::UnboundedSender<RuntimeMessage>>,
    app: Option<AppHandle>,
    recordings_dir: PathBuf,
    recording: Option<RecordingSession>,
    lifecycle: RuntimeLifecycle,
    last_input_level_emit: Option<Instant>,
    smoothed_input_level: f32,
    active_input_device_id: String,
    cue_player: Option<CuePlayer>,
    settings: SettingsHandle,
    scoring: ScoringSystem,
    bundle_generation: u64,
    bundle_dirty: bool,
    generation_rx: watch::Receiver<u64>,
    last_score_emit: Option<Instant>,
    last_score_log: Option<Instant>,
}

impl RuntimeCore {
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
        settings: SettingsHandle,
        bundle: RuntimeBundle,
    ) -> Result<Self, RuntimeError> {
        let generation_rx = settings.subscribe_generation();
        let scoring = ScoringSystem::new(
            bundle.profile.clone(),
            bundle.config.sensitivity,
            bundle.speaker_model_path.as_deref(),
        )
        .map_err(RuntimeError)?;
        Ok(Self {
            state_machine: StateMachine::new(),
            audio_capture,
            ring_buffer: AudioRingBuffer::new(RING_BUFFER_CAPACITY_MS),
            evoke_model,
            dictation_model,
            text_diff: TextDiffEngine::new(),
            text_injector,
            input_monitor,
            history_store: HistoryStore::new(Arc::clone(&db)),
            config_store: ConfigStore::new(Arc::clone(&db)),
            db,
            tray_manager,
            window_manager,
            sender: None,
            app: None,
            recordings_dir: PathBuf::new(),
            recording: None,
            lifecycle: RuntimeLifecycle::default(),
            last_input_level_emit: None,
            smoothed_input_level: 0.0,
            active_input_device_id: String::new(),
            cue_player: None,
            settings,
            scoring,
            bundle_generation: bundle.generation,
            bundle_dirty: false,
            generation_rx,
            last_score_emit: None,
            last_score_log: None,
        })
    }

    /// 启动 Runtime：创建托盘、迁移数据库、进入 `State::Configure` 并立即显示 MainWindow。
    pub fn start(&mut self) -> Result<(), RuntimeError> {
        tracing::info!("RuntimeCore start begin");
        if self.lifecycle.started {
            tracing::error!("RuntimeCore start rejected: already started");
            return Err(RuntimeError("runtime has already been started".to_owned()));
        }
        self.sender()?;
        self.app()?;

        let result = self.start_components();
        if let Err(error) = result {
            tracing::error!(%error, "RuntimeCore component startup error");
            self.input_monitor.stop();
            self.audio_capture.stop();
            self.tray_manager.destroy();
            return Err(error);
        }
        self.lifecycle.started = true;
        tracing::info!("RuntimeCore start success");
        Ok(())
    }

    /// 统一事件入口：所有外部输入（唤醒词检测、dismiss、托盘点击等）都通过此方法驱动状态机，
    /// 并据此执行状态机计算出的副作用（加载/卸载模型、显示/隐藏窗口等，见 plan.md §5.1）。
    pub fn handle_event(&mut self, event: StateEvent) -> Result<State, RuntimeError> {
        if event == StateEvent::MainWindowClosed && self.current_state() == State::Configure {
            self.bundle_dirty = true;
            self.refresh_runtime_bundle()?;
            self.evoke_model.reset();
            self.scoring.reset();
        }
        let from = self.state_machine.current();
        let event_session_id = event_session_id(&event);
        let mut session_id = event_session_id.or(self.state_machine.active_session_id());
        tracing::info!(?from, ?event, ?session_id, "state event received");
        let transition = match self.state_machine.apply(event) {
            Ok(transition) => transition,
            Err(TransitionError::Ignored) => {
                tracing::warn!(
                    ?from,
                    ?session_id,
                    "state event ignored (duplicate or stale session)"
                );
                return Ok(self.state_machine.current());
            }
            Err(TransitionError::Invalid(state, event)) => {
                let error = RuntimeError(format!(
                    "invalid runtime transition from {state:?} for event {event:?}"
                ));
                tracing::error!(
                    ?state,
                    ?event,
                    ?session_id,
                    error = %error,
                    "invalid state event"
                );
                return Err(error);
            }
        };
        session_id = event_session_id.or(self.state_machine.active_session_id());
        tracing::info!(
            ?from,
            target = ?transition.new_state,
            ?session_id,
            effects = ?transition.effects,
            "state transition accepted"
        );
        self.play_transition_cue(from, transition.new_state);

        let mut error = None;
        for effect in transition.effects {
            let effect_name = format!("{effect:?}");
            tracing::info!(
                effect = %effect_name,
                state = ?transition.new_state,
                ?session_id,
                "transition effect begin"
            );
            match self.execute_effect(effect) {
                Ok(()) => tracing::info!(
                    effect = %effect_name,
                    state = ?transition.new_state,
                    ?session_id,
                    "transition effect success"
                ),
                Err(effect_error) => {
                    tracing::error!(
                        effect = %effect_name,
                        state = ?transition.new_state,
                        ?session_id,
                        error = %effect_error,
                        "transition effect error"
                    );
                    record_first_error(&mut error, effect_error);
                }
            }
        }
        if let Err(light_error) = self
            .window_manager
            .set_hud_light(transition.new_state.hud_light())
            .map_err(|error| RuntimeError(format!("failed to update HUD light: {}", error.0)))
        {
            tracing::error!(error = %light_error, "HUD light effect error");
            record_first_error(&mut error, light_error);
        }
        if let Err(emit_error) = self.emit_state(transition.new_state) {
            tracing::error!(error = %emit_error, "state event emission error");
            record_first_error(&mut error, emit_error);
        }

        if transition.new_state == State::Unloading {
            if let Some(session_id) = self.state_machine.active_session_id() {
                self.send_internal(
                    RuntimeMessage::StateEvent {
                        event: StateEvent::CleanupFinished { session_id },
                        reply: None,
                    },
                    "enqueue cleanup completion",
                )?;
            } else {
                record_first_error(
                    &mut error,
                    RuntimeError("Unloading state has no active session id".to_owned()),
                );
            }
        } else if error.is_some()
            && matches!(transition.new_state, State::Loading | State::Dictating)
        {
            if let Err(recovery_error) = self.send_internal(
                RuntimeMessage::StateEvent {
                    event: StateEvent::DismissDetected,
                    reply: None,
                },
                "enqueue transition recovery",
            ) {
                record_first_error(&mut error, recovery_error);
            }
        }

        match error {
            Some(error) => {
                tracing::error!(
                    ?from,
                    target = ?transition.new_state,
                    ?session_id,
                    %error,
                    "state transition completed with error"
                );
                Err(error)
            }
            None => {
                tracing::info!(
                    ?from,
                    target = ?transition.new_state,
                    ?session_id,
                    "state transition complete"
                );
                Ok(transition.new_state)
            }
        }
    }

    pub fn current_state(&self) -> State {
        self.state_machine.current()
    }

    pub fn history(&self) -> &HistoryStore {
        &self.history_store
    }

    pub fn config(&self) -> &ConfigStore {
        &self.config_store
    }

    /// 退出：由系统托盘菜单"退出"或 MainWindow 标题栏电源按钮触发，两者走同一路径，
    /// 无需二次确认，直接终止整个 Runtime 进程（见 plan.md §3.3、§6）。
    pub fn shutdown(&mut self) {
        if self.lifecycle.shutting_down {
            tracing::debug!("RuntimeCore shutdown ignored: already shutting down");
            return;
        }
        tracing::info!(state = ?self.current_state(), "RuntimeCore shutdown begin");
        self.lifecycle.shutting_down = true;
        if let Some(load) = self.lifecycle.model_load.take() {
            tracing::info!("aborting in-flight dictation model load during shutdown");
            load.cancel();
        }
        self.input_monitor.stop();
        self.audio_capture.stop();
        self.dictation_model.unload();
        self.ring_buffer.clear();
        self.text_diff.reset();
        if let Some(recording) = self.recording.take() {
            if let Err(error) = recording.discard() {
                tracing::error!(%error, "shutdown failed to discard incomplete recording");
            }
        }
        self.tray_manager.destroy();
        if let Some(app) = &self.app {
            tracing::info!("RuntimeCore shutdown complete; exiting Tauri");
            app.exit(0);
        } else {
            tracing::error!("shutdown could not exit Tauri: app handle is unavailable");
        }
    }

    fn start_components(&mut self) -> Result<(), RuntimeError> {
        tracing::debug!("component startup phase: migrate database");
        self.db.migrate().map_err(|error| RuntimeError(error.0))?;
        tracing::debug!("component startup phase: prepare recordings directory");
        fs::create_dir_all(&self.recordings_dir).map_err(|error| {
            RuntimeError(format!(
                "failed to create recordings directory '{}': {error}",
                self.recordings_dir.display()
            ))
        })?;

        tracing::debug!("component startup phase: initialize runtime cue output");
        match CuePlayer::new() {
            Ok(player) => {
                self.cue_player = Some(player);
                tracing::info!("runtime cue output initialized");
            }
            Err(error) => {
                self.cue_player = None;
                tracing::warn!(%error, "runtime cue output unavailable");
            }
        }

        tracing::debug!("component startup phase: apply persisted configuration");
        let mut config = self
            .config_store
            .load()
            .map_err(|error| RuntimeError(error.0))?;
        if self.evoke_model.active_word() != config.evoke_word {
            self.evoke_model
                .set_active_word(config.evoke_word.clone())
                .map_err(|error| RuntimeError(error.0))?;
        }
        self.evoke_model.set_sensitivity(config.sensitivity);

        tracing::debug!("component startup phase: enumerate and start audio input");
        let devices = enumerate_audio_devices()?;
        let device = select_startup_device(&devices, &config.input_device_id)?;
        tracing::info!(
            device_name = %device.name,
            device_id = %device.id,
            is_default = device.is_default,
            "selected startup audio input device"
        );
        self.audio_capture
            .start(device)
            .map_err(|error| RuntimeError(format!("failed to start audio capture: {}", error.0)))?;
        self.active_input_device_id.clone_from(&device.id);
        if config.input_device_id != device.id {
            config.input_device_id.clone_from(&device.id);
            if let Err(error) = self.config_store.save(&config) {
                self.audio_capture.stop();
                return Err(RuntimeError(format!(
                    "audio capture started but selected device could not be persisted: {}",
                    error.0
                )));
            }
            tracing::info!(
                device_id = %device.id,
                "persisted automatically selected input device"
            );
        }

        Ok(())
    }

    fn activate_initial_window(&mut self) -> Result<(), RuntimeError> {
        if self.lifecycle.surfaces_activated {
            return Ok(());
        }
        tracing::debug!("user surface activation: create tray callbacks");
        let open_sender = self.sender()?.clone();
        let exit_sender = self.sender()?.clone();
        self.tray_manager
            .create(
                Box::new(move || {
                    if open_sender
                        .send(RuntimeMessage::StateEvent {
                            event: StateEvent::OpenMainWindowRequested,
                            reply: None,
                        })
                        .is_err()
                    {
                        tracing::error!("tray open callback failed: runtime actor is unavailable");
                    }
                }),
                Box::new(move || {
                    if exit_sender
                        .send(RuntimeMessage::Shutdown { reply: None })
                        .is_err()
                    {
                        tracing::error!("tray exit callback failed: runtime actor is unavailable");
                    }
                }),
            )
            .map_err(|error| RuntimeError(format!("failed to create tray icon: {}", error.0)))?;

        tracing::debug!("user surface activation: apply initial window state");
        let state = self.state_machine.current();
        self.window_manager
            .apply_visible_window(state.visible_window())
            .map_err(|error| {
                RuntimeError(format!(
                    "failed to show initial runtime window: {}",
                    error.0
                ))
            })?;
        self.window_manager
            .set_hud_light(state.hud_light())
            .map_err(|error| {
                RuntimeError(format!("failed to set initial HUD light: {}", error.0))
            })?;
        self.emit_state(state)?;
        self.lifecycle.surfaces_activated = true;
        Ok(())
    }

    fn install_audio_callback(
        &mut self,
        sender: mpsc::Sender<AudioFrame>,
        drop_reporter: Arc<AudioDropReporter>,
        audio_bus: AudioBus,
    ) {
        self.audio_capture
            .set_frame_callback(Box::new(move |frame| {
                audio_bus.publish(frame.clone());
                match sender.try_send(frame) {
                    Ok(()) => {}
                    Err(mpsc::error::TrySendError::Full(_)) => {
                        drop_reporter.report("queue full");
                    }
                    Err(mpsc::error::TrySendError::Closed(_)) => {
                        drop_reporter.report("queue closed");
                    }
                }
            }));
    }

    fn play_transition_cue(&self, from: State, target: State) {
        let Some(cue) = transition_cue(from, target) else {
            return;
        };
        match &self.cue_player {
            Some(player) => match player.play(cue) {
                Ok(()) => tracing::debug!(?cue, ?from, ?target, "runtime cue queued"),
                Err(error) => {
                    tracing::error!(?cue, ?from, ?target, %error, "runtime cue decode failed")
                }
            },
            None => {
                tracing::warn!(
                    ?cue,
                    ?from,
                    ?target,
                    "runtime cue skipped: output unavailable"
                )
            }
        }
    }

    async fn run(
        mut self,
        mut receiver: mpsc::UnboundedReceiver<RuntimeMessage>,
        mut audio_receiver: mpsc::Receiver<AudioFrame>,
    ) {
        tracing::info!("runtime actor started");
        while !self.lifecycle.shutting_down {
            tokio::select! {
                biased;
                message = receiver.recv() => {
                    let Some(message) = message else {
                        break;
                    };
                    let state_before_message = self.current_state();
                    self.process_message(message);
                    if state_before_message == State::Unloading
                        || self.current_state() == State::Unloading
                    {
                        let drained = drain_audio_frames_bounded(
                            &mut audio_receiver,
                            UNLOADING_AUDIO_DRAIN_LIMIT,
                        );
                        if drained > 0 {
                            tracing::debug!(
                                drained,
                                limit = UNLOADING_AUDIO_DRAIN_LIMIT,
                                "bounded audio queue drain while unloading"
                            );
                        }
                    }
                }
                frame = audio_receiver.recv() => {
                    let Some(frame) = frame else {
                        break;
                    };
                    if let Err(error) = self.process_audio_frame(frame) {
                        tracing::error!(
                            state = ?self.current_state(),
                            %error,
                            "audio frame processing failed"
                        );
                    }
                }
                changed = self.generation_rx.changed() => {
                    if changed.is_err() {
                        tracing::warn!("settings generation watch closed");
                        continue;
                    }
                    self.bundle_dirty = true;
                    if matches!(self.current_state(), State::Listening | State::Configure) {
                        if let Err(error) = self.refresh_runtime_bundle() {
                            tracing::error!(
                                state = ?self.current_state(),
                                %error,
                                "failed to refresh runtime bundle after settings change"
                            );
                        } else {
                            self.evoke_model.reset();
                            self.scoring.reset();
                            self.last_score_emit = None;
                        }
                    }
                }
            }
        }

        if !self.lifecycle.shutting_down {
            tracing::warn!("runtime actor input channel closed; initiating shutdown");
            self.shutdown();
        }
        tracing::info!("runtime actor stopped");
    }

    fn process_message(&mut self, message: RuntimeMessage) {
        match message {
            RuntimeMessage::GetState(reply) => {
                let _ = reply.send(Ok(self.current_state()));
            }
            RuntimeMessage::GetConfig(reply) => {
                let result = self
                    .config_store
                    .load()
                    .map_err(|error| RuntimeError(error.0));
                let _ = reply.send(result);
            }
            RuntimeMessage::GetHistory(reply) => {
                let result = self
                    .history_store
                    .list()
                    .map_err(|error| RuntimeError(error.0));
                let _ = reply.send(result);
            }
            RuntimeMessage::ListDevices(reply) => {
                let _ = reply.send(enumerate_audio_devices());
            }
            RuntimeMessage::SetInputDevice { device_id, reply } => {
                let result = self.set_input_device(&device_id);
                let _ = reply.send(result);
            }
            RuntimeMessage::GetHistoryEntry { id, reply } => {
                let result = self
                    .history_store
                    .get(&id)
                    .map_err(|error| RuntimeError(error.0))
                    .and_then(|entry| {
                        entry.ok_or_else(|| RuntimeError(format!("history entry not found: {id}")))
                    });
                let _ = reply.send(result);
            }
            RuntimeMessage::StateEvent { event, reply } => {
                let result = self.handle_event(event);
                if let Some(reply) = reply {
                    let _ = reply.send(result);
                } else if let Err(error) = result {
                    tracing::error!(%error, "runtime event callback failed");
                }
            }
            RuntimeMessage::DictationModelLoaded { session_id, result } => {
                self.finish_model_load(session_id, result);
            }
            RuntimeMessage::ActivateInitialWindow { reply } => {
                let result = self.activate_initial_window();
                if let Err(error) = &result {
                    tracing::error!(%error, "initial user surface activation failed");
                }
                let _ = reply.send(result);
            }
            RuntimeMessage::Shutdown { reply } => {
                self.shutdown();
                if let Some(reply) = reply {
                    let _ = reply.send(Ok(()));
                }
            }
        }
    }

    fn process_audio_frame(&mut self, frame: AudioFrame) -> Result<(), RuntimeError> {
        match self.current_state() {
            State::Listening => {
                let candidate = self.evoke_model.process_frame(&frame).is_some();
                self.scoring.push_frame(&frame, candidate);
                if candidate && self.scoring.evaluate_candidate().accepted {
                    self.handle_event(StateEvent::WakeWordDetected)?;
                    if self.current_state() == State::Loading {
                        self.buffer_audio_frame(frame)?;
                    }
                }
                Ok(())
            }
            State::Loading => {
                let input_result = self.process_input_level(&frame);
                combine_results(self.buffer_audio_frame(frame), input_result)
            }
            State::Dictating => {
                let recording_result = self.write_recording_frame(&frame);
                let dictation_result = self.process_dictation_frame(&frame);
                let input_result = self.process_input_level(&frame);
                combine_results(
                    combine_results(recording_result, dictation_result),
                    input_result,
                )
            }
            State::Configure => self.process_configure_frame(&frame),
            State::Unloading => Ok(()),
        }
    }

    fn process_input_level(&mut self, frame: &AudioFrame) -> Result<(), RuntimeError> {
        let level = normalized_rms(&frame.samples);
        self.smoothed_input_level = if self.last_input_level_emit.is_none() {
            level
        } else {
            self.smoothed_input_level * 0.72 + level * 0.28
        };

        let now = Instant::now();
        if self
            .last_input_level_emit
            .is_some_and(|last| now.duration_since(last) < INPUT_LEVEL_INTERVAL)
        {
            return Ok(());
        }

        self.last_input_level_emit = Some(now);
        emit_input_level(self.app()?, self.smoothed_input_level)
            .map_err(|error| RuntimeError(format!("failed to emit input-level: {error}")))
    }

    fn process_configure_frame(&mut self, frame: &AudioFrame) -> Result<(), RuntimeError> {
        let keyword_hit = self.evoke_model.process_frame(frame).is_some();
        self.scoring.push_frame(frame, keyword_hit);
        let input_result = self.process_input_level(frame);
        let now = Instant::now();
        if !self
            .last_score_emit
            .is_some_and(|last| now.duration_since(last) < Duration::from_millis(100))
        {
            self.last_score_emit = Some(now);
            let score = self.scoring.preview();
            if !self
                .last_score_log
                .is_some_and(|last| now.duration_since(last) < Duration::from_secs(1))
            {
                self.last_score_log = Some(now);
                tracing::debug!(
                    overall = score.overall,
                    threshold = score.threshold,
                    voice_activity = score.voice_activity,
                    phrase_score = score.phrase_score,
                    mode_score = score.mode_score,
                    accepted = score.accepted,
                    mode = ?score.mode,
                    "emitting evoke score preview"
                );
            }
            emit_evoke_score(self.app()?, score)
                .map_err(|error| RuntimeError(format!("failed to emit evoke-score: {error}")))?;
        }
        input_result
    }

    fn buffer_audio_frame(&mut self, frame: AudioFrame) -> Result<(), RuntimeError> {
        let recording_result = self.write_recording_frame(&frame);
        self.ring_buffer.push(frame);
        recording_result
    }

    fn write_recording_frame(&mut self, frame: &AudioFrame) -> Result<(), RuntimeError> {
        self.recording
            .as_mut()
            .ok_or_else(|| RuntimeError("audio arrived without an active recording".to_owned()))?
            .write_frame(frame)
    }

    fn process_dictation_frame(&mut self, frame: &AudioFrame) -> Result<(), RuntimeError> {
        let updates = self
            .dictation_model
            .process_frame(frame)
            .map_err(|error| RuntimeError(format!("dictation recognition failed: {}", error.0)))?;
        for update in updates {
            let suffix = self.text_diff.compute_suffix(&update.full_text);
            if suffix.is_empty() {
                continue;
            }
            self.text_injector.type_text(&suffix).map_err(|error| {
                RuntimeError(format!("failed to inject dictated text: {}", error.0))
            })?;
        }
        Ok(())
    }

    fn execute_effect(&mut self, effect: TransitionEffect) -> Result<(), RuntimeError> {
        match effect {
            TransitionEffect::StartEvokeModel | TransitionEffect::StopEvokeModel => Ok(()),
            TransitionEffect::StartLoadingDictationModel => self.start_model_load(),
            TransitionEffect::StartAudioBuffering => self.begin_recording(),
            TransitionEffect::DrainRingBufferIntoDictationModel => {
                let mut error = None;
                for frame in self.ring_buffer.drain() {
                    if let Err(frame_error) = self.process_dictation_frame(&frame) {
                        record_first_error(&mut error, frame_error);
                    }
                }
                error.map_or(Ok(()), Err)
            }
            TransitionEffect::StopDictationModelAndDiscardPending => {
                self.ring_buffer.clear();
                self.dictation_model.discard_pending();
                Ok(())
            }
            TransitionEffect::UnloadDictationModel => {
                if let Some(load) = self.lifecycle.model_load.take() {
                    load.cancel();
                }
                self.dictation_model.unload();
                Ok(())
            }
            TransitionEffect::WriteHistoryEntry => self.finish_recording(),
            TransitionEffect::ShowMainWindow => self
                .window_manager
                .show_main_window()
                .map_err(|error| RuntimeError(error.0)),
            TransitionEffect::HideMainWindow => self
                .window_manager
                .hide_main_window()
                .map_err(|error| RuntimeError(error.0)),
            TransitionEffect::ShowHudWindow => self
                .window_manager
                .show_hud_window()
                .map_err(|error| RuntimeError(error.0)),
            TransitionEffect::HideHudWindow => self
                .window_manager
                .hide_hud_window()
                .map_err(|error| RuntimeError(error.0)),
            TransitionEffect::StartGlobalInputMonitor => {
                let sender = self.sender()?.clone();
                self.input_monitor
                    .start(Box::new(move |_| {
                        if sender
                            .send(RuntimeMessage::StateEvent {
                                event: StateEvent::DismissDetected,
                                reply: None,
                            })
                            .is_err()
                        {
                            tracing::error!(
                                "input monitor callback failed: runtime actor is unavailable"
                            );
                        }
                    }))
                    .map_err(|error| {
                        RuntimeError(format!("failed to start global input monitor: {}", error.0))
                    })
            }
            TransitionEffect::StopGlobalInputMonitor => {
                self.input_monitor.stop();
                Ok(())
            }
        }
    }

    fn start_model_load(&mut self) -> Result<(), RuntimeError> {
        let session_id = self.state_machine.active_session_id().ok_or_else(|| {
            RuntimeError("model loading started without an active session".into())
        })?;
        if let Some(previous) = self.lifecycle.model_load.take() {
            tracing::warn!(session_id, "aborting previous dictation model load");
            previous.cancel();
        }
        let spec = self.dictation_model.spec().cloned().ok_or_else(|| {
            RuntimeError("model loading started without a dictation model spec".to_owned())
        })?;
        tracing::info!(
            session_id,
            model_id = %spec.id,
            model_path = %spec.root.display(),
            recognizer_type = ?spec.recognizer.recognizer_type(),
            output_mode = ?spec.output_mode(),
            "dictation model load start"
        );
        let sender = self.sender()?.clone();
        let cancelled = Arc::new(AtomicBool::new(false));
        let load_cancelled = Arc::clone(&cancelled);
        let handle = tokio::spawn(async move {
            let mut model = DictationModelEngine::new(Some(spec.clone()));
            let result = model
                .load_with_cancellation(load_cancelled)
                .await
                .map(|()| model)
                .map_err(|error| {
                    RuntimeError(format!("failed to load dictation model: {}", error.0))
                });
            match &result {
                Ok(_) => tracing::info!(
                    session_id,
                    model_id = %spec.id,
                    model_path = %spec.root.display(),
                    "dictation model load finish"
                ),
                Err(error) => tracing::error!(
                    session_id,
                    model_id = %spec.id,
                    model_path = %spec.root.display(),
                    %error,
                    "dictation model load error"
                ),
            }
            if sender
                .send(RuntimeMessage::DictationModelLoaded { session_id, result })
                .is_err()
            {
                tracing::error!(
                    session_id,
                    "model loading callback failed: runtime actor is unavailable"
                );
            }
        });
        self.lifecycle.model_load = Some(ModelLoadTask { handle, cancelled });
        Ok(())
    }

    fn finish_model_load(
        &mut self,
        session_id: u64,
        result: Result<DictationModelEngine, RuntimeError>,
    ) {
        if self.state_machine.active_session_id() != Some(session_id)
            || self.current_state() != State::Loading
        {
            tracing::warn!(
                session_id,
                active_session_id = ?self.state_machine.active_session_id(),
                state = ?self.current_state(),
                "discarding stale dictation model load result"
            );
            return;
        }
        self.lifecycle.model_load.take();

        match result {
            Ok(model) => {
                tracing::info!(session_id, "applying loaded dictation model");
                self.dictation_model = model;
                if let Err(error) =
                    self.handle_event(StateEvent::DictationModelLoaded { session_id })
                {
                    tracing::error!(
                        session_id,
                        %error,
                        "model-loaded event failed"
                    );
                }
            }
            Err(error) => {
                tracing::error!(
                    session_id,
                    %error,
                    "dictation model loading failed"
                );
                if let Err(cleanup_error) = self.handle_event(StateEvent::DismissDetected) {
                    tracing::error!(
                        session_id,
                        error = %cleanup_error,
                        "failed to clean up after model loading failure"
                    );
                }
            }
        }
    }

    fn begin_recording(&mut self) -> Result<(), RuntimeError> {
        if self.recording.is_some() {
            return Err(RuntimeError(
                "cannot begin a recording while another recording is active".to_owned(),
            ));
        }
        fs::create_dir_all(&self.recordings_dir).map_err(|error| {
            RuntimeError(format!(
                "failed to create recordings directory '{}': {error}",
                self.recordings_dir.display()
            ))
        })?;
        self.text_diff.reset();
        self.ring_buffer.clear();
        self.recording = Some(RecordingSession::new(&self.recordings_dir)?);
        Ok(())
    }

    fn finish_recording(&mut self) -> Result<(), RuntimeError> {
        let text = self.text_diff.final_full_text().to_owned();
        let result = match self.recording.take() {
            Some(recording) => recording.finalize(text).and_then(|entry| {
                if let Err(error) = self.history_store.append(entry.clone()) {
                    tracing::error!(
                        history_id = %entry.id,
                        error = %error.0,
                        "history append error"
                    );
                    return Err(match self.history_store.get(&entry.id) {
                        Ok(Some(_)) => {
                            tracing::warn!(
                                history_id = %entry.id,
                                "history entry committed despite append cleanup error"
                            );
                            let emit_result = self.app().and_then(|app| {
                                emit_history_updated(app, entry.clone()).map_err(|emit_error| {
                                    RuntimeError(format!(
                                        "failed to emit history-updated: {emit_error}"
                                    ))
                                })
                            });
                            match emit_result {
                                Ok(()) => RuntimeError(format!(
                                    "history entry was committed but append cleanup failed: {}",
                                    error.0
                                )),
                                Err(emit_error) => RuntimeError(format!(
                                    "history entry was committed but append cleanup failed: {}; {}",
                                    error.0, emit_error.0
                                )),
                            }
                        }
                        Ok(None) => match remove_recording_file(Path::new(&entry.audio_path)) {
                            Ok(()) => RuntimeError(format!(
                                "failed to append history entry: {}",
                                error.0
                            )),
                            Err(cleanup_error) => RuntimeError(format!(
                                "failed to append history entry: {}; additionally failed to remove orphaned recording: {}",
                                error.0, cleanup_error.0
                            )),
                        },
                        Err(lookup_error) => RuntimeError(format!(
                            "history append failed: {}; commit status could not be verified, so the finalized recording was retained: {}",
                            error.0, lookup_error.0
                        )),
                    });
                }
                match self.history_store.list() {
                    Ok(entries) => tracing::info!(
                        history_id = %entry.id,
                        count = entries.len(),
                        "history append success"
                    ),
                    Err(error) => tracing::error!(
                        history_id = %entry.id,
                        error = %error.0,
                        "history append succeeded but count query failed"
                    ),
                }
                emit_history_updated(self.app()?, entry.clone()).map_err(|error| {
                    RuntimeError(format!("failed to emit history-updated: {error}"))
                })?;
                Ok(())
            }),
            None => Err(RuntimeError(
                "cannot write history without an active recording".to_owned(),
            )),
        };
        self.text_diff.reset();
        result
    }

    fn set_input_device(&mut self, device_id: &str) -> Result<AppConfig, RuntimeError> {
        tracing::info!(device_id, "input device config mutation begin");
        let devices = enumerate_audio_devices()?;
        let selected = devices
            .iter()
            .find(|device| device.id == device_id)
            .ok_or_else(|| RuntimeError(format!("audio input device not found: {device_id}")))?
            .clone();
        tracing::info!(
            device_id = %selected.id,
            device_name = %selected.name,
            "selected input device"
        );
        let old_config = self
            .settings
            .config()
            .map_err(|error| RuntimeError(error.0))?;
        let old_device = devices
            .iter()
            .find(|device| device.id == old_config.input_device_id)
            .cloned();

        if let Err(error) = self.audio_capture.start(&selected) {
            let primary = RuntimeError(format!(
                "failed to start selected audio device '{}': {}",
                selected.name, error.0
            ));
            return Err(self.restore_audio_device(old_device.as_ref(), primary));
        }

        let new_config = self
            .settings
            .persist_input_device(&selected.id)
            .map_err(|error| {
                self.restore_audio_device(
                    old_device.as_ref(),
                    RuntimeError(format!(
                        "selected audio device started but config persistence failed: {}",
                        error.0
                    )),
                )
            })?;
        self.active_input_device_id
            .clone_from(&new_config.input_device_id);
        tracing::info!(
            device_id = %new_config.input_device_id,
            "input device config mutation success"
        );
        Ok(new_config)
    }

    fn restore_audio_device(
        &mut self,
        old_device: Option<&AudioDeviceInfo>,
        primary: RuntimeError,
    ) -> RuntimeError {
        match old_device {
            Some(device) => match self.audio_capture.start(device) {
                Ok(()) => {
                    self.active_input_device_id.clone_from(&device.id);
                    tracing::warn!(
                        device_id = %device.id,
                        device_name = %device.name,
                        "restored previous input device after mutation error"
                    );
                    primary
                }
                Err(error) => RuntimeError(format!(
                    "{}; additionally failed to restore previous audio device '{}': {}",
                    primary.0, device.name, error.0
                )),
            },
            None => {
                tracing::warn!("stopping audio capture: no previous device to restore");
                self.audio_capture.stop();
                primary
            }
        }
    }

    fn refresh_runtime_bundle(&mut self) -> Result<(), RuntimeError> {
        if !self.bundle_dirty {
            return Ok(());
        }
        let bundle = self
            .settings
            .runtime_bundle()
            .map_err(|error| RuntimeError(error.0))?;
        if bundle.generation == self.bundle_generation {
            self.bundle_dirty = false;
            return Ok(());
        }
        self.evoke_model
            .set_active_word(bundle.profile.phrase.clone())
            .map_err(|error| RuntimeError(error.0))?;
        self.evoke_model.set_sensitivity(bundle.config.sensitivity);
        self.dictation_model
            .set_spec(bundle.dictation_model)
            .map_err(|error| RuntimeError(error.0))?;
        self.scoring = ScoringSystem::new(
            bundle.profile,
            bundle.config.sensitivity,
            bundle.speaker_model_path.as_deref(),
        )
        .map_err(RuntimeError)?;
        let devices = enumerate_audio_devices()?;
        if let Some(device) = devices
            .iter()
            .find(|device| device.id == bundle.config.input_device_id)
        {
            if self.active_input_device_id != device.id {
                let old_device = devices
                    .iter()
                    .find(|candidate| candidate.id == self.active_input_device_id)
                    .cloned();
                if let Err(error) = self.audio_capture.start(device) {
                    return Err(self.restore_audio_device(
                        old_device.as_ref(),
                        RuntimeError(format!(
                            "failed to apply selected input device: {}",
                            error.0
                        )),
                    ));
                }
                self.active_input_device_id.clone_from(&device.id);
            }
        }
        self.bundle_generation = bundle.generation;
        self.bundle_dirty = false;
        Ok(())
    }

    fn emit_state(&self, state: State) -> Result<(), RuntimeError> {
        emit_state_changed(self.app()?, state)
            .map_err(|error| RuntimeError(format!("failed to emit state-changed: {error}")))
    }

    fn app(&self) -> Result<&AppHandle, RuntimeError> {
        self.app
            .as_ref()
            .ok_or_else(|| RuntimeError("runtime Tauri app handle is not attached".to_owned()))
    }

    fn sender(&self) -> Result<&mpsc::UnboundedSender<RuntimeMessage>, RuntimeError> {
        self.sender
            .as_ref()
            .ok_or_else(|| RuntimeError("runtime actor sender is not attached".to_owned()))
    }

    fn send_internal(
        &self,
        message: RuntimeMessage,
        context: &'static str,
    ) -> Result<(), RuntimeError> {
        self.sender()?
            .send(message)
            .map_err(|_| RuntimeError(format!("failed to {context}: runtime actor is unavailable")))
    }
}

fn event_session_id(event: &StateEvent) -> Option<u64> {
    match event {
        StateEvent::DictationModelLoaded { session_id }
        | StateEvent::CleanupFinished { session_id } => Some(*session_id),
        _ => None,
    }
}

fn drain_audio_frames_bounded(receiver: &mut mpsc::Receiver<AudioFrame>, limit: usize) -> usize {
    let mut drained = 0;
    while drained < limit && receiver.try_recv().is_ok() {
        drained += 1;
    }
    drained
}

fn enumerate_audio_devices() -> Result<Vec<AudioDeviceInfo>, RuntimeError> {
    CpalAudioCapture::new()
        .list_devices()
        .map_err(|error| RuntimeError(error.0))
}

fn transition_cue(from: State, target: State) -> Option<RuntimeCue> {
    match (from, target) {
        (State::Listening, State::Loading) => Some(RuntimeCue::Wake),
        (State::Dictating, State::Unloading) => Some(RuntimeCue::End),
        _ => None,
    }
}

fn normalized_rms(samples: &[i16]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    let mean_square = samples
        .iter()
        .map(|sample| {
            let normalized = f64::from(*sample) / f64::from(i16::MAX);
            normalized * normalized
        })
        .sum::<f64>()
        / samples.len() as f64;
    mean_square.sqrt().clamp(0.0, 1.0) as f32
}

fn select_startup_device<'a>(
    devices: &'a [AudioDeviceInfo],
    persisted_id: &str,
) -> Result<&'a AudioDeviceInfo, RuntimeError> {
    devices
        .iter()
        .find(|device| !persisted_id.is_empty() && device.id == persisted_id)
        .or_else(|| devices.iter().find(|device| device.is_default))
        .or_else(|| devices.first())
        .ok_or_else(|| RuntimeError("no audio input devices are available".to_owned()))
}

fn record_first_error(target: &mut Option<RuntimeError>, error: RuntimeError) {
    if target.is_none() {
        *target = Some(error);
    } else {
        tracing::error!(%error, "additional runtime effect failed");
    }
}

fn combine_results(
    first: Result<(), RuntimeError>,
    second: Result<(), RuntimeError>,
) -> Result<(), RuntimeError> {
    match (first, second) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(error), Ok(())) | (Ok(()), Err(error)) => Err(error),
        (Err(first), Err(second)) => Err(RuntimeError(format!("{}; {}", first.0, second.0))),
    }
}

type RecordingWriter = hound::WavWriter<BufWriter<File>>;

struct RecordingSession {
    id: String,
    timestamp_ms: u64,
    path: PathBuf,
    sample_rate: Option<u32>,
    writer: Option<RecordingWriter>,
    failure: Option<String>,
}

impl RecordingSession {
    fn new(recordings_dir: &Path) -> Result<Self, RuntimeError> {
        let id = Uuid::new_v4().to_string();
        let timestamp_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|error| RuntimeError(format!("system clock is before Unix epoch: {error}")))?
            .as_millis()
            .try_into()
            .map_err(|_| RuntimeError("current timestamp exceeds u64 milliseconds".to_owned()))?;
        Ok(Self {
            path: recordings_dir.join(format!("{id}.wav")),
            id,
            timestamp_ms,
            sample_rate: None,
            writer: None,
            failure: None,
        })
    }

    fn write_frame(&mut self, frame: &AudioFrame) -> Result<(), RuntimeError> {
        if let Some(failure) = &self.failure {
            return Err(RuntimeError(failure.clone()));
        }
        if frame.sample_rate == 0 {
            return Err(self.fail("recording frame has an invalid zero sample rate".to_owned()));
        }

        if self.writer.is_none() {
            let spec = hound::WavSpec {
                channels: 1,
                sample_rate: frame.sample_rate,
                bits_per_sample: 16,
                sample_format: hound::SampleFormat::Int,
            };
            match hound::WavWriter::create(&self.path, spec) {
                Ok(writer) => {
                    self.writer = Some(writer);
                    self.sample_rate = Some(frame.sample_rate);
                }
                Err(error) => {
                    return Err(self.fail(format!(
                        "failed to create recording '{}': {error}",
                        self.path.display()
                    )));
                }
            }
        }
        if self.sample_rate != Some(frame.sample_rate) {
            return Err(self.fail(format!(
                "recording sample rate changed from {} to {}",
                self.sample_rate.unwrap_or_default(),
                frame.sample_rate
            )));
        }

        let write_result = {
            let writer = self.writer.as_mut().expect("writer initialized above");
            frame
                .samples
                .iter()
                .try_for_each(|sample| writer.write_sample(*sample))
        };
        write_result.map_err(|error| {
            self.fail(format!(
                "failed to write recording '{}': {error}",
                self.path.display()
            ))
        })
    }

    fn finalize(mut self, text: String) -> Result<HistoryEntry, RuntimeError> {
        if let Some(failure) = self.failure.take() {
            let cleanup = remove_recording_file(&self.path);
            return Err(match cleanup {
                Ok(()) => RuntimeError(failure),
                Err(error) => RuntimeError(format!("{}; {}", failure, error.0)),
            });
        }
        let Some(writer) = self.writer.take() else {
            let _ = remove_recording_file(&self.path);
            return Err(RuntimeError(
                "recording received no audio frames; history entry was not created".to_owned(),
            ));
        };
        if let Err(error) = writer.finalize() {
            let cleanup = remove_recording_file(&self.path);
            return Err(match cleanup {
                Ok(()) => RuntimeError(format!(
                    "failed to finalize recording '{}': {error}",
                    self.path.display()
                )),
                Err(cleanup_error) => RuntimeError(format!(
                    "failed to finalize recording '{}': {error}; {}",
                    self.path.display(),
                    cleanup_error.0
                )),
            });
        }
        let audio_path = self.path.to_str().ok_or_else(|| {
            let cleanup = remove_recording_file(&self.path);
            match cleanup {
                Ok(()) => RuntimeError("recording path is not valid UTF-8".to_owned()),
                Err(error) => {
                    RuntimeError(format!("recording path is not valid UTF-8; {}", error.0))
                }
            }
        })?;
        Ok(HistoryEntry {
            id: self.id,
            timestamp_ms: self.timestamp_ms,
            text,
            audio_path: audio_path.to_owned(),
        })
    }

    fn discard(mut self) -> Result<(), RuntimeError> {
        self.writer.take();
        remove_recording_file(&self.path)
    }

    fn fail(&mut self, message: String) -> RuntimeError {
        self.writer.take();
        let message = match remove_recording_file(&self.path) {
            Ok(()) => message,
            Err(error) => format!("{message}; {}", error.0),
        };
        self.failure = Some(message.clone());
        RuntimeError(message)
    }
}

fn remove_recording_file(path: &Path) -> Result<(), RuntimeError> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(RuntimeError(format!(
            "failed to remove recording '{}': {error}",
            path.display()
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static RECORDING_ID: AtomicU64 = AtomicU64::new(0);

    fn recording_directory() -> PathBuf {
        let id = RECORDING_ID.fetch_add(1, Ordering::Relaxed);
        std::env::current_dir()
            .unwrap()
            .join("target")
            .join(format!(
                "runtime-recording-test-{}-{id}",
                std::process::id()
            ))
    }

    #[test]
    fn recording_is_lazy_and_finalizes_before_history_use() {
        let directory = recording_directory();
        fs::create_dir_all(&directory).unwrap();
        let mut recording = RecordingSession::new(&directory).unwrap();
        assert!(!recording.path.exists());

        recording
            .write_frame(&AudioFrame {
                samples: vec![1, -2, 3],
                sample_rate: 16_000,
                timestamp_ms: 0,
            })
            .unwrap();
        let entry = recording.finalize("hello".to_owned()).unwrap();
        let reader = hound::WavReader::open(&entry.audio_path).unwrap();
        assert_eq!(reader.spec().channels, 1);
        assert_eq!(reader.spec().sample_rate, 16_000);
        assert_eq!(
            reader
                .into_samples::<i16>()
                .collect::<Result<Vec<_>, _>>()
                .unwrap(),
            vec![1, -2, 3]
        );

        fs::remove_file(entry.audio_path).unwrap();
        fs::remove_dir(directory).unwrap();
    }

    #[test]
    fn failed_recording_removes_incomplete_file() {
        let directory = recording_directory();
        fs::create_dir_all(&directory).unwrap();
        let mut recording = RecordingSession::new(&directory).unwrap();
        recording
            .write_frame(&AudioFrame {
                samples: vec![1],
                sample_rate: 16_000,
                timestamp_ms: 0,
            })
            .unwrap();
        let path = recording.path.clone();
        assert!(recording
            .write_frame(&AudioFrame {
                samples: vec![2],
                sample_rate: 48_000,
                timestamp_ms: 1,
            })
            .is_err());
        assert!(!path.exists());
        assert!(recording.finalize(String::new()).is_err());

        fs::remove_dir(directory).unwrap();
    }

    #[test]
    fn startup_device_prefers_persisted_then_default_then_first() {
        let devices = vec![
            AudioDeviceInfo {
                id: "first".to_owned(),
                name: "First".to_owned(),
                is_default: false,
            },
            AudioDeviceInfo {
                id: "default".to_owned(),
                name: "Default".to_owned(),
                is_default: true,
            },
        ];
        assert_eq!(
            select_startup_device(&devices, "first").unwrap().id,
            "first"
        );
        assert_eq!(
            select_startup_device(&devices, "missing").unwrap().id,
            "default"
        );
        assert_eq!(
            select_startup_device(&devices[..1], "").unwrap().id,
            "first"
        );
    }

    #[test]
    fn input_level_is_normalized_rms() {
        assert_eq!(normalized_rms(&[]), 0.0);
        assert_eq!(normalized_rms(&[0, 0]), 0.0);
        assert!((normalized_rms(&[i16::MAX, i16::MIN]) - 1.0).abs() < 0.0001);
        assert!((normalized_rms(&[16_384, -16_384]) - 0.5).abs() < 0.001);
    }

    #[tokio::test]
    async fn handle_reports_a_closed_actor_channel() {
        let (sender, receiver) = mpsc::unbounded_channel();
        drop(receiver);
        let handle = RuntimeHandle { sender };
        assert_eq!(
            handle.get_state().await,
            Err(RuntimeError("runtime actor is unavailable".to_owned()))
        );
    }

    #[test]
    fn unloading_audio_drain_is_strictly_bounded() {
        let (sender, mut receiver) = mpsc::channel(16);
        for timestamp_ms in 0..10 {
            sender
                .try_send(AudioFrame {
                    samples: vec![1],
                    sample_rate: 16_000,
                    timestamp_ms,
                })
                .unwrap();
        }

        assert_eq!(drain_audio_frames_bounded(&mut receiver, 3), 3);
        let mut remaining = 0;
        while receiver.try_recv().is_ok() {
            remaining += 1;
        }
        assert_eq!(remaining, 7);
    }

    #[test]
    fn runtime_cues_only_cover_wake_and_dictation_exit() {
        assert_eq!(
            transition_cue(State::Listening, State::Loading),
            Some(RuntimeCue::Wake)
        );
        assert_eq!(
            transition_cue(State::Dictating, State::Unloading),
            Some(RuntimeCue::End)
        );
        assert_eq!(transition_cue(State::Loading, State::Unloading), None);
        assert_eq!(transition_cue(State::Unloading, State::Listening), None);
    }

    #[test]
    fn embedded_runtime_cue_files_are_valid_pcm_wav() {
        let wake = hound::WavReader::new(Cursor::new(WAKE_CUE_WAV)).unwrap();
        assert_eq!(wake.spec().sample_rate, 48_000);
        assert_eq!(wake.duration(), 4_320);

        let end = hound::WavReader::new(Cursor::new(END_CUE_WAV)).unwrap();
        assert_eq!(end.spec().sample_rate, 48_000);
        assert_eq!(end.duration(), 4_800);
    }
}
