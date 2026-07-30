use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter};
use tokio::sync::watch;

use crate::audio::{AudioBus, AudioFrame};
use crate::evoke_setup::features::{frames_to_16k, recording_quality, write_wav_16k};
use crate::evoke_setup::{
    processor_for, EvokeProfile, EvokeSetupPhase, EvokeSetupSession, ProcessInput,
    RecordingReceipt, StartEvokeSetup,
};
use crate::models::DictationModelSpec;
use crate::storage::{
    AppConfig, AppPaths, AssetDescriptor, AssetGroup, AssetInstallRequest, AssetKind, AssetManager,
    AssetPhase, AssetSummary, ConfigStore, OperationKind, OperationPhase, OperationProgress,
    ProgressCallback, SettingsSnapshot, SettingsStore, StorageError,
};

pub const EVENT_OPERATION_PROGRESS: &str = "operation-progress";
pub const EVENT_SETTINGS_CHANGED: &str = "settings-changed";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsChanged {
    pub generation: u64,
}

#[derive(Debug, Clone)]
pub struct RuntimeBundle {
    pub generation: u64,
    pub config: AppConfig,
    pub profile: EvokeProfile,
    pub preset_model_path: PathBuf,
    pub dictation_model: Option<DictationModelSpec>,
    pub speaker_model_path: Option<PathBuf>,
}

#[derive(Clone)]
pub struct SettingsHandle {
    store: SettingsStore,
    config_store: ConfigStore,
    assets: AssetManager,
    paths: AppPaths,
    audio_bus: AudioBus,
    app: AppHandle,
    generation_tx: watch::Sender<u64>,
    asset_operations: Arc<Mutex<std::collections::HashMap<String, String>>>,
    active_captures: Arc<Mutex<std::collections::HashSet<String>>>,
    cancelled_setups: Arc<Mutex<std::collections::HashSet<String>>>,
}

impl SettingsHandle {
    pub fn new(
        store: SettingsStore,
        config_store: ConfigStore,
        assets: AssetManager,
        paths: AppPaths,
        audio_bus: AudioBus,
        app: AppHandle,
    ) -> Result<Self, StorageError> {
        let mut config = config_store.load()?;
        config_store.save(&config)?;
        if config.active_dictation_asset_id.is_none() {
            let ready_models = assets
                .summaries(None)
                .into_iter()
                .filter(|asset| {
                    asset.kind == AssetKind::DictationModel && asset.phase == AssetPhase::Ready
                })
                .collect::<Vec<_>>();
            if ready_models.len() == 1 {
                config = store.select_dictation_asset(&ready_models[0].id)?;
            }
        }
        let generation = config.generation;
        let (generation_tx, _) = watch::channel(generation);
        let handle = Self {
            store,
            config_store,
            assets,
            paths,
            audio_bus,
            app,
            generation_tx,
            asset_operations: Arc::new(Mutex::new(std::collections::HashMap::new())),
            active_captures: Arc::new(Mutex::new(std::collections::HashSet::new())),
            cancelled_setups: Arc::new(Mutex::new(std::collections::HashSet::new())),
        };
        handle.start_background_verification();
        Ok(handle)
    }

    pub fn subscribe_generation(&self) -> watch::Receiver<u64> {
        self.generation_tx.subscribe()
    }

    pub fn snapshot(&self) -> Result<SettingsSnapshot, StorageError> {
        self.store.snapshot(&self.assets)
    }

    pub fn config(&self) -> Result<AppConfig, StorageError> {
        self.config_store.load()
    }

    pub fn persist_input_device(&self, device_id: &str) -> Result<AppConfig, StorageError> {
        let config = self.store.set_input_device(device_id)?;
        self.notify_generation(config.generation);
        Ok(config)
    }

    pub fn inspect_asset(&self, asset_path: &str) -> AssetSummary {
        let previous_phase = self.assets.cached_phase_for_path(asset_path);
        let selected = self
            .config_store
            .load()
            .ok()
            .and_then(|config| config.active_dictation_asset_id);
        let summary = self.assets.inspect(asset_path, selected.as_deref());
        if previous_phase != AssetPhase::Ready && summary.phase == AssetPhase::Ready {
            match self.config_store.load() {
                Ok(config) => {
                    if summary.selected {
                        self.notify_generation(config.generation);
                    }
                }
                Err(error) => {
                    tracing::error!(
                        asset_path,
                        %error,
                        "failed to publish completed asset verification"
                    );
                }
            }
        }
        summary
    }

    pub fn install_asset(&self, request: AssetInstallRequest) -> Result<String, StorageError> {
        let asset_id = self
            .assets
            .descriptor_for_path(&request.asset_path)?
            .id
            .clone();
        let mut operations = self
            .asset_operations
            .lock()
            .unwrap_or_else(|lock| lock.into_inner());
        if let Some(operation_id) = operations.get(&asset_id) {
            if self
                .store
                .get_operation(operation_id)?
                .is_some_and(|operation| {
                    !matches!(
                        operation.phase,
                        OperationPhase::Completed
                            | OperationPhase::Failed
                            | OperationPhase::Cancelled
                    )
                })
            {
                return Ok(operation_id.clone());
            }
            operations.remove(&asset_id);
        }
        let operation = self.store.create_operation(OperationKind::AssetInstall)?;
        let operation_id = operation.operation_id.clone();
        operations.insert(asset_id.clone(), operation_id.clone());
        drop(operations);
        self.emit_operation(&operation);
        let this = self.clone();
        tauri::async_runtime::spawn(async move {
            let callback_this = this.clone();
            let callback_id = operation_id.clone();
            let progress: ProgressCallback = Arc::new(move |asset_progress| {
                let operation = OperationProgress {
                    operation_id: callback_id.clone(),
                    kind: OperationKind::AssetInstall,
                    phase: operation_phase_for_asset(asset_progress.phase),
                    progress: asset_progress.progress,
                    message: asset_progress.message,
                    error: None,
                };
                let _ = callback_this.store.update_operation(&operation);
                callback_this.emit_operation(&operation);
            });
            match this.assets.install(request, progress).await {
                Ok(asset) => {
                    let mut generation_changed = false;
                    if asset.kind == AssetKind::DictationModel {
                        let config = this.config_store.load();
                        let only_ready_dictation_model = this
                            .assets
                            .summaries(None)
                            .into_iter()
                            .filter(|summary| {
                                summary.kind == AssetKind::DictationModel
                                    && summary.phase == AssetPhase::Ready
                            })
                            .count()
                            == 1;
                        if config
                            .as_ref()
                            .is_ok_and(|config| config.active_dictation_asset_id.is_none())
                            && only_ready_dictation_model
                        {
                            if let Ok(config) = this.store.select_dictation_asset(&asset.id) {
                                this.notify_generation(config.generation);
                                generation_changed = true;
                            }
                        }
                    }
                    let operation = OperationProgress {
                        operation_id: operation_id.clone(),
                        kind: OperationKind::AssetInstall,
                        phase: OperationPhase::Completed,
                        progress: Some(1.0),
                        message: Some(format!("{} 已就绪", asset.display_name)),
                        error: None,
                    };
                    let _ = this.store.update_operation(&operation);
                    this.emit_operation(&operation);
                    if !generation_changed {
                        let _ = this.bump_generation();
                    }
                }
                Err(error) => {
                    let operation = OperationProgress {
                        operation_id: operation_id.clone(),
                        kind: OperationKind::AssetInstall,
                        phase: OperationPhase::Failed,
                        progress: None,
                        message: None,
                        error: Some(error.0),
                    };
                    let _ = this.store.update_operation(&operation);
                    this.emit_operation(&operation);
                }
            }
            this.asset_operations
                .lock()
                .unwrap_or_else(|lock| lock.into_inner())
                .remove(&asset_id);
        });
        Ok(operation.operation_id)
    }

    pub fn select_dictation_model(&self, asset_id: &str) -> Result<SettingsSnapshot, StorageError> {
        let descriptor = self.assets.descriptor(asset_id)?;
        if descriptor.kind != AssetKind::DictationModel {
            return Err(StorageError(format!(
                "asset '{asset_id}' is not a dictation model"
            )));
        }
        self.assets.require_verified(descriptor)?;
        let config = self.store.select_dictation_asset(asset_id)?;
        self.notify_generation(config.generation);
        self.snapshot()
    }

    pub fn set_sensitivity(&self, value: f32) -> Result<SettingsSnapshot, StorageError> {
        if !value.is_finite() || !(0.0..=1.0).contains(&value) {
            return Err(StorageError(
                "sensitivity must be between 0 and 1".to_owned(),
            ));
        }

        let config = self.store.set_sensitivity(value)?;
        self.notify_generation(config.generation);
        self.snapshot()
    }

    fn start_background_verification(&self) {
        let asset_paths = self
            .assets
            .summaries(None)
            .into_iter()
            .filter(|asset| asset.phase == AssetPhase::Checking)
            .map(|asset| asset.asset_path)
            .collect::<Vec<_>>();
        if asset_paths.is_empty() {
            return;
        }
        let settings = self.clone();
        tauri::async_runtime::spawn(async move {
            for asset_path in asset_paths {
                tracing::info!(asset_path, "background asset verification begin");
                let verification = settings.clone();
                match tokio::task::spawn_blocking(move || verification.inspect_asset(&asset_path))
                    .await
                {
                    Ok(summary) => tracing::info!(
                        asset_id = %summary.id,
                        phase = ?summary.phase,
                        "background asset verification finish"
                    ),
                    Err(error) => {
                        tracing::error!(
                            %error,
                            "background asset verification task failed"
                        );
                    }
                }
            }
        });
    }

    pub fn begin_evoke_setup(
        &self,
        request: StartEvokeSetup,
    ) -> Result<EvokeSetupSession, StorageError> {
        let phrase = request.phrase.trim();
        if phrase.is_empty() || phrase.chars().count() > 32 {
            return Err(StorageError(
                "wake phrase must contain between 1 and 32 characters".to_owned(),
            ));
        }
        let required_assets = self.required_assets_for_mode(request.mode)?;
        for descriptor in &required_assets {
            let path = self.assets.asset_path(descriptor)?;
            crate::storage::verify_asset_directory(&path, descriptor).map_err(|_| {
                StorageError(format!(
                    "required asset '{}' is not ready for {} mode",
                    descriptor.display_name,
                    request.mode.as_str()
                ))
            })?;
        }
        self.store.create_setup(
            request.mode,
            phrase,
            required_assets
                .into_iter()
                .map(|descriptor| descriptor.id.clone())
                .collect(),
        )
    }

    pub async fn capture_evoke_sample(
        &self,
        setup_id: &str,
    ) -> Result<RecordingReceipt, StorageError> {
        {
            let mut captures = self
                .active_captures
                .lock()
                .unwrap_or_else(|lock| lock.into_inner());
            if !captures.insert(setup_id.to_owned()) {
                return Err(StorageError(
                    "a recording is already active for this setup".to_owned(),
                ));
            }
        }
        let result = self.capture_evoke_sample_claimed(setup_id).await;
        if result.is_err() {
            let _ = self.store.release_setup_capture(setup_id);
        }
        self.active_captures
            .lock()
            .unwrap_or_else(|lock| lock.into_inner())
            .remove(setup_id);
        result
    }

    async fn capture_evoke_sample_claimed(
        &self,
        setup_id: &str,
    ) -> Result<RecordingReceipt, StorageError> {
        let setup = self
            .store
            .get_setup(setup_id)?
            .ok_or_else(|| StorageError(format!("evoke setup not found: {setup_id}")))?;
        if setup.completed_recordings >= setup.plan.required_recordings {
            return Err(StorageError(
                "all required recordings are complete".to_owned(),
            ));
        }
        if matches!(
            setup.phase,
            EvokeSetupPhase::Capturing
                | EvokeSetupPhase::Processing
                | EvokeSetupPhase::Committed
                | EvokeSetupPhase::Cancelled
        ) {
            return Err(StorageError(format!(
                "cannot record while setup is in {:?} phase",
                setup.phase
            )));
        }
        self.store.claim_setup_capture(setup_id)?;
        let mut receiver = self.audio_bus.subscribe();
        let started = Instant::now();
        let mut frames = Vec::<AudioFrame>::new();
        while started.elapsed() < Duration::from_millis(5_000) {
            let remaining = Duration::from_millis(5_300)
                .saturating_sub(started.elapsed())
                .max(Duration::from_millis(50));
            match tokio::time::timeout(remaining, receiver.recv()).await {
                Ok(Ok(frame)) => frames.push(frame),
                Ok(Err(tokio::sync::broadcast::error::RecvError::Lagged(_))) => continue,
                Ok(Err(tokio::sync::broadcast::error::RecvError::Closed)) | Err(_) => {
                    let released = self.store.release_setup_capture(setup_id)?;
                    if !released && self.is_cancelled(setup_id) {
                        return Err(StorageError("evoke setup was cancelled".to_owned()));
                    }
                    return Err(StorageError(
                        "audio capture stopped before the 5-second recording completed".to_owned(),
                    ));
                }
            }
        }
        let mut samples = frames_to_16k(&frames);
        samples.resize(80_000, 0.0);
        samples.truncate(80_000);
        if self.is_cancelled(setup_id) {
            return Err(StorageError("evoke setup was cancelled".to_owned()));
        }
        let quality = recording_quality(&samples);
        if !quality.accepted {
            let released = self.store.release_setup_capture(setup_id)?;
            if !released && self.is_cancelled(setup_id) {
                return Err(StorageError("evoke setup was cancelled".to_owned()));
            }
            return Ok(RecordingReceipt {
                setup_id: setup_id.to_owned(),
                index: setup.completed_recordings,
                duration_ms: 5_000,
                quality,
                completed_recordings: setup.completed_recordings,
                remaining_recordings: setup
                    .plan
                    .required_recordings
                    .saturating_sub(setup.completed_recordings),
            });
        }
        let directory = self.paths.sessions.join(setup_id).join("recordings");
        let path = directory.join(format!("{:02}.wav", setup.completed_recordings));
        write_wav_16k(&path, &samples).map_err(StorageError)?;
        let completed = match self.store.add_recording(
            setup_id,
            path.to_str()
                .ok_or_else(|| StorageError("recording path is not valid UTF-8".to_owned()))?,
            &quality,
        ) {
            Ok(completed) => completed,
            Err(error) => {
                let _ = std::fs::remove_file(&path);
                return Err(error);
            }
        };
        Ok(RecordingReceipt {
            setup_id: setup_id.to_owned(),
            index: completed.saturating_sub(1),
            duration_ms: 5_000,
            quality,
            completed_recordings: completed,
            remaining_recordings: setup.plan.required_recordings.saturating_sub(completed),
        })
    }

    pub fn finish_evoke_setup(&self, setup_id: &str) -> Result<String, StorageError> {
        let setup = self
            .store
            .get_setup(setup_id)?
            .ok_or_else(|| StorageError(format!("evoke setup not found: {setup_id}")))?;
        if setup.completed_recordings != setup.plan.required_recordings {
            return Err(StorageError(format!(
                "setup requires {} recordings, found {}",
                setup.plan.required_recordings, setup.completed_recordings
            )));
        }
        if setup.phase != EvokeSetupPhase::ReadyToProcess {
            return Err(StorageError(format!(
                "setup cannot finish while in {:?} phase",
                setup.phase
            )));
        }
        let operation = self
            .store
            .create_operation(OperationKind::EvokeProcessing)?;
        if let Err(error) = self
            .store
            .claim_setup_processing(setup_id, &operation.operation_id)
        {
            let cancelled = OperationProgress {
                operation_id: operation.operation_id.clone(),
                kind: OperationKind::EvokeProcessing,
                phase: OperationPhase::Cancelled,
                progress: None,
                message: None,
                error: Some(error.0.clone()),
            };
            let _ = self.store.update_operation(&cancelled);
            return Err(error);
        }
        self.emit_operation(&operation);
        let operation_id = operation.operation_id.clone();
        let setup_id = setup_id.to_owned();
        let this = self.clone();
        tauri::async_runtime::spawn(async move {
            tracing::info!(
                setup_id,
                operation_id,
                mode = setup.mode.as_str(),
                "evoke setup processing started"
            );
            let result = async {
                let mut progress = OperationProgress {
                    operation_id: operation_id.clone(),
                    kind: OperationKind::EvokeProcessing,
                    phase: OperationPhase::Processing,
                    progress: Some(0.1),
                    message: Some("正在读取录音数据".to_owned()),
                    error: None,
                };
                this.store.update_operation(&progress)?;
                this.emit_operation(&progress);
                let recordings = this.store.recordings(&setup_id)?;
                if this.is_cancelled(&setup_id) {
                    return Err(StorageError("evoke setup was cancelled".to_owned()));
                }
                let input = ProcessInput {
                    mode: setup.mode,
                    phrase: setup.phrase.clone(),
                    recording_paths: recordings.into_iter().map(|item| item.path).collect(),
                };
                progress.progress = Some(0.35);
                progress.message = Some("正在提取声学特征".to_owned());
                this.store.update_operation(&progress)?;
                this.emit_operation(&progress);
                let processor = processor_for(setup.mode);
                let profile = tokio::time::timeout(
                    Duration::from_secs(60),
                    processor.process(input, &this.assets),
                )
                .await
                .map_err(|_| StorageError("唤醒数据处理超过 60 秒，请重试".to_owned()))??;
                tracing::info!(
                    setup_id,
                    operation_id,
                    profile_id = profile.id,
                    "evoke profile features created"
                );
                if this.is_cancelled(&setup_id) {
                    return Err(StorageError("evoke setup was cancelled".to_owned()));
                }
                progress.progress = Some(0.85);
                progress.message = Some("正在验证并激活唤醒配置".to_owned());
                this.store.update_operation(&progress)?;
                this.emit_operation(&progress);
                this.persist_profile_file(&profile)?;
                let config = this
                    .store
                    .commit_profile(&setup_id, &operation_id, &profile)?;
                tracing::info!(
                    setup_id,
                    operation_id,
                    generation = config.generation,
                    "evoke profile and operation committed"
                );
                let _ = std::fs::remove_dir_all(this.paths.sessions.join(&setup_id));
                let completed = OperationProgress {
                    operation_id: operation_id.clone(),
                    kind: OperationKind::EvokeProcessing,
                    phase: OperationPhase::Completed,
                    progress: Some(1.0),
                    message: Some(format!("“{}”设置完成", profile.phrase)),
                    error: None,
                };
                this.emit_operation(&completed);
                this.notify_generation(config.generation);
                tracing::info!(setup_id, operation_id, "evoke setup processing completed");
                Ok::<_, StorageError>(profile)
            }
            .await;
            match result {
                Ok(_) => {}
                Err(error) => {
                    tracing::error!(setup_id, operation_id, %error, "evoke setup processing failed");
                    let cancelled = this.is_cancelled(&setup_id);
                    let failed = OperationProgress {
                        operation_id: operation_id.clone(),
                        kind: OperationKind::EvokeProcessing,
                        phase: if cancelled {
                            OperationPhase::Cancelled
                        } else {
                            OperationPhase::Failed
                        },
                        progress: None,
                        message: None,
                        error: Some(error.0.clone()),
                    };
                    let _ = this.store.update_operation(&failed);
                    if !cancelled {
                        let _ = this.store.set_setup_phase(
                            &setup_id,
                            EvokeSetupPhase::Failed,
                            Some(&operation_id),
                            Some(&error.0),
                        );
                    }
                    this.emit_operation(&failed);
                }
            }
        });
        Ok(operation.operation_id)
    }

    pub fn get_evoke_setup(&self, setup_id: &str) -> Result<EvokeSetupSession, StorageError> {
        let mut setup = self
            .store
            .get_setup(setup_id)?
            .ok_or_else(|| StorageError(format!("evoke setup not found: {setup_id}")))?;
        setup.plan.required_asset_ids = self
            .required_assets_for_mode(setup.mode)?
            .into_iter()
            .map(|descriptor| descriptor.id.clone())
            .collect();
        Ok(setup)
    }

    pub fn cancel_evoke_setup(&self, setup_id: &str) -> Result<(), StorageError> {
        self.cancelled_setups
            .lock()
            .unwrap_or_else(|lock| lock.into_inner())
            .insert(setup_id.to_owned());
        self.store.cancel_setup(setup_id)
    }

    pub fn get_operation(&self, operation_id: &str) -> Result<OperationProgress, StorageError> {
        self.store
            .get_operation(operation_id)?
            .ok_or_else(|| StorageError(format!("operation not found: {operation_id}")))
    }

    pub fn runtime_bundle(&self) -> Result<RuntimeBundle, StorageError> {
        let snapshot = self.snapshot()?;
        if !snapshot.readiness.can_enter_listening {
            return Err(StorageError(format!(
                "DictatingMe is not ready: {}",
                snapshot.readiness.blocking_reasons.join(", ")
            )));
        }
        let profile = self
            .store
            .active_profile()?
            .ok_or_else(|| StorageError("active evoke profile is missing".to_owned()))?;
        let preset = self
            .assets
            .first_descriptor_of_kind(AssetKind::PresetEvoke)?;
        let dictation_id = snapshot
            .config
            .active_dictation_asset_id
            .as_deref()
            .ok_or_else(|| StorageError("no dictation model is selected".to_owned()))?;
        let dictation = self.assets.descriptor(dictation_id)?;
        let dictation_model =
            DictationModelSpec::from_descriptor(dictation, self.assets.asset_path(dictation)?)
                .map_err(|error| StorageError(error.0))?;
        let speaker_model_path = self.speaker_model_path(&profile, false)?;
        Ok(RuntimeBundle {
            generation: snapshot.generation,
            config: snapshot.config,
            profile,
            preset_model_path: self.assets.asset_path(preset)?,
            dictation_model: Some(dictation_model),
            speaker_model_path,
        })
    }

    pub fn initial_bundle(&self) -> Result<RuntimeBundle, StorageError> {
        let config = self.config_store.load()?;
        let profile = self
            .store
            .active_profile()?
            .ok_or_else(|| StorageError("active evoke profile is missing".to_owned()))?;
        let preset = self
            .assets
            .first_descriptor_of_kind(AssetKind::PresetEvoke)?;
        let dictation_model = config
            .active_dictation_asset_id
            .as_deref()
            .and_then(|id| self.assets.descriptor(id).ok())
            .and_then(|descriptor| {
                let path = self.assets.asset_path(descriptor).ok()?;
                DictationModelSpec::from_descriptor(descriptor, path).ok()
            });
        let speaker_model_path = self.speaker_model_path(&profile, true)?;
        Ok(RuntimeBundle {
            generation: config.generation,
            config,
            profile,
            preset_model_path: self.assets.asset_path(preset)?,
            dictation_model,
            speaker_model_path,
        })
    }

    fn persist_profile_file(&self, profile: &EvokeProfile) -> Result<(), StorageError> {
        let directory = self.paths.profiles.join(&profile.id);
        std::fs::create_dir_all(&directory).map_err(|error| {
            StorageError(format!(
                "failed to create profile directory '{}': {error}",
                directory.display()
            ))
        })?;
        let bytes = serde_json::to_vec_pretty(profile)
            .map_err(|error| StorageError(format!("failed to encode profile: {error}")))?;
        let temporary = directory.join("profile.json.tmp");
        let final_path = directory.join("profile.json");
        std::fs::write(&temporary, bytes).map_err(|error| {
            StorageError(format!(
                "failed to write profile '{}': {error}",
                temporary.display()
            ))
        })?;
        std::fs::rename(&temporary, &final_path).map_err(|error| {
            StorageError(format!(
                "failed to atomically commit profile '{}': {error}",
                final_path.display()
            ))
        })
    }

    fn required_assets_for_mode(
        &self,
        mode: crate::evoke_setup::EvokeMode,
    ) -> Result<Vec<&AssetDescriptor>, StorageError> {
        match mode {
            crate::evoke_setup::EvokeMode::Text | crate::evoke_setup::EvokeMode::VoiceMatch => {
                Ok(Vec::new())
            }
            crate::evoke_setup::EvokeMode::SpeakerVerify => self
                .assets
                .descriptors_for_group(AssetGroup::SpeakerRecognition),
            crate::evoke_setup::EvokeMode::Classifier => self
                .assets
                .descriptors_for_group(AssetGroup::ClassifierRecognition),
        }
    }

    fn speaker_model_path(
        &self,
        profile: &EvokeProfile,
        verify: bool,
    ) -> Result<Option<PathBuf>, StorageError> {
        if profile.mode != crate::evoke_setup::EvokeMode::SpeakerVerify {
            return Ok(None);
        }
        let mut profile_descriptor = None;
        for asset_id in &profile.required_asset_ids {
            let descriptor = self.assets.descriptor(asset_id)?;
            if descriptor.kind == AssetKind::SpeakerEmbedding {
                profile_descriptor = Some(descriptor);
                break;
            }
        }
        let descriptor = match profile_descriptor {
            Some(descriptor) => descriptor,
            None => self
                .assets
                .primary_descriptor(AssetGroup::SpeakerRecognition)?,
        };
        let directory = self.assets.asset_path(descriptor)?;
        if verify {
            crate::storage::verify_asset_directory(&directory, descriptor)?;
        }
        let file = descriptor.files.first().ok_or_else(|| {
            StorageError(format!("speaker asset '{}' has no files", descriptor.id))
        })?;
        Ok(Some(directory.join(&file.path)))
    }

    fn bump_generation(&self) -> Result<(), StorageError> {
        let config = self.store.bump_generation()?;
        self.notify_generation(config.generation);
        Ok(())
    }

    fn notify_generation(&self, generation: u64) {
        let _ = self.generation_tx.send(generation);
        let _ = self
            .app
            .emit(EVENT_SETTINGS_CHANGED, SettingsChanged { generation });
    }

    fn emit_operation(&self, operation: &OperationProgress) {
        let _ = self.app.emit(EVENT_OPERATION_PROGRESS, operation.clone());
    }

    fn is_cancelled(&self, setup_id: &str) -> bool {
        self.cancelled_setups
            .lock()
            .unwrap_or_else(|lock| lock.into_inner())
            .contains(setup_id)
    }
}

fn operation_phase_for_asset(phase: AssetPhase) -> OperationPhase {
    match phase {
        AssetPhase::Missing | AssetPhase::Checking => OperationPhase::Queued,
        AssetPhase::Connecting => OperationPhase::Connecting,
        AssetPhase::Downloading => OperationPhase::Downloading,
        AssetPhase::Verifying => OperationPhase::Verifying,
        AssetPhase::Ready => OperationPhase::Completed,
        AssetPhase::Failed => OperationPhase::Failed,
    }
}
