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
  const article = document.createElement('article');
  article.className = 'history-item';
  article.dataset.historyId = props.entry.id;

  const content = document.createElement('div');
  content.className = 'history-item__content';

  const meta = document.createElement('div');
  meta.className = 'history-item__meta';
  const time = document.createElement('time');
  time.className = 'history-item__time';
  const date = new Date(props.entry.timestampMs);
  if (Number.isNaN(date.getTime())) {
    time.textContent = '时间未知';
  } else {
    time.dateTime = date.toISOString();
    time.textContent = new Intl.DateTimeFormat('zh-CN', {
      month: 'short',
      day: 'numeric',
      hour: '2-digit',
      minute: '2-digit',
    }).format(date);
  }

  const text = document.createElement('p');
  text.className = 'history-item__text';
  text.textContent = props.entry.text;

  const actions = document.createElement('div');
  actions.className = 'history-item__actions';

  const playButton = document.createElement('button');
  playButton.type = 'button';
  playButton.className = 'icon-button history-item__action';
  playButton.setAttribute('aria-label', `播放 ${time.textContent} 的录音`);
  playButton.title = '播放录音';
  playButton.append(createHistoryIcon('play'));
  playButton.addEventListener('click', () => props.onPlay(props.entry.id));

  const copyButton = document.createElement('button');
  copyButton.type = 'button';
  copyButton.className = 'icon-button history-item__action';
  copyButton.setAttribute('aria-label', `复制 ${time.textContent} 的听写文本`);
  copyButton.title = '复制文本';
  copyButton.append(createHistoryIcon('copy'));
  copyButton.addEventListener('click', () => props.onCopy(props.entry.id));

  actions.append(playButton, copyButton);
  meta.append(time, actions);
  content.append(meta, text);
  article.append(content);
  return article;
}

function createHistoryIcon(kind: 'play' | 'copy'): SVGSVGElement {
  const namespace = 'http://www.w3.org/2000/svg';
  const svg = document.createElementNS(namespace, 'svg');
  svg.setAttribute('viewBox', '0 0 24 24');
  svg.setAttribute('fill', 'none');
  svg.setAttribute('stroke', 'currentColor');
  svg.setAttribute('stroke-width', '1.7');
  svg.setAttribute('stroke-linecap', 'round');
  svg.setAttribute('stroke-linejoin', 'round');
  svg.setAttribute('aria-hidden', 'true');
  if (kind === 'play') {
    const path = document.createElementNS(namespace, 'path');
    path.setAttribute('d', 'm8 5 11 7-11 7V5z');
    svg.append(path);
  } else {
    const front = document.createElementNS(namespace, 'rect');
    front.setAttribute('x', '8');
    front.setAttribute('y', '8');
    front.setAttribute('width', '11');
    front.setAttribute('height', '11');
    front.setAttribute('rx', '2');
    const back = document.createElementNS(namespace, 'path');
    back.setAttribute('d', 'M16 8V5a2 2 0 0 0-2-2H5a2 2 0 0 0-2 2v9a2 2 0 0 0 2 2h3');
    svg.append(front, back);
  }
  return svg;
}
