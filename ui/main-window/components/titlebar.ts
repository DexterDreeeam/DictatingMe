/** 自定义无边框标题栏。 */

export interface TitleBarProps {
  /** 固定显示 "Dictating Me" */
  appName: string;
  /** 点击播放按钮：进入后台运行，调用 `requestBackground()`（等价于关闭 MainWindow） */
  onBackground: () => void;
  /** 点击电源按钮：无需确认，调用 `quitApp()` 直接终止整个 Runtime 进程 */
  onQuit: () => void;
}

export function renderTitleBar(props: TitleBarProps): HTMLElement {
  const titleBar = document.createElement('header');
  titleBar.className = 'titlebar';

  const identity = document.createElement('div');
  identity.className = 'titlebar__identity';

  const homeButton = document.createElement('button');
  homeButton.className = 'titlebar__home';
  homeButton.type = 'button';
  homeButton.dataset.route = 'home';
  homeButton.setAttribute('aria-label', '回到首页');
  homeButton.title = '回到首页';
  homeButton.append(createIcon([
    ['path', { d: 'M3 11.5 12 4l9 7.5' }],
    ['path', { d: 'M5.5 10v9a1 1 0 0 0 1 1H9a1 1 0 0 0 1-1v-4a1 1 0 0 1 1-1h2a1 1 0 0 1 1 1v4a1 1 0 0 0 1 1h2.5a1 1 0 0 0 1-1v-9' }],
  ]));

  const pageTitle = document.createElement('div');
  pageTitle.className = 'titlebar__page-title';
  const pageTitleText = document.createElement('span');
  pageTitleText.className = 'titlebar__page-title-text';
  const count = document.createElement('span');
  count.className = 'titlebar__count';
  count.hidden = true;
  pageTitle.append(pageTitleText, count);
  identity.append(homeButton, pageTitle);

  const actions = document.createElement('div');
  actions.className = 'titlebar__actions';

  const backgroundButton = document.createElement('button');
  backgroundButton.className = 'icon-button titlebar__button';
  backgroundButton.dataset.action = 'background';
  backgroundButton.type = 'button';
  backgroundButton.setAttribute('aria-label', '进入后台并开始监听');
  backgroundButton.title = '进入后台并开始监听';
  backgroundButton.append(createIcon([
    ['path', {
      d: 'M5 5.5v13c0 1.92 1.36 2.71 3.03 1.76l10.79-6.03c1.68-.94 1.68-2.52 0-3.46L8.03 4.74C6.36 3.79 5 4.58 5 6.5',
      fill: 'currentColor',
      stroke: 'none',
    }],
  ]));
  backgroundButton.addEventListener('click', props.onBackground);

  const quitButton = document.createElement('button');
  quitButton.className = 'icon-button titlebar__button titlebar__button--danger';
  quitButton.dataset.action = 'quit';
  quitButton.type = 'button';
  quitButton.setAttribute('aria-label', '退出 Dictating Me');
  quitButton.title = '退出程序';
  quitButton.append(createIcon([
    ['path', { d: 'M12 2v8' }],
    ['path', { d: 'M18.36 6.64a9 9 0 1 1-12.73 0' }],
  ]));
  quitButton.addEventListener('click', props.onQuit);

  actions.append(backgroundButton, quitButton);
  titleBar.append(identity, actions);
  return titleBar;
}

type IconNode = ['path', Record<string, string>];

function createIcon(nodes: IconNode[]): SVGSVGElement {
  const namespace = 'http://www.w3.org/2000/svg';
  const svg = document.createElementNS(namespace, 'svg');
  svg.setAttribute('viewBox', '0 0 24 24');
  svg.setAttribute('fill', 'none');
  svg.setAttribute('stroke', 'currentColor');
  svg.setAttribute('stroke-width', '1.8');
  svg.setAttribute('stroke-linecap', 'round');
  svg.setAttribute('stroke-linejoin', 'round');
  svg.setAttribute('aria-hidden', 'true');
  for (const [tag, attributes] of nodes) {
    const node = document.createElementNS(namespace, tag);
    for (const [name, value] of Object.entries(attributes)) {
      node.setAttribute(name, value);
    }
    svg.append(node);
  }
  return svg;
}
