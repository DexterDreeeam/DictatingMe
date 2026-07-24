/**
 * HudWindow 入口：订阅状态变化，切换黄/绿灯（见 brainstrom/plan.md §7）。
 * 黄=Listening(可唤醒)，绿=Loading/Dictating(记录输入中)；Configure 态本窗口不存在/隐藏。
 */

import { onStateChanged } from '../shared/events';
import type { DmState, HudLight } from '../shared/types';

/** 状态 -> HUD 灯光颜色的映射（与 Rust `State::hud_light()` 保持一致）。 */
export function mapStateToHudLight(state: DmState): HudLight {
  throw new Error('Not Implemented');
}

/** 将灯光颜色应用到 DOM（更新 #hud 的样式/文案）。 */
function applyHudLight(light: HudLight): void {
  throw new Error('Not Implemented');
}

function bootstrap(): void {
  throw new Error('Not Implemented');
}

bootstrap();
