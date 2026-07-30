/**
 * 前端共享类型定义（对应 Rust 端 state_machine / storage / audio 模块的数据形状）。
 * 见 brainstrom/plan.md §4.1、§6、§9。
 */

/** Runtime 状态机的 5 个状态（对应 Rust `State` enum）。 */
export type DmState = 'Configure' | 'Listening' | 'Loading' | 'Dictating' | 'Unloading';

/** HUD 灯光颜色（黄=可唤醒，绿=记录输入中，Off=无响应/瞬时熄灭）。 */
export type HudLight = 'Yellow' | 'Green' | 'Off';

/** 一个可用的音频输入设备（InputDevice 二级页）。 */
export interface AudioDeviceInfo {
  id: string;
  name: string;
  isDefault: boolean;
}

/** 持久化配置（对应 Rust `AppConfig`）。 */
export interface AppConfig {
  inputDeviceId: string;
  /** 当前生效唤醒词，同一时间只能有 1 个（见 plan.md §6.3） */
  evokeWord: string;
  /** 敏感度 0.0（不易误触发）- 1.0（更容易唤醒） */
  sensitivity: number;
  activeEvokeProfileId: string | null;
  activeDictationAssetId: string | null;
  generation: number;
}

export type AssetKind =
  | 'presetEvoke'
  | 'dictationModel'
  | 'speakerEmbedding'
  | 'classifierResource';

export type AssetGroup =
  | 'speakerRecognition'
  | 'classifierRecognition'
  | 'speechModels';

export type AssetPhase =
  | 'missing'
  | 'checking'
  | 'connecting'
  | 'downloading'
  | 'verifying'
  | 'ready'
  | 'failed';

export type RecognizerType = 'onlineTransducer' | 'offlineGenerative';
export type OutputMode = 'streaming' | 'utterance';

export interface AssetSummary {
  id: string;
  kind: AssetKind;
  assetGroup: AssetGroup | null;
  displayName: string;
  fileSizeBytes: number | null;
  recognizerType: RecognizerType | null;
  outputMode: OutputMode | null;
  version: string;
  assetPath: string;
  sources: string[];
  phase: AssetPhase;
  progress: number | null;
  error: string | null;
  selected: boolean;
}

export interface AppReadiness {
  canEnterListening: boolean;
  evokeProfileReady: boolean;
  dictationModelReady: boolean;
  blockingReasons: string[];
}

export type EvokeMode = 'text' | 'voiceMatch' | 'speakerVerify' | 'classifier';

export interface EvokeProfileSummary {
  id: string;
  mode: EvokeMode;
  phrase: string;
  createdAtMs: number;
}

export interface SettingsSnapshot {
  generation: number;
  config: AppConfig;
  readiness: AppReadiness;
  assets: AssetSummary[];
  activeEvoke: EvokeProfileSummary | null;
}

export type OperationKind = 'assetInstall' | 'evokeProcessing';
export type OperationPhase =
  | 'queued'
  | 'connecting'
  | 'downloading'
  | 'verifying'
  | 'processing'
  | 'completed'
  | 'failed'
  | 'cancelled';

export interface OperationProgress {
  operationId: string;
  kind: OperationKind;
  phase: OperationPhase;
  progress: number | null;
  message: string | null;
  error: string | null;
}

export interface StartEvokeSetup {
  mode: EvokeMode;
  phrase: string;
}

export interface EnrollmentPlan {
  mode: EvokeMode;
  requiredRecordings: number;
  recordingDurationMs: number;
  prompts: Array<{ id: string; text: string }>;
  requiredAssetIds: string[];
}

export type EvokeSetupPhase =
  | 'draft'
  | 'capturing'
  | 'readyToProcess'
  | 'processing'
  | 'committed'
  | 'failed'
  | 'cancelled';

export interface EvokeSetupSession {
  id: string;
  mode: EvokeMode;
  phrase: string;
  phase: EvokeSetupPhase;
  plan: EnrollmentPlan;
  completedRecordings: number;
  operationId: string | null;
  error: string | null;
}

export interface RecordingQuality {
  accepted: boolean;
  rms: number;
  peak: number;
  clippingRatio: number;
  voicedRatio: number;
  rejection: string | null;
}

export interface RecordingReceipt {
  setupId: string;
  index: number;
  durationMs: number;
  quality: RecordingQuality;
  completedRecordings: number;
  remainingRecordings: number;
}

export interface EvokeScore {
  overall: number;
  threshold: number;
  voiceActivity: number;
  phraseScore: number;
  modeScore: number;
  accepted: boolean;
  mode: EvokeMode;
}

/** 一条历史听写记录（History 二级页，FIFO 最多 20 条）。 */
export interface HistoryEntry {
  id: string;
  /** Unix 毫秒时间戳 */
  timestampMs: number;
  text: string;
}

/** History 列表容量上限，见 plan.md §6.4。 */
export const HISTORY_CAPACITY = 20;
