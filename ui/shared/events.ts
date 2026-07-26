/**
 * Tauri `listen()` 事件订阅封装（对应 runtime/src/events.rs 中的事件常量）。
 */

import { listen } from '@tauri-apps/api/event';
import type {
  DmState,
  EvokeScore,
  HistoryEntry,
  OperationProgress,
} from './types';

/** 取消订阅函数类型。 */
export type Unlisten = () => void;

/** 订阅状态变化事件（"state-changed"）。HudWindow 用于切换黄/绿灯。 */
export function onStateChanged(callback: (state: DmState) => void): Promise<Unlisten> {
  return listen<DmState>('state-changed', (event) => callback(event.payload));
}

/** 订阅历史记录新增事件（"history-updated"）。History 二级页用于实时刷新列表。 */
export function onHistoryUpdated(callback: (entry: HistoryEntry) => void): Promise<Unlisten> {
  return listen<HistoryEntry>('history-updated', (event) => callback(event.payload));
}

/** 订阅当前输入设备的实时音量（"input-level"，范围 0-1）。 */
export function onInputLevel(callback: (level: number) => void): Promise<Unlisten> {
  return listen<number>('input-level', (event) => callback(event.payload));
}

export function onEvokeScore(callback: (score: EvokeScore) => void): Promise<Unlisten> {
  return listen<EvokeScore>('evoke-score', (event) => callback(event.payload));
}

export function onOperationProgress(
  callback: (progress: OperationProgress) => void,
): Promise<Unlisten> {
  return listen<OperationProgress>('operation-progress', (event) => callback(event.payload));
}

export function onSettingsChanged(callback: (generation: number) => void): Promise<Unlisten> {
  return listen<{ generation: number }>('settings-changed', (event) => {
    callback(event.payload.generation);
  });
}
