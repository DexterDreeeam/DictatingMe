use std::path::PathBuf;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{params, OptionalExtension, TransactionBehavior};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::evoke_setup::{
    EnrollmentPlan, EvokeArtifact, EvokeMode, EvokeProfile, EvokeProfileSummary, EvokeSetupPhase,
    EvokeSetupSession, RecordingQuality,
};

use super::{
    AppConfig, AssetKind, AssetManager, AssetPhase, AssetSummary, ConfigStore, Database,
    StorageError,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppReadiness {
    pub can_enter_listening: bool,
    pub evoke_profile_ready: bool,
    pub dictation_model_ready: bool,
    pub blocking_reasons: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsSnapshot {
    pub generation: u64,
    pub config: AppConfig,
    pub readiness: AppReadiness,
    pub assets: Vec<AssetSummary>,
    pub active_evoke: Option<EvokeProfileSummary>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum OperationKind {
    AssetInstall,
    EvokeProcessing,
}

impl OperationKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::AssetInstall => "assetInstall",
            Self::EvokeProcessing => "evokeProcessing",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum OperationPhase {
    Queued,
    Connecting,
    Downloading,
    Verifying,
    Processing,
    Completed,
    Failed,
    Cancelled,
}

impl OperationPhase {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Connecting => "connecting",
            Self::Downloading => "downloading",
            Self::Verifying => "verifying",
            Self::Processing => "processing",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        match value {
            "queued" => Some(Self::Queued),
            "connecting" => Some(Self::Connecting),
            "downloading" => Some(Self::Downloading),
            "verifying" => Some(Self::Verifying),
            "processing" => Some(Self::Processing),
            "completed" => Some(Self::Completed),
            "failed" => Some(Self::Failed),
            "cancelled" => Some(Self::Cancelled),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OperationProgress {
    pub operation_id: String,
    pub kind: OperationKind,
    pub phase: OperationPhase,
    pub progress: Option<f32>,
    pub message: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone)]
pub struct StoredRecording {
    pub index: u8,
    pub path: PathBuf,
    pub quality: RecordingQuality,
}

#[derive(Clone)]
pub struct SettingsStore {
    db: Arc<Database>,
}

impl SettingsStore {
    pub fn new(db: Arc<Database>) -> Self {
        Self { db }
    }

    pub fn snapshot(&self, assets: &AssetManager) -> Result<SettingsSnapshot, StorageError> {
        let config = ConfigStore::new(Arc::clone(&self.db)).load()?;
        let active_evoke = config
            .active_evoke_profile_id
            .as_deref()
            .map(|id| self.get_profile(id))
            .transpose()?
            .flatten()
            .filter(EvokeProfile::is_runtime_ready);
        let asset_summaries = assets.summaries(config.active_dictation_asset_id.as_deref());
        let dictation_ready = config
            .active_dictation_asset_id
            .as_deref()
            .is_some_and(|selected| {
                asset_summaries.iter().any(|asset| {
                    asset.id == selected
                        && asset.kind == AssetKind::DictationModel
                        && asset.phase == AssetPhase::Ready
                })
            });
        let evoke_ready = active_evoke.is_some();
        let mut blocking_reasons = Vec::new();
        if !evoke_ready {
            blocking_reasons.push("missingEvokeProfile".to_owned());
        }
        if !dictation_ready {
            blocking_reasons.push("missingDictationModel".to_owned());
        }
        Ok(SettingsSnapshot {
            generation: config.generation,
            config,
            readiness: AppReadiness {
                can_enter_listening: evoke_ready && dictation_ready,
                evoke_profile_ready: evoke_ready,
                dictation_model_ready: dictation_ready,
                blocking_reasons,
            },
            assets: asset_summaries,
            active_evoke: active_evoke.as_ref().map(EvokeProfileSummary::from),
        })
    }

    pub fn select_dictation_asset(&self, asset_id: &str) -> Result<AppConfig, StorageError> {
        let connection = self.db.connection()?;
        connection
            .execute(
                "
                UPDATE config
                SET active_dictation_asset_id = ?1,
                    generation = generation + 1
                WHERE singleton = 1
                ",
                params![asset_id],
            )
            .map_err(|error| StorageError(format!("failed to select dictation asset: {error}")))?;
        drop(connection);
        ConfigStore::new(Arc::clone(&self.db)).load()
    }

    pub fn set_sensitivity(&self, value: f32) -> Result<AppConfig, StorageError> {
        let connection = self.db.connection()?;
        connection
            .execute(
                "
                UPDATE config
                SET sensitivity = ?1,
                    generation = generation + 1
                WHERE singleton = 1
                ",
                params![value],
            )
            .map_err(|error| StorageError(format!("failed to set sensitivity: {error}")))?;
        drop(connection);
        ConfigStore::new(Arc::clone(&self.db)).load()
    }

    pub fn set_input_device(&self, device_id: &str) -> Result<AppConfig, StorageError> {
        let connection = self.db.connection()?;
        connection
            .execute(
                "
                UPDATE config
                SET input_device_id = ?1,
                    generation = generation + 1
                WHERE singleton = 1
                ",
                params![device_id],
            )
            .map_err(|error| StorageError(format!("failed to set input device: {error}")))?;
        drop(connection);
        ConfigStore::new(Arc::clone(&self.db)).load()
    }

    pub fn bump_generation(&self) -> Result<AppConfig, StorageError> {
        let connection = self.db.connection()?;
        connection
            .execute(
                "UPDATE config SET generation = generation + 1 WHERE singleton = 1",
                [],
            )
            .map_err(|error| StorageError(format!("failed to bump generation: {error}")))?;
        drop(connection);
        ConfigStore::new(Arc::clone(&self.db)).load()
    }

    pub fn get_profile(&self, id: &str) -> Result<Option<EvokeProfile>, StorageError> {
        let connection = self.db.connection()?;
        connection
            .query_row(
                "
                SELECT id, mode, phrase, threshold, artifact_json,
                       required_asset_ids_json, created_at_ms
                FROM evoke_profiles
                WHERE id = ?1
                ",
                params![id],
                |row| {
                    let mode: String = row.get(1)?;
                    let artifact_json: String = row.get(4)?;
                    let required_json: String = row.get(5)?;
                    let created_at: i64 = row.get(6)?;
                    Ok((
                        row.get::<_, String>(0)?,
                        mode,
                        row.get::<_, String>(2)?,
                        row.get::<_, f32>(3)?,
                        artifact_json,
                        required_json,
                        created_at,
                    ))
                },
            )
            .optional()
            .map_err(|error| StorageError(format!("failed to load evoke profile {id}: {error}")))?
            .map(
                |(id, mode, phrase, threshold, artifact_json, required_json, created_at)| {
                    let mode = EvokeMode::parse(&mode).ok_or_else(|| {
                        StorageError(format!("invalid evoke mode in profile {id}"))
                    })?;
                    let artifact: EvokeArtifact =
                        serde_json::from_str(&artifact_json).map_err(|error| {
                            StorageError(format!("invalid artifact for profile {id}: {error}"))
                        })?;
                    let required_asset_ids =
                        serde_json::from_str(&required_json).map_err(|error| {
                            StorageError(format!(
                                "invalid required assets for profile {id}: {error}"
                            ))
                        })?;
                    Ok(EvokeProfile {
                        id,
                        mode,
                        phrase,
                        threshold,
                        artifact,
                        required_asset_ids,
                        created_at_ms: u64::try_from(created_at).unwrap_or_default(),
                    })
                },
            )
            .transpose()
    }

    pub fn active_profile(&self) -> Result<Option<EvokeProfile>, StorageError> {
        let config = ConfigStore::new(Arc::clone(&self.db)).load()?;
        config
            .active_evoke_profile_id
            .as_deref()
            .map(|id| self.get_profile(id))
            .transpose()
            .map(Option::flatten)
            .map(|profile| profile.filter(EvokeProfile::is_runtime_ready))
    }

    pub fn commit_profile(
        &self,
        setup_id: &str,
        operation_id: &str,
        profile: &EvokeProfile,
    ) -> Result<AppConfig, StorageError> {
        let mut connection = self.db.connection()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| {
                StorageError(format!(
                    "failed to begin evoke profile transaction: {error}"
                ))
            })?;
        let claimed = transaction
            .execute(
                "
                UPDATE evoke_setups
                SET phase = 'committed', error = NULL
                WHERE id = ?1 AND phase = 'processing' AND operation_id = ?2
                ",
                params![setup_id, operation_id],
            )
            .map_err(|error| {
                StorageError(format!(
                    "failed to claim setup for profile activation: {error}"
                ))
            })?;
        if claimed != 1 {
            return Err(StorageError(
                "evoke setup was cancelled or is no longer processing".to_owned(),
            ));
        }
        transaction
            .execute(
                "UPDATE evoke_profiles SET state = 'retired' WHERE state = 'active'",
                [],
            )
            .map_err(|error| StorageError(format!("failed to retire old profile: {error}")))?;
        transaction
            .execute(
                "
                INSERT INTO evoke_profiles(
                    id, mode, phrase, threshold, artifact_json,
                    required_asset_ids_json, created_at_ms, state
                ) VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, 'active')
                ",
                params![
                    profile.id,
                    profile.mode.as_str(),
                    profile.phrase,
                    profile.threshold,
                    serde_json::to_string(&profile.artifact).map_err(|error| StorageError(
                        format!("failed to encode artifact: {error}")
                    ))?,
                    serde_json::to_string(&profile.required_asset_ids).map_err(|error| {
                        StorageError(format!("failed to encode profile asset ids: {error}"))
                    })?,
                    i64::try_from(profile.created_at_ms).map_err(|_| {
                        StorageError("profile timestamp exceeds SQLite INTEGER range".to_owned())
                    })?
                ],
            )
            .map_err(|error| StorageError(format!("failed to insert evoke profile: {error}")))?;
        let current_generation: i64 = transaction
            .query_row(
                "SELECT generation FROM config WHERE singleton = 1",
                [],
                |row| row.get(0),
            )
            .optional()
            .map_err(|error| StorageError(format!("failed to read config generation: {error}")))?
            .unwrap_or(0);
        transaction
            .execute(
                "
                INSERT INTO config(
                    singleton, input_device_id, evoke_word, sensitivity,
                    active_evoke_profile_id, active_dictation_asset_id, generation
                )
                VALUES(1, '', ?1, 0.65, ?2, NULL, ?3)
                ON CONFLICT(singleton) DO UPDATE SET
                    evoke_word = excluded.evoke_word,
                    active_evoke_profile_id = excluded.active_evoke_profile_id,
                    generation = excluded.generation
                ",
                params![
                    profile.phrase,
                    profile.id,
                    current_generation.saturating_add(1)
                ],
            )
            .map_err(|error| StorageError(format!("failed to activate evoke profile: {error}")))?;
        let operation_completed = transaction
            .execute(
                "
                UPDATE operations
                SET phase = 'completed',
                    progress = 1.0,
                    message = ?2,
                    error = NULL,
                    updated_at_ms = ?3
                WHERE id = ?1
                  AND kind = 'evokeProcessing'
                  AND phase = 'processing'
                ",
                params![
                    operation_id,
                    format!("“{}”设置完成", profile.phrase),
                    i64::try_from(now_ms()).unwrap_or(i64::MAX)
                ],
            )
            .map_err(|error| {
                StorageError(format!(
                    "failed to complete evoke processing operation: {error}"
                ))
            })?;
        if operation_completed != 1 {
            return Err(StorageError(
                "evoke processing operation is no longer active".to_owned(),
            ));
        }
        transaction
            .commit()
            .map_err(|error| StorageError(format!("failed to commit evoke profile: {error}")))?;
        drop(connection);
        ConfigStore::new(Arc::clone(&self.db)).load()
    }

    pub fn create_setup(
        &self,
        mode: EvokeMode,
        phrase: &str,
        required_asset_ids: Vec<String>,
    ) -> Result<EvokeSetupSession, StorageError> {
        let id = Uuid::new_v4().to_string();
        let plan = EnrollmentPlan::for_mode(mode, required_asset_ids);
        let phase = if plan.required_recordings == 0 {
            EvokeSetupPhase::ReadyToProcess
        } else {
            EvokeSetupPhase::Draft
        };
        let connection = self.db.connection()?;
        connection
            .execute(
                "
                INSERT INTO evoke_setups(
                    id, mode, phrase, phase, required_recordings, created_at_ms
                ) VALUES(?1, ?2, ?3, ?4, ?5, ?6)
                ",
                params![
                    id,
                    mode.as_str(),
                    phrase,
                    phase.as_str(),
                    i64::from(plan.required_recordings),
                    i64::try_from(now_ms()).unwrap_or(i64::MAX)
                ],
            )
            .map_err(|error| StorageError(format!("failed to create evoke setup: {error}")))?;
        Ok(EvokeSetupSession {
            id,
            mode,
            phrase: phrase.to_owned(),
            phase,
            plan,
            completed_recordings: 0,
            operation_id: None,
            error: None,
        })
    }

    pub fn get_setup(&self, id: &str) -> Result<Option<EvokeSetupSession>, StorageError> {
        let connection = self.db.connection()?;
        let raw = connection
            .query_row(
                "
                SELECT mode, phrase, phase, required_recordings, operation_id, error
                FROM evoke_setups WHERE id = ?1
                ",
                params![id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, i64>(3)?,
                        row.get::<_, Option<String>>(4)?,
                        row.get::<_, Option<String>>(5)?,
                    ))
                },
            )
            .optional()
            .map_err(|error| StorageError(format!("failed to load evoke setup {id}: {error}")))?;
        raw.map(
            |(mode, phrase, phase, required_recordings, operation_id, error)| {
                let mode = EvokeMode::parse(&mode)
                    .ok_or_else(|| StorageError(format!("invalid mode for setup {id}")))?;
                let phase = EvokeSetupPhase::parse(&phase)
                    .ok_or_else(|| StorageError(format!("invalid phase for setup {id}")))?;
                let completed: i64 = connection
                    .query_row(
                        "SELECT COUNT(*) FROM evoke_recordings WHERE setup_id = ?1",
                        params![id],
                        |row| row.get(0),
                    )
                    .map_err(|error| {
                        StorageError(format!(
                            "failed to count recordings for setup {id}: {error}"
                        ))
                    })?;
                let mut plan = EnrollmentPlan::for_mode(mode, Vec::new());
                plan.required_recordings =
                    u8::try_from(required_recordings).unwrap_or(plan.required_recordings);
                Ok(EvokeSetupSession {
                    id: id.to_owned(),
                    mode,
                    phrase,
                    phase,
                    plan,
                    completed_recordings: u8::try_from(completed).unwrap_or(u8::MAX),
                    operation_id,
                    error,
                })
            },
        )
        .transpose()
    }

    pub fn add_recording(
        &self,
        setup_id: &str,
        path: &str,
        quality: &RecordingQuality,
    ) -> Result<u8, StorageError> {
        let mut connection = self.db.connection()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| {
                StorageError(format!("failed to begin recording transaction: {error}"))
            })?;
        let phase: String = transaction
            .query_row(
                "SELECT phase FROM evoke_setups WHERE id = ?1",
                params![setup_id],
                |row| row.get(0),
            )
            .map_err(|error| {
                StorageError(format!("failed to read setup phase for recording: {error}"))
            })?;
        if phase != EvokeSetupPhase::Capturing.as_str() {
            return Err(StorageError(
                "evoke setup was cancelled or is not capturing".to_owned(),
            ));
        }
        let index: i64 = transaction
            .query_row(
                "SELECT COUNT(*) FROM evoke_recordings WHERE setup_id = ?1",
                params![setup_id],
                |row| row.get(0),
            )
            .map_err(|error| StorageError(format!("failed to count setup recordings: {error}")))?;
        transaction
            .execute(
                "
                INSERT INTO evoke_recordings(setup_id, recording_index, audio_path, quality_json)
                VALUES(?1, ?2, ?3, ?4)
                ",
                params![
                    setup_id,
                    index,
                    path,
                    serde_json::to_string(quality).map_err(|error| StorageError(format!(
                        "failed to encode quality: {error}"
                    )))?
                ],
            )
            .map_err(|error| StorageError(format!("failed to save setup recording: {error}")))?;
        let completed = u8::try_from(index.saturating_add(1)).unwrap_or(u8::MAX);
        let required: i64 = transaction
            .query_row(
                "SELECT required_recordings FROM evoke_setups WHERE id = ?1",
                params![setup_id],
                |row| row.get(0),
            )
            .map_err(|error| {
                StorageError(format!(
                    "failed to read setup recording requirement: {error}"
                ))
            })?;
        let next_phase = if index.saturating_add(1) >= required {
            EvokeSetupPhase::ReadyToProcess
        } else {
            EvokeSetupPhase::Draft
        };
        transaction
            .execute(
                "UPDATE evoke_setups SET phase = ?2 WHERE id = ?1 AND phase = 'capturing'",
                params![setup_id, next_phase.as_str()],
            )
            .map_err(|error| {
                StorageError(format!("failed to complete recording transaction: {error}"))
            })?;
        transaction
            .commit()
            .map_err(|error| StorageError(format!("failed to commit recording: {error}")))?;
        Ok(completed)
    }

    pub fn recordings(&self, setup_id: &str) -> Result<Vec<StoredRecording>, StorageError> {
        let connection = self.db.connection()?;
        let mut statement = connection
            .prepare(
                "
                SELECT recording_index, audio_path, quality_json
                FROM evoke_recordings
                WHERE setup_id = ?1
                ORDER BY recording_index
                ",
            )
            .map_err(|error| {
                StorageError(format!("failed to prepare recordings query: {error}"))
            })?;
        let rows = statement
            .query_map(params![setup_id], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })
            .map_err(|error| StorageError(format!("failed to query setup recordings: {error}")))?;
        rows.map(|row| {
            let (index, path, quality) = row
                .map_err(|error| StorageError(format!("failed to read recording row: {error}")))?;
            Ok(StoredRecording {
                index: u8::try_from(index).unwrap_or(u8::MAX),
                path: PathBuf::from(path),
                quality: serde_json::from_str(&quality).map_err(|error| {
                    StorageError(format!("invalid recording quality metadata: {error}"))
                })?,
            })
        })
        .collect()
    }

    pub fn set_setup_phase(
        &self,
        setup_id: &str,
        phase: EvokeSetupPhase,
        operation_id: Option<&str>,
        error: Option<&str>,
    ) -> Result<(), StorageError> {
        let connection = self.db.connection()?;
        connection
            .execute(
                "
                UPDATE evoke_setups
                SET phase = ?2, operation_id = COALESCE(?3, operation_id), error = ?4
                WHERE id = ?1
                ",
                params![setup_id, phase.as_str(), operation_id, error],
            )
            .map_err(|error| StorageError(format!("failed to update evoke setup: {error}")))?;
        Ok(())
    }

    pub fn claim_setup_processing(
        &self,
        setup_id: &str,
        operation_id: &str,
    ) -> Result<(), StorageError> {
        let connection = self.db.connection()?;
        let changed = connection
            .execute(
                "
                UPDATE evoke_setups
                SET phase = 'processing', operation_id = ?2, error = NULL
                WHERE id = ?1 AND phase = 'readyToProcess'
                ",
                params![setup_id, operation_id],
            )
            .map_err(|error| {
                StorageError(format!(
                    "failed to claim evoke setup for processing: {error}"
                ))
            })?;
        if changed != 1 {
            return Err(StorageError(
                "evoke setup is not ready to process or is already processing".to_owned(),
            ));
        }

        Ok(())
    }

    pub fn claim_setup_capture(&self, setup_id: &str) -> Result<(), StorageError> {
        let connection = self.db.connection()?;
        let changed = connection
            .execute(
                "
                UPDATE evoke_setups
                SET phase = 'capturing', error = NULL
                WHERE id = ?1 AND phase = 'draft'
                ",
                params![setup_id],
            )
            .map_err(|error| {
                StorageError(format!(
                    "failed to claim evoke setup for recording: {error}"
                ))
            })?;
        if changed != 1 {
            return Err(StorageError(
                "evoke setup is not ready for recording".to_owned(),
            ));
        }
        Ok(())
    }

    pub fn release_setup_capture(&self, setup_id: &str) -> Result<bool, StorageError> {
        let connection = self.db.connection()?;
        connection
            .execute(
                "
                UPDATE evoke_setups
                SET phase = 'draft'
                WHERE id = ?1 AND phase = 'capturing'
                ",
                params![setup_id],
            )
            .map(|changed| changed == 1)
            .map_err(|error| {
                StorageError(format!("failed to release evoke recording claim: {error}"))
            })
    }

    pub fn cancel_setup(&self, setup_id: &str) -> Result<(), StorageError> {
        let connection = self.db.connection()?;
        let changed = connection
            .execute(
                "
                UPDATE evoke_setups
                SET phase = 'cancelled', error = NULL
                WHERE id = ?1
                  AND phase IN ('draft', 'capturing', 'readyToProcess', 'processing')
                ",
                params![setup_id],
            )
            .map_err(|error| StorageError(format!("failed to cancel evoke setup: {error}")))?;
        if changed != 1 {
            return Err(StorageError(
                "evoke setup can no longer be cancelled".to_owned(),
            ));
        }
        Ok(())
    }

    pub fn create_operation(&self, kind: OperationKind) -> Result<OperationProgress, StorageError> {
        let operation = OperationProgress {
            operation_id: Uuid::new_v4().to_string(),
            kind,
            phase: OperationPhase::Queued,
            progress: None,
            message: None,
            error: None,
        };
        self.update_operation(&operation)?;
        Ok(operation)
    }

    pub fn update_operation(&self, operation: &OperationProgress) -> Result<(), StorageError> {
        let connection = self.db.connection()?;
        connection
            .execute(
                "
                INSERT INTO operations(id, kind, phase, progress, message, error, updated_at_ms)
                VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7)
                ON CONFLICT(id) DO UPDATE SET
                    phase = excluded.phase,
                    progress = excluded.progress,
                    message = excluded.message,
                    error = excluded.error,
                    updated_at_ms = excluded.updated_at_ms
                ",
                params![
                    operation.operation_id,
                    operation.kind.as_str(),
                    operation.phase.as_str(),
                    operation.progress,
                    operation.message,
                    operation.error,
                    i64::try_from(now_ms()).unwrap_or(i64::MAX)
                ],
            )
            .map_err(|error| StorageError(format!("failed to update operation: {error}")))?;
        Ok(())
    }

    pub fn get_operation(&self, id: &str) -> Result<Option<OperationProgress>, StorageError> {
        let connection = self.db.connection()?;
        connection
            .query_row(
                "SELECT kind, phase, progress, message, error FROM operations WHERE id = ?1",
                params![id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, Option<f32>>(2)?,
                        row.get::<_, Option<String>>(3)?,
                        row.get::<_, Option<String>>(4)?,
                    ))
                },
            )
            .optional()
            .map_err(|error| StorageError(format!("failed to get operation {id}: {error}")))?
            .map(|(kind, phase, progress, message, error)| {
                let kind = match kind.as_str() {
                    "assetInstall" => OperationKind::AssetInstall,
                    "evokeProcessing" => OperationKind::EvokeProcessing,
                    _ => return Err(StorageError(format!("invalid operation kind for {id}"))),
                };
                let phase = OperationPhase::parse(&phase)
                    .ok_or_else(|| StorageError(format!("invalid operation phase for {id}")))?;
                Ok(OperationProgress {
                    operation_id: id.to_owned(),
                    kind,
                    phase,
                    progress,
                    message,
                    error,
                })
            })
            .transpose()
    }
}

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(u128::from(u64::MAX)) as u64)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::evoke_setup::EvokeArtifact;

    #[test]
    fn loads_bootstrapped_default_profile() {
        let database = Database::open(":memory:").unwrap();
        let store = SettingsStore::new(database);
        let profile = store.active_profile().unwrap().unwrap();
        assert_eq!(profile.id, "default-text-nihao");
        assert_eq!(profile.phrase, "你好");
        assert!(matches!(profile.artifact, EvokeArtifact::Text { .. }));
    }

    #[test]
    fn cancelled_processing_cannot_activate_a_profile() {
        let database = Database::open(":memory:").unwrap();
        let store = SettingsStore::new(database);
        let setup = store
            .create_setup(EvokeMode::Text, "天气助手", Vec::new())
            .unwrap();
        store
            .claim_setup_processing(&setup.id, "operation-1")
            .unwrap();
        store.cancel_setup(&setup.id).unwrap();
        let profile = EvokeProfile {
            id: "cancelled-profile".to_owned(),
            mode: EvokeMode::Text,
            phrase: "天气助手".to_owned(),
            threshold: 0.5,
            artifact: EvokeArtifact::Text {
                keyword_syntax: "t iān q ì zh ù sh ǒu".to_owned(),
            },
            required_asset_ids: Vec::new(),
            created_at_ms: 1,
        };
        assert!(store
            .commit_profile(&setup.id, "operation-1", &profile)
            .is_err());
        assert_eq!(
            store.active_profile().unwrap().unwrap().id,
            "default-text-nihao"
        );
    }

    #[test]
    fn profile_activation_completes_operation_atomically() {
        let database = Database::open(":memory:").unwrap();
        let store = SettingsStore::new(database);
        let setup = store
            .create_setup(EvokeMode::Text, "天气助手", Vec::new())
            .unwrap();
        let mut operation = store
            .create_operation(OperationKind::EvokeProcessing)
            .unwrap();
        store
            .claim_setup_processing(&setup.id, &operation.operation_id)
            .unwrap();
        operation.phase = OperationPhase::Processing;
        operation.progress = Some(0.85);
        store.update_operation(&operation).unwrap();
        let profile = EvokeProfile {
            id: "active-profile".to_owned(),
            mode: EvokeMode::Text,
            phrase: "天气助手".to_owned(),
            threshold: 0.5,
            artifact: EvokeArtifact::Text {
                keyword_syntax: "t iān q ì zh ù sh ǒu".to_owned(),
            },
            required_asset_ids: Vec::new(),
            created_at_ms: 1,
        };

        store
            .commit_profile(&setup.id, &operation.operation_id, &profile)
            .unwrap();

        let completed = store
            .get_operation(&operation.operation_id)
            .unwrap()
            .unwrap();
        assert_eq!(completed.phase, OperationPhase::Completed);
        assert_eq!(completed.progress, Some(1.0));
        assert_eq!(store.active_profile().unwrap().unwrap().id, profile.id);
        assert_eq!(
            store.get_setup(&setup.id).unwrap().unwrap().phase,
            EvokeSetupPhase::Committed
        );
    }

    #[test]
    fn processing_claim_is_single_use() {
        let database = Database::open(":memory:").unwrap();
        let store = SettingsStore::new(database);
        let setup = store
            .create_setup(EvokeMode::Text, "你好", Vec::new())
            .unwrap();
        store
            .claim_setup_processing(&setup.id, "operation-1")
            .unwrap();
        assert!(store
            .claim_setup_processing(&setup.id, "operation-2")
            .is_err());
    }
}
