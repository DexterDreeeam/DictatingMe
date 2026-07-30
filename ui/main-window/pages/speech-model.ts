import type { PageComponent } from '../page';
import { getSettingsSnapshot, selectDictationModel } from '../../shared/api';
import type { AssetSummary } from '../../shared/types';
import { createDownloadButton } from '../components/download-button';

export class SpeechModelPage implements PageComponent {
  #generation = 0;

  mount(container: HTMLElement): void {
    const generation = ++this.#generation;
    container.replaceChildren(this.#loading());
    void this.#load(container, generation);
  }

  unmount(): void {
    this.#generation += 1;
  }

  async #load(container: HTMLElement, generation: number): Promise<void> {
    try {
      const snapshot = await getSettingsSnapshot();
      if (generation !== this.#generation) return;
      const models = snapshot.assets.filter((asset) => asset.assetGroup === 'speechModels');
      if (models.length === 0) throw new Error('No dictation model is registered.');
      container.replaceChildren(this.#render(models));
    } catch (error) {
      console.error('Failed to load speech models:', error);
      if (generation === this.#generation) {
        container.replaceChildren(this.#error(container));
      }
    }
  }

  #render(models: AssetSummary[]): HTMLElement {
    const page = document.createElement('section');
    page.className = 'page page--models';
    const heading = document.createElement('h2');
    heading.className = 'section-heading';
    heading.textContent = '选择语音模型';
    const list = document.createElement('div');
    list.className = 'mode-list';
    for (const model of models) {
      const option = document.createElement('label');
      option.className = 'mode-option';
      const radio = document.createElement('input');
      radio.type = 'radio';
      radio.name = 'speech-model';
      radio.value = model.id;
      radio.checked = model.selected;
      radio.disabled = model.phase !== 'ready';
      const marker = document.createElement('span');
      marker.className = 'mode-option__radio';
      const copy = document.createElement('span');
      copy.className = 'mode-option__copy';
      const title = document.createElement('strong');
      title.textContent = model.displayName;
      const detail = document.createElement('span');
      const size = formatModelSize(model.fileSizeBytes);
      const output = formatOutputMode(model.outputMode);
      detail.textContent = [`版本 ${model.version}`, size, output].filter(Boolean).join(' · ');
      copy.append(title, detail);
      const download = createDownloadButton(model.sources, model.assetPath);
      download.addEventListener('dictatingme:asset-ready', () => {
        model.phase = 'ready';
        radio.disabled = false;
        option.classList.remove('mode-option--unavailable');
        void getSettingsSnapshot()
          .then((snapshot) => {
            syncRenderedSelection(list, snapshot.config.activeDictationAssetId);
          })
          .catch((error) => console.error('Failed to sync dictation model selection:', error));
      });
      option.classList.toggle('mode-option--selected', model.selected);
      option.classList.toggle('mode-option--unavailable', model.phase !== 'ready');
      radio.addEventListener('change', () => {
        if (!radio.checked || radio.disabled) return;
        void selectDictationModel(model.id)
          .then(() => selectRenderedModel(list, radio, option))
          .catch((error) => {
            radio.checked = false;
            console.error('Failed to select dictation model:', error);
          });
      });
      option.append(radio, marker, copy, download);
      list.append(option);
    }
    page.append(heading, list);
    return page;
  }

  #loading(): HTMLElement {
    const page = document.createElement('section');
    page.className = 'page';
    const skeleton = document.createElement('div');
    skeleton.className = 'skeleton skeleton--card';
    page.append(skeleton);
    return page;
  }

  #error(container: HTMLElement): HTMLElement {
    const panel = document.createElement('section');
    panel.className = 'page';
    const state = document.createElement('div');
    state.className = 'state-panel state-panel--error';
    state.innerHTML = '<h2>无法读取语音模型</h2><p>请稍后重试。</p>';
    const retry = document.createElement('button');
    retry.className = 'button button--primary';
    retry.textContent = '重新加载';
    retry.addEventListener('click', () => this.mount(container));
    state.append(retry);
    panel.append(state);
    return panel;
  }
}

export function formatModelSize(bytes: number | null): string | null {
  if (bytes === null || !Number.isFinite(bytes) || bytes < 0) return null;
  const gibibyte = 1024 ** 3;
  const divisor = bytes < gibibyte ? 1024 ** 2 : gibibyte;
  const unit = bytes < gibibyte ? 'MB' : 'GB';
  return `${(bytes / divisor).toFixed(2)} ${unit}`;
}

function formatOutputMode(mode: AssetSummary['outputMode']): string | null {
  if (mode === 'streaming') return '实时输出';
  if (mode === 'utterance') return '整句输出';
  return null;
}

function selectRenderedModel(
  list: HTMLElement,
  selectedRadio: HTMLInputElement,
  selectedOption: HTMLElement,
): void {
  list.querySelectorAll<HTMLInputElement>('input[name="speech-model"]').forEach((radio) => {
    radio.checked = radio === selectedRadio;
  });
  list.querySelectorAll('.mode-option').forEach((option) => {
    option.classList.toggle('mode-option--selected', option === selectedOption);
  });
}

function syncRenderedSelection(list: HTMLElement, selectedId: string | null): void {
  list.querySelectorAll<HTMLInputElement>('input[name="speech-model"]').forEach((radio) => {
    const selected = radio.value === selectedId;
    radio.checked = selected;
    radio.closest('.mode-option')?.classList.toggle('mode-option--selected', selected);
  });
}
