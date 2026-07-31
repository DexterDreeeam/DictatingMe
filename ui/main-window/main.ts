/** MainWindow 入口。 */

import { Router } from './router';
import { HomePage } from './pages/home';
import { InputDevicePage } from './pages/input-device';
import { EvokeWordPage } from './pages/evoke-word';
import { HistoryPage } from './pages/history';
import { SpeechModelPage } from './pages/speech-model';
import {
  frontendReady,
  getSettingsSnapshot,
  getState,
  quitApp,
  requestBackground,
} from '../shared/api';
import { onSettingsChanged, onStateChanged, type Unlisten } from '../shared/events';
import type { DmState } from '../shared/types';
import { renderTitleBar } from './components/titlebar';
import { LogicalSize } from '@tauri-apps/api/dpi';
import { getCurrentWindow } from '@tauri-apps/api/window';

function bootstrap(): void {
  if (document.readyState === 'loading') {
    document.addEventListener('DOMContentLoaded', () => void startWhenBackendReady(), { once: true });
  } else {
    void startWhenBackendReady();
  }
}

bootstrap();

async function startWhenBackendReady(): Promise<void> {
  while (true) {
    try {
      await Promise.all([getSettingsSnapshot(), getState()]);
      break;
    } catch {
      await new Promise((resolve) => window.setTimeout(resolve, 100));
    }
  }
  start();
  try {
    await frontendReady();
  } catch (error) {
    console.error('Backend was ready but user surfaces could not be activated:', error);
  }
}

function start(): void {
  const app = document.getElementById('app');
  if (!app) {
    throw new Error('DictatingMe bootstrap failed: missing #app root element.');
  }

  const shell = document.createElement('div');
  shell.className = 'app-shell';

  const content = document.createElement('main');
  content.className = 'app-content';
  content.id = 'main-content';

  let actionPending = false;
  let recordingLocked = false;
  let runtimeReady = false;
  const applyTitlebarAvailability = (): void => {
    const home = shell.querySelector<HTMLButtonElement>('.titlebar__home');
    const background = shell.querySelector<HTMLButtonElement>('[data-action="background"]');
    const quit = shell.querySelector<HTMLButtonElement>('[data-action="quit"]');
    if (home) home.disabled = recordingLocked || actionPending;
    if (background) background.disabled = recordingLocked || actionPending || !runtimeReady;
    if (quit) quit.disabled = actionPending;
  };
  const setWindowActionPending = (pending: boolean): void => {
    actionPending = pending;
    applyTitlebarAvailability();
  };
  const runWindowAction = async (action: 'background' | 'quit'): Promise<void> => {
    if (actionPending) {
      return;
    }
    setWindowActionPending(true);
    try {
      if (action === 'background') {
        await requestBackground();
      } else {
        await quitApp();
      }
    } catch (error) {
      console.error(`Failed to ${action} DictatingMe:`, error);
    } finally {
      setWindowActionPending(false);
    }
  };

  const titleBar = renderTitleBar({
    appName: 'Dictating Me',
    onBackground: () => void runWindowAction('background'),
    onQuit: () => void runWindowAction('quit'),
  });
  shell.append(titleBar, content);
  app.replaceChildren(shell);
  const windowResize = createWindowResizeController(titleBar, content);
  const disposeWindowDragging = installWindowDragging(shell);

  const router = new Router(content);
  const updateTitleBar = (route: 'home' | 'input-device' | 'evoke-word' | 'speech-model' | 'history'): void => {
    const titles = {
      'input-device': '输入设备',
      'evoke-word': '唤醒词',
      'speech-model': '语音模型',
      history: '历史记录',
    } as const;
    titleBar.classList.toggle('titlebar--home', route === 'home');
    const title = titleBar.querySelector<HTMLElement>('.titlebar__page-title-text');
    if (title) {
      title.textContent = route === 'home' ? '' : titles[route];
    }
    const count = titleBar.querySelector<HTMLElement>('.titlebar__count');
    if (count) {
      count.hidden = route !== 'history';
      if (route === 'history' && !count.textContent) {
        count.textContent = '0 / 20';
      }
    }
  };
  let activeRoute: 'home' | 'input-device' | 'evoke-word' | 'speech-model' | 'history' | null = null;
  const navigate = (route: 'home' | 'input-device' | 'evoke-word' | 'speech-model' | 'history'): void => {
    if (recordingLocked || activeRoute === route) {
      return;
    }
    router.navigate(route);
    activeRoute = route;
    updateTitleBar(route);
    windowResize.schedule();
  };

  router.register('home', () => new HomePage({ onNavigate: navigate }));
  router.register('input-device', () => new InputDevicePage());
  router.register('evoke-word', () => new EvokeWordPage());
  router.register('speech-model', () => new SpeechModelPage());
  router.register('history', () => new HistoryPage());

  shell.addEventListener('click', (event) => {
    const target = event.target;
    if (!(target instanceof Element)) {
      return;
    }
    const routeControl = target.closest<HTMLElement>('[data-route]');
    const route = routeControl?.dataset.route;
    if (route === 'home' || route === 'input-device' || route === 'evoke-word' || route === 'speech-model' || route === 'history') {
      navigate(route);
    }
  });

  try {
    navigate('home');
  } catch (error) {
    renderBootstrapError(content, error);
  }

  let unlisten: Unlisten | null = null;
  let unlistenSettings: Unlisten | null = null;
  let disposed = false;
  let stateEventReceived = false;
  const applyState = (state: DmState): void => {
    document.body.dataset.runtimeState = state;
    if (state === 'Configure') {
      setWindowActionPending(false);
    }
  };
  const handleHistoryCount = (event: Event): void => {
    const count = titleBar.querySelector<HTMLElement>('.titlebar__count');
    if (count && event instanceof CustomEvent && typeof event.detail === 'number') {
      count.textContent = `${event.detail} / 20`;
    }
  };
  window.addEventListener('dictatingme:history-count', handleHistoryCount);
  const handleRecordingLock = (event: Event): void => {
    if (!(event instanceof CustomEvent) || typeof event.detail !== 'boolean') return;
    recordingLocked = event.detail;
    applyTitlebarAvailability();
  };
  window.addEventListener('dictatingme:recording-lock', handleRecordingLock);
  void (async () => {
    try {
      const cleanup = await onStateChanged((state) => {
        stateEventReceived = true;
        applyState(state);
      });
      if (disposed) {
        cleanup();
      } else {
        unlisten = cleanup;
      }

      try {
        const applySettings = async (): Promise<void> => {
          const snapshot = await getSettingsSnapshot();
          if (!disposed) {
            runtimeReady = snapshot.readiness.canEnterListening;
            applyTitlebarAvailability();
          }
        };
        unlistenSettings = await onSettingsChanged(() => void applySettings());
        await applySettings();
      } catch (error) {
        console.error('Failed to read settings readiness:', error);
      }
    } catch (error) {
      console.error('Failed to subscribe to runtime state:', error);
    }

    try {
      const state = await getState();
      if (!disposed && !stateEventReceived) {
        applyState(state);
      }
    } catch (error) {
      console.error('Failed to read runtime state:', error);
    }
  })();

  window.addEventListener('beforeunload', () => {
    disposed = true;
    unlisten?.();
    unlistenSettings?.();
    window.removeEventListener('dictatingme:history-count', handleHistoryCount);
    window.removeEventListener('dictatingme:recording-lock', handleRecordingLock);
    disposeWindowDragging();
    windowResize.dispose();
  }, { once: true });
}

/// 真正的交互控件：在这些元素上按下鼠标是要操作控件，不是要拖窗口。
/// 其余任何位置都应该能拖动窗口。
const WINDOW_DRAG_EXCLUSION_SELECTOR = [
  'a',
  'button',
  'input',
  'label',
  'option',
  'select',
  'textarea',
  'summary',
  '[contenteditable="true"]',
  '[data-window-drag-exclude]',
  '[role="button"]',
  '[role="checkbox"]',
  '[role="link"]',
  '[role="menuitem"]',
  '[role="option"]',
  '[role="radio"]',
  '[role="slider"]',
  '[role="switch"]',
  '[role="tab"]',
  '[role="textbox"]',
].join(',');

function installWindowDragging(shell: HTMLElement): () => void {
  const appWindow = getCurrentWindow();
  const handleMouseDown = (event: MouseEvent): void => {
    if (event.button !== 0 || event.defaultPrevented) return;
    const target = event.target;
    if (!(target instanceof Element) || target.closest(WINDOW_DRAG_EXCLUSION_SELECTOR)) return;
    event.preventDefault();
    void appWindow.startDragging().catch((error: unknown) => {
      console.error('Failed to start window dragging:', error);
    });
  };
  shell.addEventListener('mousedown', handleMouseDown);
  return () => shell.removeEventListener('mousedown', handleMouseDown);
}

/// 窗口最小高度。测量正确时内容通常远高于此，这只是兜底，
/// 防止渲染异常导致窗口塌成一条缝。
const MIN_WINDOW_HEIGHT = 160;

function createWindowResizeController(
  titleBar: HTMLElement,
  content: HTMLElement,
): { schedule: () => void; dispose: () => void } {
  const appWindow = getCurrentWindow();  let scheduledFrame: number | null = null;
  let animationFrame: number | null = null;
  let animationGeneration = 0;
  let lastTarget = 0;

  const measureTarget = (): number => {
    const page = content.firstElementChild;
    if (!(page instanceof HTMLElement)) {
      return Math.max(MIN_WINDOW_HEIGHT, Math.round(titleBar.offsetHeight + 40));
    }
    const measurement = document.createElement('div');
    measurement.className = 'window-measure';
    measurement.style.width = `${Math.max(320, content.clientWidth || window.innerWidth)}px`;
    const clone = page.cloneNode(true);
    if (!(clone instanceof HTMLElement)) return window.innerHeight;
    // `.page` 是 height:100% 的 flex 容器。放进高度自适应的测量容器后，
    // 百分比高度会解析成 auto、内容塌成 0，量出来的 scrollHeight 就没有意义。
    // 这里把它还原成按内容撑开的普通块级流。
    clone.style.height = 'auto';
    clone.style.minHeight = '0';
    clone.style.maxHeight = 'none';
    clone.style.flex = 'none';
    clone.style.overflow = 'visible';
    clone.style.position = 'static';
    measurement.append(clone);
    document.body.append(measurement);
    // offsetHeight 取的是含 padding 的实际布局高度，比 scrollHeight 稳。
    const contentHeight = Math.max(clone.offsetHeight, clone.scrollHeight);
    const naturalHeight = titleBar.offsetHeight + contentHeight;
    measurement.remove();
    return Math.round(Math.min(
      Math.max(MIN_WINDOW_HEIGHT, naturalHeight),
      Math.max(320, window.screen.availHeight - 80),
    ));
  };

  const animateTo = (target: number): void => {
    if (Math.abs(target - lastTarget) < 2) return;
    lastTarget = target;
    const generation = ++animationGeneration;
    const start = window.innerHeight;
    const startedAt = performance.now();
    const duration = 190;
    if (animationFrame !== null) window.cancelAnimationFrame(animationFrame);
    const tick = (now: number): void => {
      if (generation !== animationGeneration) return;
      const progress = Math.min(1, (now - startedAt) / duration);
      const eased = 1 - (1 - progress) ** 4;
      const height = Math.round(start + (target - start) * eased);
      void appWindow.setSize(new LogicalSize(420, height)).catch((error: unknown) => {
        console.error('Failed to resize MainWindow:', error);
      });
      if (progress < 1) {
        animationFrame = window.requestAnimationFrame(tick);
      } else {
        animationFrame = null;
      }
    };
    animationFrame = window.requestAnimationFrame(tick);
  };

  const schedule = (): void => {
    if (scheduledFrame !== null) window.cancelAnimationFrame(scheduledFrame);
    scheduledFrame = window.requestAnimationFrame(() => {
      scheduledFrame = null;
      animateTo(measureTarget());
    });
  };

  const resizeObserver = new ResizeObserver(schedule);
  resizeObserver.observe(content);
  const mutationObserver = new MutationObserver(schedule);
  mutationObserver.observe(content, {
    subtree: true,
    childList: true,
    attributes: true,
    attributeFilter: ['class', 'hidden'],
  });
  schedule();

  return {
    schedule,
    dispose: () => {
      resizeObserver.disconnect();
      mutationObserver.disconnect();
      if (scheduledFrame !== null) window.cancelAnimationFrame(scheduledFrame);
      if (animationFrame !== null) window.cancelAnimationFrame(animationFrame);
    },
  };
}

function renderBootstrapError(container: HTMLElement, error: unknown): void {
  console.error('Failed to initialize DictatingMe UI:', error);
  const panel = document.createElement('section');
  panel.className = 'state-panel state-panel--error bootstrap-error';
  panel.setAttribute('role', 'alert');
  const title = document.createElement('h1');
  title.textContent = 'DictatingMe 无法启动';
  const detail = document.createElement('p');
  detail.textContent = '界面初始化失败。请重新打开窗口；如果问题持续，请重启应用。';
  panel.append(title, detail);
  container.replaceChildren(panel);
}

export { Router, HomePage, InputDevicePage, EvokeWordPage, SpeechModelPage, HistoryPage };
