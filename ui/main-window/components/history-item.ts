/**
 * History 二级页的单条记录（时间戳 + 文本 + 复制/播放按钮），
 * 见 brainstrom/plan.md §6.4。
 */

import type { HistoryEntry } from '../../shared/types';

export interface HistoryItemProps {
  entry: HistoryEntry;
  onCopy: (id: string) => void;
  onPlay: (id: string) => void;
}

export function renderHistoryItem(props: HistoryItemProps): HTMLElement {
  throw new Error('Not Implemented');
}
