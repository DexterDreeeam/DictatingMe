//! Tauri 事件层：Runtime 向前端（MainWindow / HudWindow）广播的事件（见 brainstrom/plan.md §7）。

use tauri::{AppHandle, Emitter};

use crate::scoring::EvokeScore;
use crate::state_machine::State;
use crate::storage::HistoryEntry;

/// 事件名常量，供前端 `listen()` 订阅（见 ui/shared/events.ts）。
pub const EVENT_STATE_CHANGED: &str = "state-changed";
pub const EVENT_HISTORY_UPDATED: &str = "history-updated";
pub const EVENT_INPUT_LEVEL: &str = "input-level";
pub const EVENT_EVOKE_SCORE: &str = "evoke-score";

/// 状态变化时广播（HudWindow 据此切换黄/绿灯，MainWindow 首页据此更新提示）。
pub fn emit_state_changed(app: &AppHandle, state: State) -> Result<(), tauri::Error> {
    app.emit(EVENT_STATE_CHANGED, state)
}

/// 新增历史记录时广播（History 二级页据此实时刷新列表）。
pub fn emit_history_updated(app: &AppHandle, entry: HistoryEntry) -> Result<(), tauri::Error> {
    app.emit(EVENT_HISTORY_UPDATED, entry)
}

/// 输入设备页打开期间推送节流后的麦克风电平（0.0-1.0）。
pub fn emit_input_level(app: &AppHandle, level: f32) -> Result<(), tauri::Error> {
    app.emit(EVENT_INPUT_LEVEL, level.clamp(0.0, 1.0))
}

pub fn emit_evoke_score(app: &AppHandle, score: EvokeScore) -> Result<(), tauri::Error> {
    app.emit(EVENT_EVOKE_SCORE, score)
}
