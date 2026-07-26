//! Config Store（见 brainstrom/plan.md §9）。
//!
//! 持久化保存 InputDevice / EvokeWord（唤醒词+敏感度）等设置，重启后保留。

use super::db::{Database, StorageError};
use rusqlite::{params, OptionalExtension};
use std::sync::Arc;

/// 持久化配置项（对应 MainWindow 的 InputDevice / EvokeWord 二级页）。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppConfig {
    /// 当前选中的输入设备 id（对应 `crate::audio::AudioDeviceInfo::id`）
    pub input_device_id: String,
    /// 当前生效唤醒词（同一时间只能有 1 个，见 plan.md §6.3）
    pub evoke_word: String,
    /// 唤醒词敏感度 0.0-1.0
    pub sensitivity: f32,
    pub active_evoke_profile_id: Option<String>,
    pub active_dictation_asset_id: Option<String>,
    pub generation: u64,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            input_device_id: String::new(),
            evoke_word: "你好".to_owned(),
            sensitivity: 0.65,
            active_evoke_profile_id: Some("default-text-nihao".to_owned()),
            active_dictation_asset_id: None,
            generation: 0,
        }
    }
}

/// Config Store：`AppConfig` 的持久化读写封装。
#[derive(Clone)]
pub struct ConfigStore {
    db: Arc<Database>,
}

impl ConfigStore {
    pub fn new(db: Arc<Database>) -> Self {
        Self { db }
    }

    /// 读取配置；若从未保存过，返回内置默认值。
    pub fn load(&self) -> Result<AppConfig, StorageError> {
        let connection = self.db.connection()?;
        let config = connection
            .query_row(
                "
                SELECT input_device_id, evoke_word, sensitivity,
                       active_evoke_profile_id, active_dictation_asset_id, generation
                FROM config
                WHERE singleton = 1
                ",
                [],
                |row| {
                    Ok(AppConfig {
                        input_device_id: row.get(0)?,
                        evoke_word: row.get(1)?,
                        sensitivity: row.get(2)?,
                        active_evoke_profile_id: row.get(3)?,
                        active_dictation_asset_id: row.get(4)?,
                        generation: u64::try_from(row.get::<_, i64>(5)?).unwrap_or_default(),
                    })
                },
            )
            .optional()
            .map_err(|error| StorageError(format!("failed to load config: {error}")))?
            .unwrap_or_default();
        validate_config(&config)?;
        Ok(config)
    }

    pub fn save(&self, config: &AppConfig) -> Result<(), StorageError> {
        validate_config(config)?;

        let connection = self.db.connection()?;
        connection
            .execute(
                "
                INSERT INTO config(
                    singleton, input_device_id, evoke_word, sensitivity,
                    active_evoke_profile_id, active_dictation_asset_id, generation
                )
                VALUES(1, ?1, ?2, ?3, ?4, ?5, ?6)
                ON CONFLICT(singleton) DO UPDATE SET
                    input_device_id = excluded.input_device_id,
                    evoke_word = excluded.evoke_word,
                    sensitivity = excluded.sensitivity,
                    active_evoke_profile_id = excluded.active_evoke_profile_id,
                    active_dictation_asset_id = excluded.active_dictation_asset_id,
                    generation = excluded.generation
                ",
                params![
                    config.input_device_id,
                    config.evoke_word,
                    config.sensitivity,
                    config.active_evoke_profile_id,
                    config.active_dictation_asset_id,
                    i64::try_from(config.generation).map_err(|_| StorageError(
                        "config generation exceeds SQLite INTEGER range".to_owned()
                    ))?
                ],
            )
            .map(|_| ())
            .map_err(|error| StorageError(format!("failed to save config: {error}")))
    }
}

fn validate_config(config: &AppConfig) -> Result<(), StorageError> {
    if !config.sensitivity.is_finite() || !(0.0..=1.0).contains(&config.sensitivity) {
        return Err(StorageError(
            "config sensitivity must be a finite value between 0.0 and 1.0".to_owned(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEST_DATABASE_ID: AtomicU64 = AtomicU64::new(0);

    fn database_path() -> std::path::PathBuf {
        let id = TEST_DATABASE_ID.fetch_add(1, Ordering::Relaxed);
        std::env::current_dir()
            .unwrap()
            .join("target")
            .join(format!("config-store-test-{}-{id}.db", std::process::id()))
    }

    #[test]
    fn returns_defaults_then_persists_config() {
        let path = database_path();
        let path_text = path.to_str().unwrap();
        let database = Database::open(path_text).unwrap();
        let store = ConfigStore::new(database.clone());
        assert_eq!(store.load().unwrap(), AppConfig::default());

        let config = AppConfig {
            input_device_id: "microphone-2".to_owned(),
            evoke_word: "开始听写".to_owned(),
            sensitivity: 0.8,
            active_evoke_profile_id: Some("profile-2".to_owned()),
            active_dictation_asset_id: Some("dictation.sherpa-zipformer-zh-en".to_owned()),
            generation: 2,
        };
        store.save(&config).unwrap();
        drop(store);
        drop(database);

        let reopened = Database::open(path_text).unwrap();
        assert_eq!(ConfigStore::new(reopened).load().unwrap(), config);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn rejects_invalid_sensitivity_explicitly() {
        let database = Database::open(":memory:").unwrap();
        let store = ConfigStore::new(database);
        let mut config = AppConfig::default();
        config.sensitivity = f32::NAN;
        assert!(store.save(&config).is_err());
    }
}
