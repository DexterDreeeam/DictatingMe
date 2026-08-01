import type { PageComponent } from '../page';
import {
  beginEvokeSetup,
  cancelEvokeSetup,
  captureEvokeSample,
  finishEvokeSetup,
  getOperation,
  getSettingsSnapshot,
  listDevices,
  setSensitivity,
} from '../../shared/api';
import {
  onEvokeScore,
  onInputLevel,
  onOperationProgress,
  type Unlisten,
} from '../../shared/events';
import type {
  AssetGroup,
  EvokeMode,
  EvokeScore,
  EvokeSetupSession,
  OperationProgress,
  SettingsSnapshot,
} from '../../shared/types';
import { createDownloadButton } from '../components/download-button';

const MODES: Array<{
  mode: EvokeMode;
  title: string;
  description: string;
  assetGroup?: AssetGroup;
  hidden?: boolean;
}> = [
  { mode: 'text', title: '通用文字', description: '输入唤醒文字' },
  { mode: 'voiceMatch', title: '语音匹配', description: '录制唤醒词，使用语音比对匹配' },
  {
    mode: 'speakerVerify',
    title: '声纹验证',
    description: '录制唤醒词，验证说话人身份',
    assetGroup: 'speakerRecognition',
  },
  {
    mode: 'classifier',
    title: '分类训练',
    description: '录制唤醒词，训练小型分类模型',
    assetGroup: 'classifierRecognition',
    hidden: true,
  },
];

const MODE_ICON_MARKUP: Record<EvokeMode, string> = {
  text: `
    <svg viewBox="0 0 24 24">
      <text class="evoke-mode-icon__glyph" x="12" y="17" text-anchor="middle">A</text>
    </svg>
  `,
  voiceMatch: `
    <svg viewBox="0 0 24 24">
      <path d="M3 12h2l1.5-4 2.5 8 2-11 2.5 14 2-9 1.5 4h4"></path>
    </svg>
  `,
  speakerVerify: `
    <svg viewBox="0 0 24 24">
      <circle cx="9" cy="8" r="3"></circle>
      <path d="M3.5 18c.8-3.1 2.6-4.7 5.5-4.7 2.1 0 3.7.8 4.7 2.4M16 7.5c1.5 1 2.3 2.5 2.3 4.5s-.8 3.5-2.3 4.5M18.7 5c2.1 1.7 3.1 4 3.1 7s-1 5.3-3.1 7"></path>
    </svg>
  `,
  classifier: `
    <svg viewBox="0 0 24 24">
      <path d="M4 12h5c3 0 3-6 7-6h1M9 12c3 0 3 6 7 6h1"></path>
      <circle cx="19" cy="6" r="2"></circle>
      <circle cx="19" cy="18" r="2"></circle>
    </svg>
  `,
};

export class EvokeWordPage implements PageComponent {
  #generation = 0;
  #snapshot: SettingsSnapshot | null = null;
  #scoreUnlisten: Unlisten | null = null;
  #operationUnlisten: Unlisten | null = null;
  #levelUnlisten: Unlisten | null = null;
  #setup: EvokeSetupSession | null = null;
  #selectedMode: EvokeMode = 'text';
  #recording = false;
  #finishing = false;
  #countdownTimer: number | null = null;
  #operationPollTimer: number | null = null;
  #lastScoreLogAt = 0;
  #scoreAnimationFrame: number | null = null;
  #scoreAnimationTime = 0;
  #displayedScore: EvokeScore | null = null;
  #targetScore: EvokeScore | null = null;

  mount(container: HTMLElement): void {
    const generation = ++this.#generation;
    container.replaceChildren(this.#loading());
    void this.#load(container, generation);
  }

  unmount(): void {
    this.#generation += 1;
    this.#scoreUnlisten?.();
    this.#operationUnlisten?.();
    this.#levelUnlisten?.();
    this.#scoreUnlisten = null;
    this.#operationUnlisten = null;
    this.#levelUnlisten = null;
    if (this.#countdownTimer !== null) window.clearInterval(this.#countdownTimer);
    this.#stopOperationPolling();
    this.#stopScoreAnimation();
    this.#setRecordingLock(false);
    this.#recording = false;
    this.#finishing = false;
    this.#setup = null;
  }

  async #load(container: HTMLElement, generation: number): Promise<void> {
    try {
      const [snapshot, devices] = await Promise.all([getSettingsSnapshot(), listDevices()]);
      if (generation !== this.#generation) return;
      this.#snapshot = snapshot;
      this.#selectedMode = snapshot.activeEvoke?.mode ?? 'text';
      const selectedDevice = devices.find((device) => device.id === snapshot.config.inputDeviceId)
        ?? devices.find((device) => device.isDefault);
      container.replaceChildren(this.#dashboard(container, selectedDevice?.name ?? '未检测到输入设备'));
      this.#scoreUnlisten = await onEvokeScore((score) => this.#updateScore(container, score));
      this.#levelUnlisten = await onInputLevel((level) => {
        const bars = Array.from(container.querySelectorAll<HTMLElement>('.recording-level span'));
        const visible = Math.max(0, (level - 0.003) / 0.997) ** 0.22;
        const center = (bars.length - 1) / 2;
        bars.forEach((bar, index) => {
          const distance = Math.abs(index - center) / Math.max(1, center);
          bar.style.height = `${3 + visible * (0.4 + (1 - distance) * 0.6) * 14}px`;
          bar.style.opacity = String(0.25 + visible * 0.75);
        });
      });
    } catch (error) {
      console.error('Failed to load evoke settings:', error);
      if (generation === this.#generation) container.replaceChildren(this.#error(container));
    }
  }

  #dashboard(container: HTMLElement, deviceName: string): HTMLElement {
    const page = this.#shell();
    const panel = document.createElement('div');
    panel.className = 'evoke-dashboard';
    const current = document.createElement('div');
    current.className = 'evoke-current';
    const currentLabel = document.createElement('span');
    currentLabel.textContent = '唤醒词';
    const currentSetting = document.createElement('span');
    currentSetting.className = 'evoke-current__setting';
    const activeEvoke = this.#snapshot?.activeEvoke;
    if (activeEvoke) currentSetting.append(createModeIcon(activeEvoke.mode, true));
    const currentPhrase = document.createElement('strong');
    currentPhrase.textContent = activeEvoke?.phrase ?? '尚未设置';
    currentSetting.append(currentPhrase);
    current.append(currentLabel, currentSetting);

    const sensitivityGroup = document.createElement('div');
    sensitivityGroup.className = 'metric-group';
    const sensitivityHead = document.createElement('div');
    sensitivityHead.className = 'metric-head';
    sensitivityHead.innerHTML = '<span>灵敏度</span>';
    const sensitivityValue = document.createElement('output');
    const sensitivity = this.#snapshot?.config.sensitivity ?? 0.65;
    sensitivityValue.textContent = `${Math.round(sensitivity * 100)}%`;
    sensitivityHead.append(sensitivityValue);
    const range = document.createElement('input');
    range.className = 'sensitivity-slider';
    range.type = 'range';
    range.min = '0';
    range.max = '1';
    range.step = '0.01';
    range.value = String(sensitivity);
    range.addEventListener('input', () => {
      sensitivityValue.textContent = `${Math.round(Number(range.value) * 100)}%`;
    });
    let pointerId: number | null = null;
    let saving = false;
    let pendingValue: number | null = null;
    let lastSavedValue = sensitivity;
    let lastSubmittedValue = sensitivity;
    const savePendingValue = async (): Promise<void> => {
      if (saving) return;
      saving = true;
      try {
        while (pendingValue !== null) {
          const value = pendingValue;
          pendingValue = null;
          try {
            const snapshot = await setSensitivity(value);
            this.#snapshot = snapshot;
            lastSavedValue = value;
          } catch (error) {
            console.error('Failed to set sensitivity:', error);
            if (pendingValue === null) lastSubmittedValue = lastSavedValue;
          }
        }
      } finally {
        saving = false;
      }
    };
    const queueSensitivitySave = (): void => {
      const value = Number(range.value);
      if (value === lastSubmittedValue) return;
      lastSubmittedValue = value;
      pendingValue = value;
      void savePendingValue();
    };
    range.addEventListener('pointerdown', (event) => {
      pointerId = event.pointerId;
      range.setPointerCapture(event.pointerId);
    });
    range.addEventListener('pointerup', (event) => {
      if (range.hasPointerCapture(event.pointerId)) range.releasePointerCapture(event.pointerId);
      pointerId = null;
      queueSensitivitySave();
    });
    range.addEventListener('pointercancel', (event) => {
      if (range.hasPointerCapture(event.pointerId)) range.releasePointerCapture(event.pointerId);
      pointerId = null;
    });
    range.addEventListener('change', () => {
      if (pointerId === null) queueSensitivitySave();
    });
    sensitivityGroup.append(sensitivityHead, range);

    const scoreGroup = document.createElement('div');
    scoreGroup.className = 'metric-group';
    scoreGroup.dataset.role = 'score-group';
    const scoreHead = document.createElement('div');
    scoreHead.className = 'metric-head';
    scoreHead.innerHTML = '<span>识别检测</span>';
    const scoreValue = document.createElement('output');
    scoreValue.dataset.role = 'score-value';
    scoreValue.textContent = '0%';
    scoreHead.append(scoreValue);
    const meter = document.createElement('div');
    meter.className = 'score-meter';
    meter.setAttribute('role', 'meter');
    const fill = document.createElement('span');
    fill.className = 'score-meter__fill';
    fill.dataset.role = 'score-fill';
    const marker = document.createElement('span');
    marker.className = 'score-meter__threshold';
    marker.dataset.role = 'score-threshold';
    meter.append(fill, marker);
    const breakdown = document.createElement('div');
    breakdown.className = 'score-breakdown';
    breakdown.dataset.role = 'score-breakdown';
    breakdown.textContent = '请说出唤醒词测试当前配置';
    const device = document.createElement('div');
    device.className = 'score-device';
    device.textContent = `输入设备：${deviceName}`;
    scoreGroup.append(scoreHead, meter, breakdown, device);

    const setup = document.createElement('button');
    setup.className = 'button button--primary button--wide';
    setup.textContent = '设置唤醒词';
    setup.addEventListener('click', () => {
      this.#scoreUnlisten?.();
      this.#scoreUnlisten = null;
      this.#stopScoreAnimation();
      container.replaceChildren(this.#modeSelection(container));
    });
    panel.append(current, sensitivityGroup, scoreGroup, setup);
    page.append(panel);
    return page;
  }

  #modeSelection(container: HTMLElement): HTMLElement {
    const page = this.#shell();
    const heading = document.createElement('h2');
    heading.className = 'section-heading';
    heading.textContent = '选择唤醒模式';
    const list = document.createElement('div');
    list.className = 'mode-list';
    for (const mode of MODES) {
      if (mode.hidden) continue;
      const assets = mode.assetGroup
        ? this.#snapshot?.assets.filter((item) => item.assetGroup === mode.assetGroup) ?? []
        : [];
      const ready = !mode.assetGroup
        || (assets.length > 0 && assets.every((asset) => asset.phase === 'ready'));
      const option = document.createElement('label');
      option.className = 'mode-option';
      option.classList.toggle('mode-option--selected', this.#selectedMode === mode.mode);
      option.classList.toggle('mode-option--unavailable', !ready);
      const radio = document.createElement('input');
      radio.type = 'radio';
      radio.name = 'evoke-mode';
      radio.value = mode.mode;
      radio.checked = this.#selectedMode === mode.mode;
      radio.disabled = !ready;
      radio.addEventListener('change', () => {
        if (!radio.checked) return;
        this.#selectedMode = mode.mode;
        list.querySelectorAll('.mode-option').forEach((item) => {
          item.classList.toggle('mode-option--selected', item === option);
        });
      });
      const marker = document.createElement('span');
      marker.className = 'mode-option__radio';
      const copy = document.createElement('span');
      copy.className = 'mode-option__copy';
      copy.innerHTML = `<strong>${mode.title}</strong><span>${mode.description}</span>`;
      option.append(radio, marker, copy);
      if (assets.length > 0) {
        const assetControls = document.createElement('span');
        assetControls.className = 'mode-option__assets';
        for (const asset of assets) {
          const download = createDownloadButton(asset.sources, asset.assetPath);
          download.title = asset.displayName;
          download.addEventListener('dictatingme:asset-ready', () => {
            asset.phase = 'ready';
            const allReady = assets.every((item) => item.phase === 'ready');
            radio.disabled = !allReady;
            option.classList.toggle('mode-option--unavailable', !allReady);
          });
          assetControls.append(download);
        }
        option.append(assetControls);
      }
      option.append(createModeIcon(mode.mode));
      list.append(option);
    }
    const actions = actionStack(
      '下一步',
      () => container.replaceChildren(this.#setupPage(container)),
      '返回',
      () => this.#remount(container),
    );
    page.append(heading, list, actions);
    return page;
  }

  #setupPage(container: HTMLElement): HTMLElement {
    const page = this.#shell();
    page.classList.add('page--evoke-setup');
    const phrase = document.createElement('input');
    phrase.className = 'text-input';
    phrase.type = 'text';
    phrase.maxLength = 32;
    phrase.value = this.#snapshot?.activeEvoke?.phrase ?? '你好';
    phrase.placeholder = '输入唤醒文字';
    const total = MODES.find((item) => item.mode === this.#selectedMode)
      ? recordingCount(this.#selectedMode)
      : 0;
    const module = document.createElement('div');
    module.className = 'recording-module';
    module.hidden = total === 0;
    const head = document.createElement('div');
    head.className = 'recording-head';
    head.innerHTML = '<strong>录制唤醒词</strong>';
    const steps = document.createElement('span');
    steps.className = 'recording-steps';
    steps.dataset.role = 'recording-steps';
    head.append(steps);
    const level = document.createElement('div');
    level.className = 'recording-level';
    for (let index = 0; index < 12; index += 1) level.append(document.createElement('span'));
    const record = document.createElement('button');
    record.className = 'button button--record button--wide';
    record.textContent = '开始录制';
    const tip = document.createElement('p');
    tip.className = 'action-tip';
    tip.dataset.role = 'recording-tip';
    tip.textContent = '正常音量说出唤醒词';
    record.addEventListener('click', () => void this.#record(container, phrase, record, tip, steps, total));
    module.append(head, level, record, tip);
    this.#renderSteps(steps, total, 0, false);

    const completeWrap = document.createElement('div');
    completeWrap.className = 'action-stack';
    const complete = document.createElement('button');
    complete.className = 'button button--primary button--wide';
    complete.textContent = '完成设置';
    complete.hidden = total > 0;
    complete.dataset.role = 'complete-setup';
    complete.addEventListener('click', () => void this.#finish(container, phrase));
    const estimate = document.createElement('p');
    estimate.className = 'action-tip';
    estimate.textContent = this.#selectedMode === 'classifier'
      ? '预计需要约 1–5 分钟处理数据'
      : '';
    estimate.hidden = this.#selectedMode !== 'classifier';
    const back = document.createElement('button');
    back.className = 'button button--secondary button--wide';
    back.textContent = '返回';
    back.addEventListener('click', () => {
      if (this.#recording) return;
      if (this.#setup) void cancelEvokeSetup(this.#setup.id);
      this.#setup = null;
      container.replaceChildren(this.#modeSelection(container));
    });
    completeWrap.append(complete, estimate, back);
    page.append(phrase, module, completeWrap);
    return page;
  }

  async #ensureSetup(phrase: HTMLInputElement): Promise<EvokeSetupSession> {
    const value = phrase.value.trim();
    if (!value) {
      phrase.focus();
      throw new Error('请输入唤醒文字。');
    }
    if (!this.#setup) {
      this.#setup = await beginEvokeSetup({ mode: this.#selectedMode, phrase: value });
    }
    phrase.disabled = true;
    return this.#setup;
  }

  async #record(
    container: HTMLElement,
    phrase: HTMLInputElement,
    button: HTMLButtonElement,
    tip: HTMLElement,
    steps: HTMLElement,
    total: number,
  ): Promise<void> {
    if (this.#recording) return;
    const generation = this.#generation;
    this.#recording = true;
    this.#setRecordingLock(true);
    const back = container.querySelector<HTMLButtonElement>('.button--secondary');
    if (back) back.disabled = true;
    phrase.disabled = true;
    button.disabled = true;
    tip.hidden = true;
    try {
      const setup = await this.#ensureSetup(phrase);
      if (generation !== this.#generation) return;
      this.#renderSteps(steps, total, setup.completedRecordings, true);
      const started = performance.now();
      const update = (): void => {
        button.textContent = `${Math.max(0, (5000 - (performance.now() - started)) / 1000).toFixed(1)} 秒`;
      };
      update();
      this.#countdownTimer = window.setInterval(update, 50);
      const receipt = await captureEvokeSample(setup.id);
      if (generation !== this.#generation) return;
      this.#setup = { ...setup, completedRecordings: receipt.completedRecordings };
      if (!receipt.quality.accepted) {
        tip.textContent = receipt.quality.rejection ?? '录音质量不足，请重试';
      } else {
        tip.textContent = setup.plan.prompts[receipt.completedRecordings]?.text ?? '';
      }
      this.#renderSteps(steps, total, receipt.completedRecordings, false);
      const complete = container.querySelector<HTMLButtonElement>('[data-role="complete-setup"]');
      if (receipt.completedRecordings >= total) {
        button.textContent = '录制完成';
        button.disabled = true;
        if (complete) complete.hidden = false;
      } else {
        button.textContent = '开始录制';
        button.disabled = false;
      }
    } catch (error) {
      console.error('Failed to record evoke sample:', error);
      tip.textContent = error instanceof Error ? error.message : '录制失败，请重试';
      button.textContent = '开始录制';
      button.disabled = false;
    } finally {
      if (this.#countdownTimer !== null) window.clearInterval(this.#countdownTimer);
      this.#countdownTimer = null;
      this.#recording = false;
      this.#setRecordingLock(false);
      phrase.disabled = this.#setup !== null;
      const back = container.querySelector<HTMLButtonElement>('.button--secondary');
      if (back) back.disabled = false;
      tip.hidden = false;
    }
  }

  async #finish(container: HTMLElement, phrase: HTMLInputElement): Promise<void> {
    if (this.#finishing) return;
    this.#finishing = true;
    this.#setRecordingLock(true);
    const generation = this.#generation;
    try {
      const setup = await this.#ensureSetup(phrase);
      container.replaceChildren(this.#processing());
      this.#operationUnlisten?.();
      let operationId: string | null = null;
      this.#operationUnlisten = await onOperationProgress((operation) => {
        if (generation !== this.#generation || operation.operationId !== operationId) return;
        this.#updateProcessing(container, operation, generation);
      });
      if (generation !== this.#generation) {
        this.#operationUnlisten();
        this.#operationUnlisten = null;
        return;
      }
      operationId = await finishEvokeSetup(setup.id);
      if (generation !== this.#generation) return;
      this.#startOperationPolling(container, operationId, generation);
    } catch (error) {
      console.error('Failed to finish evoke setup:', error);
      if (generation === this.#generation) {
        this.#finishing = false;
        this.#setRecordingLock(false);
      }
    }
  }

  #processing(): HTMLElement {
    const page = this.#shell();
    const panel = document.createElement('div');
    panel.className = 'processing-panel';
    panel.innerHTML = '<span class="processing-spinner"></span><strong data-role="processing-title">正在处理用户语音数据</strong><p data-role="processing-message">请稍候，不要关闭程序</p>';
    page.append(panel);
    return page;
  }

  #updateProcessing(
    container: HTMLElement,
    operation: OperationProgress,
    generation: number,
  ): void {
    if (generation !== this.#generation) return;
    const terminal = operation.phase === 'completed'
      || operation.phase === 'failed'
      || operation.phase === 'cancelled';
    if (terminal && !this.#finishing) return;
    const title = container.querySelector<HTMLElement>('[data-role="processing-title"]');
    const message = container.querySelector<HTMLElement>('[data-role="processing-message"]');
    if (message) message.textContent = operation.message ?? '';
    if (operation.phase === 'completed') {
      this.#finishing = false;
      this.#stopOperationPolling();
      this.#operationUnlisten?.();
      this.#operationUnlisten = null;
      if (title) title.textContent = '设置完成';
      window.setTimeout(() => {
        if (generation === this.#generation) this.#remount(container);
      }, 650);
    } else if (operation.phase === 'failed' || operation.phase === 'cancelled') {
      this.#stopOperationPolling();
      this.#operationUnlisten?.();
      this.#operationUnlisten = null;
      if (title) title.textContent = '设置失败';
      if (message) message.textContent = operation.error ?? '请返回重试';
      this.#finishing = false;
      this.#setRecordingLock(false);
    }
  }

  #startOperationPolling(
    container: HTMLElement,
    operationId: string,
    generation: number,
  ): void {
    this.#stopOperationPolling();
    const poll = async (): Promise<void> => {
      if (generation !== this.#generation || !this.#finishing) return;
      try {
        const operation = await getOperation(operationId);
        this.#updateProcessing(container, operation, generation);
      } catch (error) {
        console.error('Failed to refresh evoke processing operation:', error);
      }
      if (generation === this.#generation && this.#finishing) {
        this.#operationPollTimer = window.setTimeout(() => void poll(), 400);
      }
    };
    void poll();
  }

  #stopOperationPolling(): void {
    if (this.#operationPollTimer !== null) window.clearTimeout(this.#operationPollTimer);
    this.#operationPollTimer = null;
  }

  #updateScore(container: HTMLElement, score: EvokeScore): void {
    const now = performance.now();
    if (now - this.#lastScoreLogAt >= 1_000) {
      this.#lastScoreLogAt = now;
      console.debug('[evoke-score] received preview', {
        overall: score.overall,
        threshold: score.threshold,
        voiceActivity: score.voiceActivity,
        phraseScore: score.phraseScore,
        modeScore: score.modeScore,
        accepted: score.accepted,
        mode: score.mode,
      });
    }
    this.#targetScore = score;
    if (this.#displayedScore === null || this.#displayedScore.mode !== score.mode) {
      this.#displayedScore = {
        ...score,
        overall: 0,
        voiceActivity: 0,
        phraseScore: 0,
        modeScore: 0,
        accepted: false,
      };
      this.#scoreAnimationTime = performance.now();
    }
    if (this.#scoreAnimationFrame !== null) return;
    const animate = (now: number): void => {
      const target = this.#targetScore;
      const current = this.#displayedScore;
      if (!target || !current) {
        this.#scoreAnimationFrame = null;
        return;
      }
      const elapsed = Math.min(50, Math.max(0, now - this.#scoreAnimationTime));
      this.#scoreAnimationTime = now;
      const alpha = 1 - Math.exp(-elapsed / 115);
      const next: EvokeScore = {
        overall: interpolateScore(current.overall, target.overall, alpha),
        threshold: interpolateScore(current.threshold, target.threshold, alpha),
        voiceActivity: interpolateScore(current.voiceActivity, target.voiceActivity, alpha),
        phraseScore: interpolateScore(current.phraseScore, target.phraseScore, alpha),
        modeScore: interpolateScore(current.modeScore, target.modeScore, alpha),
        accepted: target.accepted,
        mode: target.mode,
      };
      this.#displayedScore = next;
      this.#renderScore(container, next);
      if (scoreDistance(next, target) < 0.001) {
        this.#displayedScore = { ...target };
        this.#renderScore(container, target);
        this.#scoreAnimationFrame = null;
        return;
      }
      this.#scoreAnimationFrame = window.requestAnimationFrame(animate);
    };
    this.#scoreAnimationFrame = window.requestAnimationFrame(animate);
  }

  #renderScore(container: HTMLElement, score: EvokeScore): void {
    const value = container.querySelector<HTMLElement>('[data-role="score-value"]');
    const fill = container.querySelector<HTMLElement>('[data-role="score-fill"]');
    const meter = container.querySelector<HTMLElement>('.score-meter');
    const threshold = container.querySelector<HTMLElement>('[data-role="score-threshold"]');
    const breakdown = container.querySelector<HTMLElement>('[data-role="score-breakdown"]');
    const percentage = Math.round(score.overall * 100);
    if (value) value.textContent = `${percentage}%`;
    if (fill) fill.style.width = `${score.overall * 100}%`;
    if (meter) meter.setAttribute('aria-valuenow', String(percentage));
    if (threshold) threshold.style.left = `${score.threshold * 100}%`;
    if (breakdown) {
      breakdown.textContent = `语音 ${Math.round(score.voiceActivity * 100)} · 文字 ${Math.round(score.phraseScore * 100)} · 模式 ${Math.round(score.modeScore * 100)}`;
      breakdown.dataset.accepted = String(score.accepted);
    }
  }

  #stopScoreAnimation(): void {
    if (this.#scoreAnimationFrame !== null) {
      window.cancelAnimationFrame(this.#scoreAnimationFrame);
    }
    this.#scoreAnimationFrame = null;
    this.#displayedScore = null;
    this.#targetScore = null;
    this.#scoreAnimationTime = 0;
  }

  #renderSteps(target: HTMLElement, total: number, completed: number, recording: boolean): void {
    target.replaceChildren();
    for (let index = 0; index < total; index += 1) {
      const step = document.createElement('span');
      step.className = 'recording-step';
      if (index < completed) step.classList.add('recording-step--done');
      else if (index === completed) {
        step.classList.add(recording ? 'recording-step--active' : 'recording-step--next');
      }
      target.append(step);
    }
  }

  #setRecordingLock(locked: boolean): void {
    window.dispatchEvent(new CustomEvent('dictatingme:recording-lock', { detail: locked }));
  }

  #shell(): HTMLElement {
    const page = document.createElement('section');
    page.className = 'page page--evoke';
    return page;
  }

  #loading(): HTMLElement {
    const page = this.#shell();
    const skeleton = document.createElement('div');
    skeleton.className = 'skeleton skeleton--form';
    page.append(skeleton);
    return page;
  }

  #error(container: HTMLElement): HTMLElement {
    const page = this.#shell();
    const panel = document.createElement('div');
    panel.className = 'state-panel state-panel--error';
    panel.innerHTML = '<h2>无法读取唤醒设置</h2><p>请稍后重试。</p>';
    const retry = document.createElement('button');
    retry.className = 'button button--primary';
    retry.textContent = '重新加载';
    retry.addEventListener('click', () => this.#remount(container));
    panel.append(retry);
    page.append(panel);
    return page;
  }

  #remount(container: HTMLElement): void {
    this.unmount();
    this.mount(container);
  }
}

function createModeIcon(mode: EvokeMode, current = false): HTMLSpanElement {
  const icon = document.createElement('span');
  icon.className = `evoke-mode-icon${current ? ' evoke-mode-icon--current' : ''}`;
  icon.innerHTML = MODE_ICON_MARKUP[mode];
  icon.title = MODES.find((item) => item.mode === mode)?.title ?? '';
  icon.setAttribute('aria-hidden', 'true');
  return icon;
}

function interpolateScore(current: number, target: number, alpha: number): number {
  const value = current + (target - current) * alpha;
  return Math.abs(target - value) < 0.0005 ? target : value;
}

function scoreDistance(left: EvokeScore, right: EvokeScore): number {
  return Math.max(
    Math.abs(left.overall - right.overall),
    Math.abs(left.threshold - right.threshold),
    Math.abs(left.voiceActivity - right.voiceActivity),
    Math.abs(left.phraseScore - right.phraseScore),
    Math.abs(left.modeScore - right.modeScore),
  );
}

function recordingCount(mode: EvokeMode): number {
  if (mode === 'text') return 0;
  if (mode === 'classifier') return 6;
  return 3;
}

function actionStack(
  primaryText: string,
  primaryAction: () => void,
  secondaryText: string,
  secondaryAction: () => void,
): HTMLElement {
  const actions = document.createElement('div');
  actions.className = 'action-stack';
  const primary = document.createElement('button');
  primary.className = 'button button--primary button--wide';
  primary.textContent = primaryText;
  primary.addEventListener('click', primaryAction);
  const secondary = document.createElement('button');
  secondary.className = 'button button--secondary button--wide';
  secondary.textContent = secondaryText;
  secondary.addEventListener('click', secondaryAction);
  actions.append(primary, secondary);
  return actions;
}
