/**
 * Tauri `listen()` 事件订阅封装（对应 runtime/src/events.rs 中的事件常量）。
 */

import type { DmState, HistoryEntry } from './types';

/** 取消订阅函数类型。 */
export type Unlisten = () => void;

/** 订阅状态变化事件（"state-changed"）。HudWindow 用于切换黄/绿灯。 */
export function onStateChanged(callback: (state: DmState) => void): Promise<Unlisten> {
  throw new Error('Not Implemented');
}

/** 订阅历史记录新增事件（"history-updated"）。History 二级页用于实时刷新列表。 */
export function onHistoryUpdated(callback: (entry: HistoryEntry) => void): Promise<Unlisten> {
  throw new Error('Not Implemented');
}
