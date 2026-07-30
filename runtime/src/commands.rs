//! Tauri 命令层：前端通过 `invoke()` 调用这些命令（见 brainstrom/plan.md §6 MainWindow UX）。

use crate::audio::AudioDeviceInfo;
use crate::evoke_setup::{EvokeSetupSession, RecordingReceipt, StartEvokeSetup};
use crate::runtime::RuntimeHandle;
use crate::settings::SettingsHandle;
use crate::state_machine::{State as DmState, StateEvent};
use crate::storage::{
    AppConfig, AssetInstallRequest, AssetSummary, HistoryEntry, OperationProgress, SettingsSnapshot,
};

struct OutputStreamBuilder;

impl OutputStreamBuilder {
    // rodio 0.22 renamed this API; keep the requested runtime terminology locally.
    fn open_default_stream() -> Result<rodio::MixerDeviceSink, rodio::DeviceSinkError> {
        rodio::DeviceSinkBuilder::open_default_sink()
    }
}

fn command_begin(command: &'static str) {
    tracing::info!(command, "Tauri command begin");
}

fn command_finish<T>(command: &'static str, result: Result<T, String>) -> Result<T, String> {
    match &result {
        Ok(_) => tracing::info!(command, "Tauri command success"),
        Err(error) => tracing::error!(command, %error, "Tauri command error"),
    }
    result
}

/// 获取当前 Runtime 状态（供 MainWindow 首页状态提示、HudWindow 灯光使用）。
#[tauri::command]
pub async fn get_state(runtime: tauri::State<'_, RuntimeHandle>) -> Result<DmState, String> {
    const COMMAND: &str = "get_state";
    command_begin(COMMAND);
    command_finish(COMMAND, runtime.get_state().await.map_err(|error| error.0))
}

#[tauri::command]
pub async fn frontend_ready(runtime: tauri::State<'_, RuntimeHandle>) -> Result<(), String> {
    command_finish(
        "frontend_ready",
        runtime.frontend_ready().await.map_err(|error| error.0),
    )
}

/// 列出可用输入设备（InputDevice 二级页）。
#[tauri::command]
pub async fn list_devices(
    runtime: tauri::State<'_, RuntimeHandle>,
) -> Result<Vec<AudioDeviceInfo>, String> {
    const COMMAND: &str = "list_devices";
    command_begin(COMMAND);
    command_finish(
        COMMAND,
        runtime.list_devices().await.map_err(|error| error.0),
    )
}

/// 切换输入设备（InputDevice 二级页选中某项）。
#[tauri::command]
pub async fn set_input_device(
    runtime: tauri::State<'_, RuntimeHandle>,
    device_id: String,
) -> Result<AppConfig, String> {
    const COMMAND: &str = "set_input_device";
    tracing::info!(command = COMMAND, %device_id, "Tauri command begin");
    command_finish(
        COMMAND,
        runtime
            .set_input_device(device_id)
            .await
            .map_err(|error| error.0),
    )
}

/// 获取当前配置（含唤醒词、敏感度、输入设备）。
#[tauri::command]
pub async fn get_config(runtime: tauri::State<'_, RuntimeHandle>) -> Result<AppConfig, String> {
    const COMMAND: &str = "get_config";
    command_begin(COMMAND);
    command_finish(COMMAND, runtime.get_config().await.map_err(|error| error.0))
}

#[tauri::command]
pub async fn get_settings_snapshot(
    settings: tauri::State<'_, SettingsHandle>,
) -> Result<SettingsSnapshot, String> {
    command_finish(
        "get_settings_snapshot",
        settings.snapshot().map_err(|error| error.0),
    )
}

#[tauri::command]
pub async fn inspect_asset(
    settings: tauri::State<'_, SettingsHandle>,
    asset_path: String,
) -> Result<AssetSummary, String> {
    command_begin("inspect_asset");
    let settings = settings.inner().clone();
    let summary = tokio::task::spawn_blocking(move || settings.inspect_asset(&asset_path))
        .await
        .map_err(|error| format!("asset verification task failed: {error}"))?;
    command_finish("inspect_asset", Ok(summary))
}

#[tauri::command]
pub async fn install_asset(
    settings: tauri::State<'_, SettingsHandle>,
    asset_link_list: Vec<String>,
    asset_path: String,
) -> Result<String, String> {
    command_finish(
        "install_asset",
        settings
            .install_asset(AssetInstallRequest {
                asset_link_list,
                asset_path,
            })
            .map_err(|error| error.0),
    )
}

#[tauri::command]
pub async fn select_dictation_model(
    settings: tauri::State<'_, SettingsHandle>,
    asset_id: String,
) -> Result<SettingsSnapshot, String> {
    command_finish(
        "select_dictation_model",
        settings
            .select_dictation_model(&asset_id)
            .map_err(|error| error.0),
    )
}

#[tauri::command]
pub async fn get_operation(
    settings: tauri::State<'_, SettingsHandle>,
    operation_id: String,
) -> Result<OperationProgress, String> {
    command_finish(
        "get_operation",
        settings
            .get_operation(&operation_id)
            .map_err(|error| error.0),
    )
}

#[tauri::command]
pub async fn begin_evoke_setup(
    settings: tauri::State<'_, SettingsHandle>,
    request: StartEvokeSetup,
) -> Result<EvokeSetupSession, String> {
    command_finish(
        "begin_evoke_setup",
        settings.begin_evoke_setup(request).map_err(|error| error.0),
    )
}

#[tauri::command]
pub async fn capture_evoke_sample(
    settings: tauri::State<'_, SettingsHandle>,
    setup_id: String,
) -> Result<RecordingReceipt, String> {
    command_finish(
        "capture_evoke_sample",
        settings
            .capture_evoke_sample(&setup_id)
            .await
            .map_err(|error| error.0),
    )
}

#[tauri::command]
pub async fn finish_evoke_setup(
    settings: tauri::State<'_, SettingsHandle>,
    setup_id: String,
) -> Result<String, String> {
    command_finish(
        "finish_evoke_setup",
        settings
            .finish_evoke_setup(&setup_id)
            .map_err(|error| error.0),
    )
}

#[tauri::command]
pub async fn get_evoke_setup(
    settings: tauri::State<'_, SettingsHandle>,
    setup_id: String,
) -> Result<EvokeSetupSession, String> {
    command_finish(
        "get_evoke_setup",
        settings.get_evoke_setup(&setup_id).map_err(|error| error.0),
    )
}

#[tauri::command]
pub async fn cancel_evoke_setup(
    settings: tauri::State<'_, SettingsHandle>,
    setup_id: String,
) -> Result<(), String> {
    command_finish(
        "cancel_evoke_setup",
        settings
            .cancel_evoke_setup(&setup_id)
            .map_err(|error| error.0),
    )
}

/// 设置唤醒词敏感度（EvokeWord 二级页滑杆，0.0-1.0）。
#[tauri::command]
pub async fn set_sensitivity(
    settings: tauri::State<'_, SettingsHandle>,
    value: f32,
) -> Result<SettingsSnapshot, String> {
    const COMMAND: &str = "set_sensitivity";
    tracing::info!(command = COMMAND, value, "Tauri command begin");
    let snapshot = settings.set_sensitivity(value).map_err(|error| error.0)?;
    command_finish(COMMAND, Ok(snapshot))
}

/// 获取历史记录列表（History 二级页，最多 20 条）。
#[tauri::command]
pub async fn list_history(
    runtime: tauri::State<'_, RuntimeHandle>,
) -> Result<Vec<HistoryEntry>, String> {
    const COMMAND: &str = "list_history";
    command_begin(COMMAND);
    let result = runtime.get_history().await.map_err(|error| error.0);
    if let Ok(entries) = &result {
        tracing::debug!(command = COMMAND, count = entries.len(), "history listed");
    }
    command_finish(COMMAND, result)
}

/// 复制某条历史记录文本到系统剪贴板。
#[tauri::command]
pub async fn copy_history_text(
    runtime: tauri::State<'_, RuntimeHandle>,
    id: String,
) -> Result<(), String> {
    const COMMAND: &str = "copy_history_text";
    tracing::info!(command = COMMAND, %id, "Tauri command begin");
    let result: Result<(), String> = async {
        let entry = runtime
            .get_history_entry(id)
            .await
            .map_err(|error| error.0)?;
        tokio::task::spawn_blocking(move || {
            let mut clipboard = arboard::Clipboard::new()
                .map_err(|error| format!("failed to open system clipboard: {error}"))?;
            clipboard
                .set_text(entry.text)
                .map_err(|error| format!("failed to copy history text: {error}"))
        })
        .await
        .map_err(|error| format!("clipboard task failed: {error}"))?
    }
    .await;
    command_finish(COMMAND, result)
}

/// 播放某条历史记录的录音。
#[tauri::command]
pub async fn play_history_audio(
    runtime: tauri::State<'_, RuntimeHandle>,
    id: String,
) -> Result<(), String> {
    const COMMAND: &str = "play_history_audio";
    tracing::info!(command = COMMAND, %id, "Tauri command begin");
    let result: Result<(), String> = async {
        let entry = runtime
            .get_history_entry(id)
            .await
            .map_err(|error| error.0)?;
        tokio::task::spawn_blocking(move || {
            let file = std::fs::File::open(&entry.audio_path).map_err(|error| {
                format!(
                    "failed to open history recording '{}': {error}",
                    entry.audio_path
                )
            })?;
            let stream = OutputStreamBuilder::open_default_stream()
                .map_err(|error| format!("failed to open default audio output: {error}"))?;
            let player = rodio::play(stream.mixer(), std::io::BufReader::new(file))
                .map_err(|error| format!("failed to play history recording: {error}"))?;
            player.sleep_until_end();
            Ok(())
        })
        .await
        .map_err(|error| format!("audio playback task failed: {error}"))?
    }
    .await;
    command_finish(COMMAND, result)
}

/// MainWindow 标题栏"播放"按钮：进入后台运行，语义等同于关闭 MainWindow——
/// 驱动 `StateEvent::MainWindowClosed`，回到 `Listening`（待唤醒），HudWindow 随之显示。
/// 新状态通过 `EVENT_STATE_CHANGED` 广播，本命令不直接返回状态。
#[tauri::command]
pub async fn request_background(runtime: tauri::State<'_, RuntimeHandle>) -> Result<(), String> {
    const COMMAND: &str = "request_background";
    command_begin(COMMAND);
    command_finish(
        COMMAND,
        runtime
            .handle_event(StateEvent::MainWindowClosed)
            .await
            .map(|_| ())
            .map_err(|error| error.0),
    )
}

/// MainWindow 标题栏"电源"按钮：无需二次确认，直接终止整个 Runtime 进程。
/// 不经过 State Machine，与系统托盘右键菜单"退出"走同一路径（见 `Runtime::shutdown`）。
/// 调用一般不会返回（进程已退出）。
#[tauri::command]
pub async fn quit_app(runtime: tauri::State<'_, RuntimeHandle>) -> Result<(), String> {
    const COMMAND: &str = "quit_app";
    command_begin(COMMAND);
    command_finish(COMMAND, runtime.shutdown().await.map_err(|error| error.0))
}
