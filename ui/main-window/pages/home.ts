/**
 * 首页：三张导航卡片（InputDevice / EvokeWord / History），见 brainstrom/plan.md §6.1。
 */

import type { PageComponent } from '../page';
import type { RouteName } from '../router';
import { getSettingsSnapshot, listDevices, listHistory } from '../../shared/api';
import { onHistoryUpdated, onSettingsChanged, type Unlisten } from '../../shared/events';
import { HISTORY_CAPACITY } from '../../shared/types';
import { renderNavCard } from '../components/nav-card';

export interface HomePageDeps {
  /** 卡片点击后请求跳转到目标二级页 */
  onNavigate: (route: RouteName) => void;
}

export class HomePage implements PageComponent {
  #generation = 0;
  #unlisten: Unlisten | null = null;
  #unlistenSettings: Unlisten | null = null;
  #historyCount = 0;
  #historySubtitle: HTMLElement | null = null;

  constructor(private readonly deps: HomePageDeps) {
  }

  mount(container: HTMLElement): void {
    const generation = ++this.#generation;
    container.replaceChildren(this.#renderLoading());
    void this.#load(container, generation);
    void this.#subscribe(container, generation);
  }

  unmount(): void {
    this.#generation += 1;
    this.#unlisten?.();
    this.#unlisten = null;
    this.#unlistenSettings?.();
    this.#unlistenSettings = null;
    this.#historySubtitle = null;
  }

  async #load(container: HTMLElement, generation: number): Promise<void> {
    try {
      const [snapshot, devices, history] = await Promise.all([
        getSettingsSnapshot(),
        listDevices(),
        listHistory(),
      ]);
      if (generation !== this.#generation) {
        return;
      }

      this.#historyCount = Math.min(history.length, HISTORY_CAPACITY);
      const selectedDevice = devices.find((device) => device.id === snapshot.config.inputDeviceId)
        ?? devices.find((device) => device.isDefault);
      container.replaceChildren(this.#renderPage(
        selectedDevice?.name ?? (devices.length === 0 ? '未检测到输入设备' : '尚未选择'),
        snapshot.activeEvoke?.phrase ?? '尚未设置',
        snapshot.readiness.dictationModelReady
          ? snapshot.assets.find((asset) => asset.selected)?.displayName ?? '尚未选择'
          : '需要设置',
        snapshot.readiness.dictationModelReady,
      ));
    } catch (error) {
      if (generation !== this.#generation) {
        return;
      }
      container.replaceChildren(this.#renderError(container));
      console.error('Failed to load home page data:', error);
    }
  }

  async #subscribe(container: HTMLElement, generation: number): Promise<void> {
    try {
      const unlisten = await onHistoryUpdated(() => {
        if (generation !== this.#generation) {
          return;
        }
        this.#historyCount = Math.min(this.#historyCount + 1, HISTORY_CAPACITY);
        this.#updateHistorySubtitle();
      });
      if (generation !== this.#generation) {
        unlisten();
        return;
      }
      this.#unlisten = unlisten;
      const unlistenSettings = await onSettingsChanged(() => {
        if (generation === this.#generation) {
          void this.#load(container, generation);
        }
      });
      if (generation !== this.#generation) {
        unlistenSettings();
        return;
      }
      this.#unlistenSettings = unlistenSettings;
    } catch (error) {
      if (generation === this.#generation) {
        console.error('Failed to subscribe to history updates:', error);
      }
    }
  }

  #renderLoading(): HTMLElement {
    const page = this.#createPageShell();
    const loading = document.createElement('div');
    loading.className = 'nav-list';
    loading.setAttribute('aria-label', '正在加载设置');
    loading.setAttribute('aria-busy', 'true');
    for (let index = 0; index < 4; index += 1) {
      const skeleton = document.createElement('div');
      skeleton.className = 'skeleton skeleton--card';
      loading.append(skeleton);
    }
    page.append(loading);
    return page;
  }

  #renderPage(
    deviceName: string,
    evokeWord: string,
    speechModel: string,
    speechModelReady: boolean,
  ): HTMLElement {
    const page = this.#createPageShell();
    const navigation = document.createElement('nav');
    navigation.className = 'nav-list';
    navigation.setAttribute('aria-label', '设置与历史');

    const inputCard = renderNavCard({
      icon: 'microphone',
      title: '输入设备',
      subtitle: deviceName,
      onClick: () => this.deps.onNavigate('input-device'),
    });
    const evokeCard = renderNavCard({
      icon: 'bell',
      title: '唤醒词',
      subtitle: evokeWord,
      onClick: () => this.deps.onNavigate('evoke-word'),
    });
    const modelCard = renderNavCard({
      icon: 'wave',
      title: '语音模型',
      subtitle: speechModel,
      onClick: () => this.deps.onNavigate('speech-model'),
    });
    modelCard.classList.toggle('nav-card--required', !speechModelReady);
    const historyCard = renderNavCard({
      icon: 'history',
      title: '历史记录',
      subtitle: `${this.#historyCount} / ${HISTORY_CAPACITY} 条记录`,
      onClick: () => this.deps.onNavigate('history'),
    });
    this.#historySubtitle = historyCard.querySelector<HTMLElement>('.nav-card__subtitle');

    navigation.append(inputCard, evokeCard, modelCard, historyCard);
    page.append(navigation);
    return page;
  }

  #createPageShell(): HTMLElement {
    const page = document.createElement('section');
    page.className = 'page page--home';

    const wordmark = document.createElement('img');
    wordmark.className = 'hero-wordmark';
    wordmark.src = new URL('../../assets/dictating-me-wordmark.png', import.meta.url).href;
    wordmark.alt = 'Dictating Me';
    wordmark.draggable = false;
    page.append(wordmark);
    return page;
  }

  #renderError(container: HTMLElement): HTMLElement {
    const page = this.#createPageShell();
    const state = document.createElement('div');
    state.className = 'state-panel state-panel--error';
    state.setAttribute('role', 'alert');
    const title = document.createElement('h2');
    title.textContent = '无法加载当前设置';
    const detail = document.createElement('p');
    detail.textContent = '请确认 DictatingMe Runtime 正在运行，然后重试。';
    const retry = document.createElement('button');
    retry.type = 'button';
    retry.className = 'button button--primary';
    retry.textContent = '重新加载';
    retry.addEventListener('click', () => {
      const generation = this.#generation;
      container.replaceChildren(this.#renderLoading());
      void this.#load(container, generation);
    });
    state.append(title, detail, retry);
    page.append(state);
    return page;
  }

  #updateHistorySubtitle(): void {
    if (this.#historySubtitle) {
      this.#historySubtitle.textContent = `${this.#historyCount} / ${HISTORY_CAPACITY} 条记录`;
    }
  }
}
