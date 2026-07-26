/**
 * Tauri `invoke()` 调用的类型安全封装（对应 runtime/src/commands.rs 中的每个 #[tauri::command]）。
 */

import { invoke } from '@tauri-apps/api/core';
import type {
  AppConfig,
  AssetSummary,
  AudioDeviceInfo,
  DmState,
  EvokeSetupSession,
  HistoryEntry,
  OperationProgress,
  RecordingReceipt,
  SettingsSnapshot,
  StartEvokeSetup,
} from './types';

/** 获取当前 Runtime 状态。对应 Rust `get_state`。 */
export function getState(): Promise<DmState> {
  return invoke<DmState>('get_state');
}

export function frontendReady(): Promise<void> {
  return invoke<void>('frontend_ready');
}

/** 列出可用输入设备。对应 Rust `list_devices`。 */
export function listDevices(): Promise<AudioDeviceInfo[]> {
  return invoke<AudioDeviceInfo[]>('list_devices');
}

/** 切换输入设备。对应 Rust `set_input_device`。 */
export function setInputDevice(deviceId: string): Promise<AppConfig> {
  return invoke<AppConfig>('set_input_device', { deviceId });
}

/** 获取当前配置。对应 Rust `get_config`。 */
export function getConfig(): Promise<AppConfig> {
  return invoke<AppConfig>('get_config');
}

/** 设置唤醒词敏感度（0.0-1.0）。对应 Rust `set_sensitivity`。 */
export function setSensitivity(value: number): Promise<SettingsSnapshot> {
  return invoke<SettingsSnapshot>('set_sensitivity', { value });
}

export function getSettingsSnapshot(): Promise<SettingsSnapshot> {
  return invoke<SettingsSnapshot>('get_settings_snapshot');
}

export function inspectAsset(assetPath: string): Promise<AssetSummary> {
  return invoke<AssetSummary>('inspect_asset', { assetPath });
}

export function installAsset(assetLinkList: readonly string[], assetPath: string): Promise<string> {
  return invoke<string>('install_asset', { assetLinkList, assetPath });
}

export function selectDictationModel(assetId: string): Promise<SettingsSnapshot> {
  return invoke<SettingsSnapshot>('select_dictation_model', { assetId });
}

export function getOperation(operationId: string): Promise<OperationProgress> {
  return invoke<OperationProgress>('get_operation', { operationId });
}

export function beginEvokeSetup(request: StartEvokeSetup): Promise<EvokeSetupSession> {
  return invoke<EvokeSetupSession>('begin_evoke_setup', { request });
}

export function captureEvokeSample(setupId: string): Promise<RecordingReceipt> {
  return invoke<RecordingReceipt>('capture_evoke_sample', { setupId });
}

export function finishEvokeSetup(setupId: string): Promise<string> {
  return invoke<string>('finish_evoke_setup', { setupId });
}

export function getEvokeSetup(setupId: string): Promise<EvokeSetupSession> {
  return invoke<EvokeSetupSession>('get_evoke_setup', { setupId });
}

export function cancelEvokeSetup(setupId: string): Promise<void> {
  return invoke<void>('cancel_evoke_setup', { setupId });
}

/** 获取历史记录列表（最多 20 条）。对应 Rust `list_history`。 */
export function listHistory(): Promise<HistoryEntry[]> {
  return invoke<HistoryEntry[]>('list_history');
}

/** 复制某条历史记录文本到剪贴板。对应 Rust `copy_history_text`。 */
export function copyHistoryText(id: string): Promise<void> {
  return invoke<void>('copy_history_text', { id });
}

/** 播放某条历史记录的音频。对应 Rust `play_history_audio`。 */
export function playHistoryAudio(id: string): Promise<void> {
  return invoke<void>('play_history_audio', { id });
}

/** 进入后台运行（标题栏播放按钮）：等价于关闭 MainWindow，回到 Listening/待唤醒，HudWindow 显示。对应 Rust `request_background`。 */
export function requestBackground(): Promise<void> {
  return invoke<void>('request_background');
}

/** 退出整个程序（标题栏电源按钮）：无需确认，直接终止 Runtime 进程。对应 Rust `quit_app`。 */
export function quitApp(): Promise<void> {
  return invoke<void>('quit_app');
}
