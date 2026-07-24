/**
 * Tauri `invoke()` 调用的类型安全封装（对应 runtime/src/commands.rs 中的每个 #[tauri::command]）。
 * 本阶段仅定义签名，均未实现。
 */

import type { AppConfig, AudioDeviceInfo, DmState, HistoryEntry } from './types';

/** 获取当前 Runtime 状态。对应 Rust `get_state`。 */
export function getState(): Promise<DmState> {
  throw new Error('Not Implemented');
}

/** 列出可用输入设备。对应 Rust `list_devices`。 */
export function listDevices(): Promise<AudioDeviceInfo[]> {
  throw new Error('Not Implemented');
}

/** 切换输入设备。对应 Rust `set_input_device`。 */
export function setInputDevice(deviceId: string): Promise<void> {
  throw new Error('Not Implemented');
}

/** 获取当前配置。对应 Rust `get_config`。 */
export function getConfig(): Promise<AppConfig> {
  throw new Error('Not Implemented');
}

/** 切换生效唤醒词。对应 Rust `set_evoke_word`。 */
export function setEvokeWord(word: string): Promise<void> {
  throw new Error('Not Implemented');
}

/** 设置唤醒词敏感度（0.0-1.0）。对应 Rust `set_sensitivity`。 */
export function setSensitivity(value: number): Promise<void> {
  throw new Error('Not Implemented');
}

/** 获取历史记录列表（最多 20 条）。对应 Rust `list_history`。 */
export function listHistory(): Promise<HistoryEntry[]> {
  throw new Error('Not Implemented');
}

/** 复制某条历史记录文本到剪贴板。对应 Rust `copy_history_text`。 */
export function copyHistoryText(id: string): Promise<void> {
  throw new Error('Not Implemented');
}

/** 播放某条历史记录的音频。对应 Rust `play_history_audio`。 */
export function playHistoryAudio(id: string): Promise<void> {
  throw new Error('Not Implemented');
}

/** 进入后台运行（标题栏播放按钮）：等价于关闭 MainWindow，回到 Listening/待唤醒，HudWindow 显示。对应 Rust `request_background`。 */
export function requestBackground(): Promise<void> {
  throw new Error('Not Implemented');
}

/** 退出整个程序（标题栏电源按钮）：无需确认，直接终止 Runtime 进程。对应 Rust `quit_app`。 */
export function quitApp(): Promise<void> {
  throw new Error('Not Implemented');
}
