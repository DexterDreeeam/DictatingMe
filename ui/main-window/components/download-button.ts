import { getOperation, inspectAsset, installAsset } from '../../shared/api';
import type { OperationProgress } from '../../shared/types';
import { onOperationProgress } from '../../shared/events';

type DownloadPhase =
  | 'checking'
  | 'download'
  | 'connecting'
  | 'downloading'
  | 'verifying'
  | 'ready'
  | 'failed';

interface DownloadState {
  phase: DownloadPhase;
  progress: number;
}

const controllers = new Map<string, AssetDownloadController>();
const operations = new Map<string, AssetDownloadController>();
let operationListener: Promise<void> | null = null;

export function createDownloadButton(
  assetLinkList: readonly string[],
  assetPath: string,
): HTMLButtonElement {
  const button = document.createElement('button');
  button.type = 'button';
  button.className = 'asset-download';
  const progress = document.createElement('span');
  progress.className = 'asset-download__progress';
  const spinner = document.createElement('span');
  spinner.className = 'asset-download__spinner';
  const label = document.createElement('span');
  label.className = 'asset-download__label';
  button.append(progress, spinner, label);

  let controller = controllers.get(assetPath);
  if (!controller) {
    controller = new AssetDownloadController(assetPath, assetLinkList);
    controllers.set(assetPath, controller);
  } else {
    controller.updateSources(assetLinkList);
  }
  controller.attach(button);
  button.addEventListener('click', (event) => {
    event.preventDefault();
    event.stopPropagation();
    void controller?.start();
  });
  return button;
}

class AssetDownloadController {
  readonly #assetPath: string;
  #sources: readonly string[];
  #state: DownloadState = { phase: 'checking', progress: 0 };
  #operationId: string | null = null;
  #buttons = new Set<HTMLButtonElement>();
  #inspection: Promise<void> | null = null;

  constructor(assetPath: string, sources: readonly string[]) {
    this.#assetPath = assetPath;
    this.#sources = sources;
  }

  updateSources(sources: readonly string[]): void {
    this.#sources = sources;
  }

  attach(button: HTMLButtonElement): void {
    for (const existing of this.#buttons) {
      if (!existing.isConnected) this.#buttons.delete(existing);
    }
    this.#buttons.add(button);
    renderButton(button, this.#state);
    if (!isActive(this.#state.phase)) void this.verify();
  }

  async verify(): Promise<void> {
    if (isActive(this.#state.phase) || this.#inspection) return this.#inspection ?? Promise.resolve();
    this.setState('checking');
    this.#inspection = (async () => {
      try {
        const asset = await inspectAsset(this.#assetPath);
        this.setState(asset.phase === 'ready' ? 'ready' : 'download', asset.progress ?? 0);
      } catch (error) {
        console.error('Failed to inspect asset:', error);
        this.setState('failed');
      } finally {
        this.#inspection = null;
      }
    })();
    return this.#inspection;
  }

  async start(): Promise<void> {
    if (isActive(this.#state.phase) || this.#state.phase === 'ready') return;
    this.setState('connecting');
    try {
      await ensureOperationListener();
      const operationId = await installAsset(this.#sources, this.#assetPath);
      if (this.#operationId && this.#operationId !== operationId) {
        operations.delete(this.#operationId);
      }
      this.#operationId = operationId;
      operations.set(operationId, this);
      this.applyOperation(await getOperation(operationId));
    } catch (error) {
      console.error('Failed to install asset:', error);
      this.setState('failed');
    }
  }

  applyOperation(operation: OperationProgress): void {
    if (operation.operationId !== this.#operationId) return;
    if (operation.phase === 'queued' || operation.phase === 'connecting') {
      this.setState('connecting');
    } else if (operation.phase === 'downloading') {
      this.setState('downloading', operation.progress ?? this.#state.progress);
    } else if (operation.phase === 'verifying') {
      this.setState('verifying', this.#state.progress);
    } else if (operation.phase === 'completed') {
      this.finishOperation(operation.operationId);
      this.setState('ready', 1);
    } else if (operation.phase === 'failed' || operation.phase === 'cancelled') {
      this.finishOperation(operation.operationId);
      this.setState('failed');
    }
  }

  private finishOperation(operationId: string): void {
    operations.delete(operationId);
    if (this.#operationId === operationId) this.#operationId = null;
  }

  private setState(phase: DownloadPhase, progress = 0): void {
    this.#state = { phase, progress: Math.min(1, Math.max(0, progress)) };
    for (const button of this.#buttons) {
      renderButton(button, this.#state);
    }
  }
}

function ensureOperationListener(): Promise<void> {
  operationListener ??= onOperationProgress((operation) => {
    operations.get(operation.operationId)?.applyOperation(operation);
  }).then(() => undefined);
  return operationListener;
}

function isActive(phase: DownloadPhase): boolean {
  return phase === 'connecting' || phase === 'downloading' || phase === 'verifying';
}

function renderButton(button: HTMLButtonElement, state: DownloadState): void {
  const previousPhase = button.dataset.phase;
  const label = button.querySelector<HTMLElement>('.asset-download__label');
  button.dataset.phase = state.phase;
  button.hidden = state.phase === 'ready';
  button.style.setProperty('--asset-progress', `${Math.round(state.progress * 100)}%`);
  button.disabled = !['download', 'failed'].includes(state.phase);
  if (label) {
    label.textContent = {
      checking: '检查中',
      download: '下载',
      connecting: '连接中',
      downloading: `${Math.round(state.progress * 100)}%`,
      verifying: '验证中',
      ready: '就绪',
      failed: '重试',
    }[state.phase];
  }
  if (state.phase === 'ready' && previousPhase !== 'ready') {
    button.dispatchEvent(new CustomEvent('dictatingme:asset-ready', { bubbles: true }));
  }
}
