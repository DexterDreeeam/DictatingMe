/**
 * HudWindow: state-driven status pill plus microphone-volume rays.
 */

import { getState } from '../shared/api';
import { onInputLevel, onStateChanged } from '../shared/events';
import type { DmState, HudLight } from '../shared/types';

export function mapStateToHudLight(state: DmState): HudLight {
  switch (state) {
    case 'Listening':
      return 'Yellow';
    case 'Loading':
    case 'Dictating':
      return 'Green';
    case 'Configure':
    case 'Unloading':
      return 'Off';
    default: {
      const exhaustive: never = state;
      return exhaustive;
    }
  }
}

class HudRayAnimator {
  #stage: HTMLElement;
  #mode: 'idle' | 'entering' | 'volume' | 'releasing' = 'idle';
  #rawVolume = 0;
  #length = 0;
  #opacity = 0;
  #frame: number | null = null;
  #frameTime = 0;
  #phaseStartedAt = 0;
  #holdUntil = 0;
  #fadeStartedAt: number | null = null;
  #fadeStartOpacity = 1;

  constructor(stage: HTMLElement) {
    this.#stage = stage;
  }

  enterDictating(): void {
    this.#mode = 'entering';
    this.#length = 4;
    this.#opacity = 0;
    this.#phaseStartedAt = performance.now();
    this.#fadeStartedAt = null;
    this.#render();
    this.#start();
  }

  updateVolume(value: number): void {
    this.#rawVolume = Number.isFinite(value) ? Math.min(1, Math.max(0, value)) : 0;
    if (this.#mode === 'volume') this.#start();
  }

  release(seedFromMain: boolean): void {
    if (this.#mode === 'releasing' && !seedFromMain) return;
    if (seedFromMain) {
      this.#length = window.innerWidth * 0.25;
      this.#opacity = 1;
    }
    if (this.#length <= 0.5) {
      this.reset();
      return;
    }
    this.#mode = 'releasing';
    this.#fadeStartedAt = null;
    this.#fadeStartOpacity = this.#opacity || 1;
    this.#start();
  }

  reset(): void {
    if (this.#frame !== null) window.cancelAnimationFrame(this.#frame);
    this.#frame = null;
    this.#mode = 'idle';
    this.#length = 0;
    this.#opacity = 0;
    this.#fadeStartedAt = null;
    this.#render();
  }

  resize(): void {
    if (this.#mode !== 'idle') this.#start();
  }

  #start(): void {
    if (this.#frame !== null) return;
    this.#frameTime = performance.now();
    this.#frame = window.requestAnimationFrame((now) => this.#animate(now));
  }

  #animate(now: number): void {
    this.#frame = null;
    const elapsed = Math.min(50, Math.max(0, now - this.#frameTime));
    this.#frameTime = now;
    const minimum = window.innerWidth * 0.125;
    const maximum = Math.max(minimum, (window.innerWidth - 126) * 0.5 - 20);

    if (this.#mode === 'entering') {
      const enterElapsed = now - this.#phaseStartedAt;
      if (enterElapsed < 160) {
        const progress = Math.max(0, enterElapsed / 160);
        this.#length = 4;
        this.#opacity = smoothstep(progress);
      } else if (enterElapsed < 400) {
        const progress = (enterElapsed - 160) / 240;
        this.#length = 4 + (minimum - 4) * (1 - (1 - progress) ** 3);
        this.#opacity = 1;
      } else {
        this.#mode = 'volume';
        this.#length = minimum;
        this.#opacity = 1;
        this.#holdUntil = now + 180;
      }
    } else if (this.#mode === 'volume') {
      const visibleVolume = this.#rawVolume ** 0.38;
      const target = minimum + visibleVolume * (maximum - minimum);
      if (target > this.#length) {
        this.#length = target;
        this.#holdUntil = now + 180;
      } else if (now >= this.#holdUntil) {
        const alpha = 1 - Math.exp(-elapsed / 520);
        this.#length += (target - this.#length) * alpha;
      }
      this.#opacity = 1;
      if (Math.abs(target - this.#length) < 0.35 && now >= this.#holdUntil) {
        this.#length = target;
        this.#render();
        return;
      }
    } else if (this.#mode === 'releasing') {
      const fadeThreshold = window.innerWidth * 0.055;
      if (this.#length > fadeThreshold) {
        const alpha = 1 - Math.exp(-elapsed / 165);
        this.#length += (0 - this.#length) * alpha;
      } else {
        this.#fadeStartedAt ??= now;
        const progress = Math.min(1, (now - this.#fadeStartedAt) / 320);
        this.#opacity = this.#fadeStartOpacity * (1 - smoothstep(progress));
        if (progress === 1) {
          this.reset();
          return;
        }
      }
    } else {
      return;
    }

    this.#render();
    this.#frame = window.requestAnimationFrame((next) => this.#animate(next));
  }

  #render(): void {
    this.#stage.style.setProperty('--hud-ray-length', `${this.#length.toFixed(1)}px`);
    this.#stage.style.setProperty('--hud-ray-opacity', this.#opacity.toFixed(3));
  }
}

function smoothstep(value: number): number {
  const clamped = Math.min(1, Math.max(0, value));
  return clamped * clamped * (3 - 2 * clamped);
}

function applyHudLight(light: HudLight): void {
  const hud = document.getElementById('hud');
  if (!hud) throw new Error('DictatingMe HUD bootstrap failed: missing #hud element.');
  hud.dataset.light = light.toLowerCase();
  const label = hud.querySelector<HTMLElement>('.hud__label');
  if (light === 'Yellow') {
    hud.setAttribute('aria-label', 'DictatingMe 正在等待唤醒词');
    if (label) label.textContent = '待唤醒';
  } else if (light === 'Green') {
    hud.setAttribute('aria-label', 'DictatingMe 正在记录输入');
    if (label) label.textContent = '听写中';
  } else {
    hud.setAttribute('aria-label', 'DictatingMe 当前未监听');
    if (label) label.textContent = '未监听';
  }
}

function bootstrap(): void {
  const start = (): void => {
    const stage = document.getElementById('hud-stage');
    if (!stage) throw new Error('DictatingMe HUD bootstrap failed: missing #hud-stage.');
    const animator = new HudRayAnimator(stage);
    let disposed = false;
    let stateEventReceived = false;
    let currentState: DmState | null = null;
    let unlistenState: (() => void) | null = null;
    let unlistenLevel: (() => void) | null = null;

    const updateState = (state: DmState): void => {
      stateEventReceived = true;
      const previous = currentState;
      currentState = state;
      if (state === 'Configure') {
        applyHudLight('Off');
        animator.reset();
      } else if (state === 'Loading' || state === 'Dictating') {
        applyHudLight('Green');
        if (previous !== 'Loading' && previous !== 'Dictating') animator.enterDictating();
      } else if (state === 'Unloading') {
        applyHudLight('Yellow');
        animator.release(false);
      } else {
        applyHudLight('Yellow');
        animator.release(previous === 'Configure' || previous === null);
      }
    };

    applyHudLight('Off');
    void (async () => {
      try {
        const cleanup = await onStateChanged(updateState);
        if (disposed) cleanup();
        else unlistenState = cleanup;
      } catch (error) {
        console.error('Failed to subscribe to HUD state:', error);
      }
      try {
        const cleanup = await onInputLevel((level) => animator.updateVolume(level));
        if (disposed) cleanup();
        else unlistenLevel = cleanup;
      } catch (error) {
        console.error('Failed to subscribe to HUD input level:', error);
      }
      try {
        const state = await getState();
        if (!disposed && !stateEventReceived) updateState(state);
      } catch (error) {
        console.error('Failed to read initial HUD state:', error);
      }
    })();

    const handleResize = (): void => animator.resize();
    window.addEventListener('resize', handleResize);
    window.addEventListener('beforeunload', () => {
      disposed = true;
      unlistenState?.();
      unlistenLevel?.();
      window.removeEventListener('resize', handleResize);
      animator.reset();
    }, { once: true });
  };

  if (document.readyState === 'loading') {
    document.addEventListener('DOMContentLoaded', start, { once: true });
  } else {
    start();
  }
}

bootstrap();
