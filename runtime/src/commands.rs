//! Tauri 命令层：前端通过 `invoke()` 调用这些命令（见 brainstrom/plan.md §6 MainWindow UX）。

use std::sync::Mutex;
use tauri::State as TauriState;

use crate::audio::AudioDeviceInfo;
use crate::runtime::Runtime;
use crate::state_machine::State;
use crate::storage::{AppConfig, HistoryEntry};

/// 被 Tauri 托管的 Runtime 类型别名（内部需要可变访问，故用 Mutex 包裹）。
pub type ManagedRuntime<'a> = TauriState<'a, Mutex<Runtime>>;

/// 获取当前 Runtime 状态（供 MainWindow 首页状态提示、HudWindow 灯光使用）。
#[tauri::command]
pub fn get_state(runtime: ManagedRuntime) -> State {
    todo!()
}

/// 列出可用输入设备（InputDevice 二级页）。
#[tauri::command]
pub fn list_devices(runtime: ManagedRuntime) -> Result<Vec<AudioDeviceInfo>, String> {
    todo!()
}

/// 切换输入设备（InputDevice 二级页选中某项）。
#[tauri::command]
pub fn set_input_device(runtime: ManagedRuntime, device_id: String) -> Result<(), String> {
    todo!()
}

/// 获取当前配置（含唤醒词、敏感度、输入设备）。
#[tauri::command]
pub fn get_config(runtime: ManagedRuntime) -> Result<AppConfig, String> {
    todo!()
}

/// 切换生效唤醒词（EvokeWord 二级页）。
#[tauri::command]
pub fn set_evoke_word(runtime: ManagedRuntime, word: String) -> Result<(), String> {
    todo!()
}

/// 设置唤醒词敏感度（EvokeWord 二级页滑杆，0.0-1.0）。
#[tauri::command]
pub fn set_sensitivity(runtime: ManagedRuntime, value: f32) -> Result<(), String> {
    todo!()
}

/// 获取历史记录列表（History 二级页，最多 20 条）。
#[tauri::command]
pub fn list_history(runtime: ManagedRuntime) -> Result<Vec<HistoryEntry>, String> {
    todo!()
}

/// 复制某条历史记录文本到系统剪贴板。
#[tauri::command]
pub fn copy_history_text(runtime: ManagedRuntime, id: String) -> Result<(), String> {
    todo!()
}

/// 播放某条历史记录的录音。
#[tauri::command]
pub fn play_history_audio(runtime: ManagedRuntime, id: String) -> Result<(), String> {
    todo!()
}

/// MainWindow 标题栏"播放"按钮：进入后台运行，语义等同于关闭 MainWindow——
/// 驱动 `StateEvent::MainWindowClosed`，回到 `Listening`（待唤醒），HudWindow 随之显示。
/// 新状态通过 `EVENT_STATE_CHANGED` 广播，本命令不直接返回状态。
#[tauri::command]
pub fn request_background(runtime: ManagedRuntime) -> Result<(), String> {
    todo!()
}

/// MainWindow 标题栏"电源"按钮：无需二次确认，直接终止整个 Runtime 进程。
/// 不经过 State Machine，与系统托盘右键菜单"退出"走同一路径（见 `Runtime::shutdown`）。
/// 调用一般不会返回（进程已退出）。
#[tauri::command]
pub fn quit_app(runtime: ManagedRuntime) -> Result<(), String> {
    todo!()
}
