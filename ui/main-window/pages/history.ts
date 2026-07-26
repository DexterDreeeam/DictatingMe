/**
 * History 二级页：最近 20 条记录，支持复制/播放，不支持删除/搜索，
 * 见 brainstrom/plan.md §6.4。
 */

import type { PageComponent } from '../page';
import { copyHistoryText, listHistory, playHistoryAudio } from '../../shared/api';
import { onHistoryUpdated, type Unlisten } from '../../shared/events';
import { HISTORY_CAPACITY, type HistoryEntry } from '../../shared/types';
import { renderHistoryItem } from '../components/history-item';

export class HistoryPage implements PageComponent {
  #generation = 0;
  #unlisten: Unlisten | null = null;
  #entries: HistoryEntry[] = [];
  #list: HTMLElement | null = null;
  #status: HTMLElement | null = null;
  readonly #busyEntries = new Set<string>();

  mount(container: HTMLElement): void {
    const generation = ++this.#generation;
    container.replaceChildren(this.#renderLoading());
    void this.#subscribe(generation);
    void this.#load(container, generation);
  }

  unmount(): void {
    this.#generation += 1;
    this.#unlisten?.();
    this.#unlisten = null;
    this.#entries = [];
    this.#list = null;
    this.#status = null;
    this.#busyEntries.clear();
  }

  async #load(container: HTMLElement, generation: number): Promise<void> {
    try {
      const entries = await listHistory();
      if (generation !== this.#generation) {
        return;
      }
      this.#entries = this.#normalize([...this.#entries, ...entries]);
      container.replaceChildren(this.#renderPage());
    } catch (error) {
      if (generation !== this.#generation) {
        return;
      }
      container.replaceChildren(this.#renderError(container));
      console.error('Failed to load history:', error);
    }
  }

  async #subscribe(generation: number): Promise<void> {
    try {
      const unlisten = await onHistoryUpdated((entry) => {
        if (generation !== this.#generation) {
          return;
        }
        this.#entries = this.#normalize([
          entry,
          ...this.#entries.filter((candidate) => candidate.id !== entry.id),
        ]);
        this.#renderList();
      });
      if (generation !== this.#generation) {
        unlisten();
        return;
      }
      this.#unlisten = unlisten;
    } catch (error) {
      if (generation === this.#generation) {
        console.error('Failed to subscribe to history updates:', error);
      }
    }
  }

  #renderPage(): HTMLElement {
    const page = this.#createPageShell();
    this.#list = document.createElement('div');
    this.#list.className = 'history-list history-scroll';
    this.#list.setAttribute('aria-label', '最近的听写记录');
    this.#status = document.createElement('p');
    this.#status.className = 'inline-status history-status';
    this.#status.setAttribute('role', 'alert');
    this.#renderList();

    page.append(this.#list, this.#status);
    return page;
  }

  #renderList(): void {
    if (!this.#list) {
      return;
    }
    window.dispatchEvent(new CustomEvent('dictatingme:history-count', {
      detail: this.#entries.length,
    }));
    this.#list.replaceChildren();
    if (this.#entries.length === 0) {
      const empty = document.createElement('div');
      empty.className = 'state-panel state-panel--empty';
      const title = document.createElement('h3');
      title.textContent = '还没有听写记录';
      const detail = document.createElement('p');
      detail.textContent = '进入后台监听并完成一次听写后，文本和录音会出现在这里。';
      empty.append(title, detail);
      this.#list.append(empty);
      return;
    }

    for (const entry of this.#entries) {
      this.#list.append(renderHistoryItem({
        entry,
        onCopy: (id) => void this.#runAction(id, 'copy'),
        onPlay: (id) => void this.#runAction(id, 'play'),
      }));
    }
  }

  async #runAction(id: string, action: 'copy' | 'play'): Promise<void> {
    if (this.#busyEntries.has(id)) {
      return;
    }
    const generation = this.#generation;
    this.#busyEntries.add(id);
    this.#setItemBusy(id, true);
    this.#setStatus('');
    try {
      if (action === 'copy') {
        await copyHistoryText(id);
      } else {
        await playHistoryAudio(id);
      }
      if (generation !== this.#generation) {
        return;
      }
      this.#setStatus('');
    } catch (error) {
      if (generation !== this.#generation) {
        return;
      }
      this.#setStatus(
        action === 'copy' ? '无法复制这条文本，请重试。' : '无法播放这段录音，请重试。',
        'error',
      );
      console.error(`Failed to ${action} history entry:`, error);
    } finally {
      this.#busyEntries.delete(id);
      if (generation === this.#generation) {
        this.#setItemBusy(id, false);
      }
    }
  }

  #normalize(entries: HistoryEntry[]): HistoryEntry[] {
    return [...new Map(entries.map((entry) => [entry.id, entry])).values()]
      .sort((left, right) => right.timestampMs - left.timestampMs)
      .slice(0, HISTORY_CAPACITY);
  }

  #createPageShell(): HTMLElement {
    const page = document.createElement('section');
    page.className = 'page page--history';
    return page;
  }

  #renderLoading(): HTMLElement {
    const page = this.#createPageShell();
    const list = document.createElement('div');
    list.className = 'history-list';
    list.setAttribute('aria-busy', 'true');
    list.setAttribute('aria-label', '正在加载听写历史');
    for (let index = 0; index < 4; index += 1) {
      const skeleton = document.createElement('div');
      skeleton.className = 'skeleton skeleton--history';
      list.append(skeleton);
    }
    page.append(list);
    return page;
  }

  #renderError(container: HTMLElement): HTMLElement {
    const page = this.#createPageShell();
    const state = document.createElement('div');
    state.className = 'state-panel state-panel--error';
    state.setAttribute('role', 'alert');
    const title = document.createElement('h2');
    title.textContent = '无法加载听写历史';
    const detail = document.createElement('p');
    detail.textContent = '历史记录仍安全保存在本机，请稍后重试。';
    const retry = document.createElement('button');
    retry.type = 'button';
    retry.className = 'button button--primary';
    retry.textContent = '重新加载';
    retry.addEventListener('click', () => {
      const generation = ++this.#generation;
      this.#unlisten?.();
      this.#unlisten = null;
      container.replaceChildren(this.#renderLoading());
      void this.#subscribe(generation);
      void this.#load(container, generation);
    });
    state.append(title, detail, retry);
    page.append(state);
    return page;
  }

  #setItemBusy(id: string, busy: boolean): void {
    this.#list?.querySelectorAll<HTMLElement>('.history-item').forEach((item) => {
      if (item.dataset.historyId !== id) {
        return;
      }
      item.setAttribute('aria-busy', String(busy));
      item.querySelectorAll('button').forEach((button) => {
        button.disabled = busy;
      });
    });
  }

  #setStatus(message: string, kind: 'success' | 'error' | null = null): void {
    if (!this.#status) {
      return;
    }
    this.#status.textContent = message;
    if (kind) {
      this.#status.dataset.kind = kind;
    } else {
      delete this.#status.dataset.kind;
    }
  }
}
