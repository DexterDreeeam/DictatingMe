/**
 * InputDevice 二级页：设备单选列表 + 实时音量/波形反馈，见 brainstrom/plan.md §6.2。
 */

import type { PageComponent } from '../page';
import { getConfig, listDevices, setInputDevice } from '../../shared/api';
import { onInputLevel, type Unlisten } from '../../shared/events';
import type { AudioDeviceInfo } from '../../shared/types';
import { renderDeviceItem } from '../components/device-item';

export class InputDevicePage implements PageComponent {
  #generation = 0;
  #selectionRequest = 0;
  #devices: AudioDeviceInfo[] = [];
  #selectedId = '';
  #list: HTMLElement | null = null;
  #status: HTMLElement | null = null;
  #waveform: HTMLElement | null = null;
  #waveformBars: HTMLElement[] = [];
  #inputLevel = 0;
  #unlistenLevel: Unlisten | null = null;

  mount(container: HTMLElement): void {
    const generation = ++this.#generation;
    container.replaceChildren(this.#renderLoading());
    void this.#subscribeInputLevel(generation);
    void this.#load(container, generation);
  }

  unmount(): void {
    this.#generation += 1;
    this.#selectionRequest += 1;
    this.#devices = [];
    this.#list = null;
    this.#status = null;
    this.#waveform = null;
    this.#waveformBars = [];
    this.#unlistenLevel?.();
    this.#unlistenLevel = null;
  }

  async #load(container: HTMLElement, generation: number): Promise<void> {
    try {
      const [devices, config] = await Promise.all([listDevices(), getConfig()]);
      if (generation !== this.#generation) {
        return;
      }
      this.#devices = devices;
      const selected = devices.find((device) => device.id === config.inputDeviceId)
        ?? devices.find((device) => device.isDefault)
        ?? devices[0];
      this.#selectedId = selected?.id ?? '';
      container.replaceChildren(this.#renderPage());
      if (selected && selected.id !== config.inputDeviceId) {
        void this.#persistInitialDevice(selected.id, generation);
      }
    } catch (error) {
      if (generation !== this.#generation) {
        return;
      }
      container.replaceChildren(this.#renderError(container));
      console.error('Failed to load input devices:', error);
    }
  }

  #renderPage(): HTMLElement {
    const page = this.#createPageShell();
    this.#status = document.createElement('p');
    this.#status.className = 'inline-status';
    this.#status.setAttribute('role', 'alert');

    this.#list = document.createElement('div');
    this.#list.className = 'device-list';
    this.#list.setAttribute('role', 'radiogroup');
    this.#list.setAttribute('aria-label', '输入设备');
    this.#renderDeviceList();

    page.append(this.#list, this.#status);
    return page;
  }

  #renderDeviceList(): void {
    if (!this.#list) {
      return;
    }
    this.#list.replaceChildren();
    if (this.#devices.length === 0) {
      const empty = document.createElement('div');
      empty.className = 'state-panel state-panel--compact';
      const title = document.createElement('h3');
      title.textContent = '未检测到输入设备';
      const detail = document.createElement('p');
      detail.textContent = '连接麦克风或检查系统录音权限后，再重新打开此页面。';
      empty.append(title, detail);
      this.#list.append(empty);
      return;
    }

    for (const device of this.#devices) {
      this.#list.append(renderDeviceItem({
        device,
        selected: device.id === this.#selectedId,
        onSelect: (deviceId) => void this.#selectDevice(deviceId),
      }));
    }
    this.#waveform = this.#list.querySelector<HTMLElement>('.device-item--selected .device-item__meter');
    this.#waveformBars = Array.from(
      this.#waveform?.querySelectorAll<HTMLElement>('span') ?? [],
    );
    this.#applyInputLevel(this.#inputLevel);
  }

  async #selectDevice(deviceId: string): Promise<void> {
    if (deviceId === this.#selectedId) {
      return;
    }
    const previousId = this.#selectedId;
    const generation = this.#generation;
    const request = ++this.#selectionRequest;
    this.#selectedId = deviceId;
    this.#renderDeviceList();
    this.#setStatus('');

    try {
      const config = await setInputDevice(deviceId);
      if (generation !== this.#generation || request !== this.#selectionRequest) {
        return;
      }
      this.#selectedId = config.inputDeviceId;
      this.#renderDeviceList();
      this.#setStatus('');
    } catch (error) {
      if (generation !== this.#generation || request !== this.#selectionRequest) {
        return;
      }
      this.#selectedId = previousId;
      this.#renderDeviceList();
      this.#setStatus('无法切换设备，请检查设备连接后重试。', 'error');
      console.error('Failed to set input device:', error);
    }
  }

  async #persistInitialDevice(deviceId: string, generation: number): Promise<void> {
    const request = this.#selectionRequest;
    try {
      const config = await setInputDevice(deviceId);
      if (generation !== this.#generation || request !== this.#selectionRequest) return;
      const selected = this.#devices.find((device) => device.id === config.inputDeviceId)
        ?? this.#devices.find((device) => device.isDefault)
        ?? this.#devices[0];
      this.#selectedId = selected?.id ?? '';
      this.#renderDeviceList();
    } catch (error) {
      if (generation === this.#generation && request === this.#selectionRequest) {
        this.#setStatus('默认设备已选中，但无法保存该选择。', 'error');
        console.error('Failed to persist default input device:', error);
      }
    }
  }

  async #subscribeInputLevel(generation: number): Promise<void> {
    try {
      const cleanup = await onInputLevel((level) => {
        if (generation === this.#generation) {
          this.#applyInputLevel(level);
        }
      });
      if (generation !== this.#generation) {
        cleanup();
      } else {
        this.#unlistenLevel?.();
        this.#unlistenLevel = cleanup;
      }
    } catch (error) {
      if (generation === this.#generation) {
        console.error('Failed to subscribe to input level:', error);
      }
    }
  }

  #applyInputLevel(value: number): void {
    const level = Number.isFinite(value) ? Math.min(1, Math.max(0, value)) : 0;
    this.#inputLevel = level;
    this.#waveform?.setAttribute('aria-label', `实时音频电平 ${Math.round(level * 100)}%`);
    const noiseFloor = 0.003;
    const normalizedLevel = Math.max(0, (level - noiseFloor) / (1 - noiseFloor));
    const displayLevel = normalizedLevel ** 0.22;
    const center = (this.#waveformBars.length - 1) / 2;
    this.#waveformBars.forEach((bar, index) => {
      const distance = Math.abs(index - center) / Math.max(1, center);
      const shape = 0.45 + (1 - distance) * 0.55;
      bar.style.height = `${4 + displayLevel * shape * 12}px`;
      bar.style.opacity = `${0.28 + displayLevel * 0.72}`;
    });
  }

  #createPageShell(): HTMLElement {
    const page = document.createElement('section');
    page.className = 'page page--devices';
    return page;
  }

  #renderLoading(): HTMLElement {
    const page = this.#createPageShell();
    const list = document.createElement('div');
    list.className = 'device-list';
    list.setAttribute('aria-busy', 'true');
    for (let index = 0; index < 3; index += 1) {
      const skeleton = document.createElement('div');
      skeleton.className = 'skeleton skeleton--row';
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
    title.textContent = '无法读取麦克风';
    const detail = document.createElement('p');
    detail.textContent = '检查系统麦克风权限和设备连接，然后重试。';
    const retry = document.createElement('button');
    retry.type = 'button';
    retry.className = 'button button--primary';
    retry.textContent = '重新检测';
    retry.addEventListener('click', () => {
      const generation = ++this.#generation;
      container.replaceChildren(this.#renderLoading());
      void this.#load(container, generation);
    });
    state.append(title, detail, retry);
    page.append(state);
    return page;
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
