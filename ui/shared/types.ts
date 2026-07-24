/**
 * 前端共享类型定义（对应 Rust 端 state_machine / storage / audio 模块的数据形状）。
 * 见 brainstrom/plan.md §4.1、§6、§9。
 */

/** Runtime 状态机的 5 个状态（对应 Rust `State` enum）。 */
export type DmState = 'Configure' | 'Listening' | 'Loading' | 'Dictating' | 'Unloading';

/** HUD 灯光颜色（黄=可唤醒，绿=记录输入中，Off=无响应/瞬时熄灭）。 */
export type HudLight = 'Yellow' | 'Green' | 'Off';

/** 一个可用的音频输入设备（InputDevice 二级页）。 */
export interface AudioDeviceInfo {
  id: string;
  name: string;
  isDefault: boolean;
}

/** 持久化配置（对应 Rust `AppConfig`）。 */
export interface AppConfig {
  inputDeviceId: string;
  /** 当前生效唤醒词，同一时间只能有 1 个（见 plan.md §6.3） */
  evokeWord: string;
  /** 敏感度 0.0（不易误触发）- 1.0（更容易唤醒） */
  sensitivity: number;
}

/** 一条历史听写记录（History 二级页，FIFO 最多 20 条）。 */
export interface HistoryEntry {
  id: string;
  /** Unix 毫秒时间戳 */
  timestampMs: number;
  text: string;
  /** 录音文件路径（供播放） */
  audioPath: string;
}

/** History 列表容量上限，见 plan.md §6.4。 */
export const HISTORY_CAPACITY = 20;
