/**
 * History 二级页：最近 20 条记录，支持复制/播放，不支持删除/搜索，
 * 见 brainstrom/plan.md §6.4。
 */

import type { PageComponent } from '../page';

export class HistoryPage implements PageComponent {
  mount(container: HTMLElement): void {
    throw new Error('Not Implemented');
  }

  unmount(): void {
    throw new Error('Not Implemented');
  }
}
