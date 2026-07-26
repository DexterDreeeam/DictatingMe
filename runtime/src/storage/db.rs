//! 持久化存储的底层连接封装（见 brainstrom/plan.md §9），供 HistoryStore / ConfigStore 共用。
//! 具体实现待定（如 rusqlite），此处仅定义接口形状。

use rusqlite::{Connection, TransactionBehavior};
use std::fmt;
use std::sync::{Arc, Mutex, MutexGuard};

/// 存储层错误。
#[derive(Debug, Clone, PartialEq)]
pub struct StorageError(pub String);

impl fmt::Display for StorageError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for StorageError {}

/// 数据库连接句柄（内部实现细节留待具体实现，如 rusqlite::Connection 的封装）。
pub struct Database {
    /// 数据文件路径（Tauri app_data_dir 下，如 "history.db"）
    path: String,
    connection: Mutex<Connection>,
}

impl Database {
    /// 打开（不存在则创建）数据库文件。
    pub fn open(path: &str) -> Result<Arc<Self>, StorageError> {
        let connection = Connection::open(path)
            .map_err(|error| StorageError(format!("failed to open database at {path}: {error}")))?;
        let database = Arc::new(Self {
            path: path.to_owned(),
            connection: Mutex::new(connection),
        });
        database.migrate()?;
        Ok(database)
    }

    /// 执行建表/升级脚本（History、Config 两张表，见 plan.md §9）。
    pub fn migrate(&self) -> Result<(), StorageError> {
        let mut connection = self.connection()?;
        let version: i64 = connection
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .map_err(|error| StorageError(format!("failed to read database version: {error}")))?;
        if version > 3 {
            return Err(StorageError(format!(
                "database version {version} is newer than supported version 3"
            )));
        }

        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| StorageError(format!("failed to begin migration: {error}")))?;
        if version < 1 {
            transaction
                .execute_batch(
                    "
                    CREATE TABLE IF NOT EXISTS history (
                        sequence INTEGER PRIMARY KEY AUTOINCREMENT,
                        id TEXT NOT NULL UNIQUE,
                        timestamp_ms INTEGER NOT NULL CHECK(timestamp_ms >= 0),
                        text TEXT NOT NULL,
                        audio_path TEXT NOT NULL
                    );
                    CREATE INDEX IF NOT EXISTS history_timestamp_sequence
                        ON history(timestamp_ms DESC, sequence DESC);

                    CREATE TABLE IF NOT EXISTS config (
                        singleton INTEGER PRIMARY KEY CHECK(singleton = 1),
                        input_device_id TEXT NOT NULL,
                        evoke_word TEXT NOT NULL,
                        sensitivity REAL NOT NULL
                    );
                    ",
                )
                .map_err(|error| StorageError(format!("failed to apply migration 1: {error}")))?;
        }
        if version < 2 {
            transaction
                .execute_batch(
                    "
                    UPDATE config
                    SET evoke_word = '你好'
                    WHERE singleton = 1 AND evoke_word = '小助手';
                    PRAGMA user_version = 2;
                    ",
                )
                .map_err(|error| StorageError(format!("failed to apply migration 2: {error}")))?;
        }
        if version < 3 {
            transaction
                .execute_batch(
                    "
                    ALTER TABLE config ADD COLUMN active_evoke_profile_id TEXT;
                    ALTER TABLE config ADD COLUMN active_dictation_asset_id TEXT;
                    ALTER TABLE config ADD COLUMN generation INTEGER NOT NULL DEFAULT 0;

                    CREATE TABLE IF NOT EXISTS evoke_profiles (
                        id TEXT PRIMARY KEY,
                        mode TEXT NOT NULL,
                        phrase TEXT NOT NULL,
                        threshold REAL NOT NULL,
                        artifact_json TEXT NOT NULL,
                        required_asset_ids_json TEXT NOT NULL,
                        created_at_ms INTEGER NOT NULL,
                        state TEXT NOT NULL
                    );

                    CREATE TABLE IF NOT EXISTS evoke_setups (
                        id TEXT PRIMARY KEY,
                        mode TEXT NOT NULL,
                        phrase TEXT NOT NULL,
                        phase TEXT NOT NULL,
                        required_recordings INTEGER NOT NULL,
                        operation_id TEXT,
                        error TEXT,
                        created_at_ms INTEGER NOT NULL
                    );

                    CREATE TABLE IF NOT EXISTS evoke_recordings (
                        setup_id TEXT NOT NULL,
                        recording_index INTEGER NOT NULL,
                        audio_path TEXT NOT NULL,
                        quality_json TEXT NOT NULL,
                        PRIMARY KEY(setup_id, recording_index),
                        FOREIGN KEY(setup_id) REFERENCES evoke_setups(id) ON DELETE CASCADE
                    );

                    CREATE TABLE IF NOT EXISTS operations (
                        id TEXT PRIMARY KEY,
                        kind TEXT NOT NULL,
                        phase TEXT NOT NULL,
                        progress REAL,
                        message TEXT,
                        error TEXT,
                        updated_at_ms INTEGER NOT NULL
                    );

                    INSERT OR IGNORE INTO evoke_profiles(
                        id, mode, phrase, threshold, artifact_json,
                        required_asset_ids_json, created_at_ms, state
                    ) VALUES(
                        'default-text-nihao', 'text', '你好', 0.5,
                        '{\"kind\":\"text\",\"keyword_syntax\":\"n ǐ h ǎo\"}',
                        '[]', 0, 'active'
                    );
                    INSERT OR REPLACE INTO evoke_profiles(
                        id, mode, phrase, threshold, artifact_json,
                        required_asset_ids_json, created_at_ms, state
                    )
                    SELECT
                        'migrated-text-v3', 'text', evoke_word, 0.5,
                        '{\"kind\":\"text\",\"keyword_syntax\":\"\"}',
                        '[]', 0, 'active'
                    FROM config
                    WHERE singleton = 1 AND evoke_word <> '你好';
                    UPDATE evoke_profiles
                    SET state = 'retired'
                    WHERE id = 'default-text-nihao'
                      AND EXISTS(
                        SELECT 1 FROM config
                        WHERE singleton = 1 AND evoke_word <> '你好'
                      );
                    UPDATE config
                    SET active_evoke_profile_id = COALESCE(
                        active_evoke_profile_id,
                        CASE WHEN evoke_word <> '你好'
                            THEN 'migrated-text-v3'
                            ELSE 'default-text-nihao'
                        END
                    );
                    PRAGMA user_version = 3;
                    ",
                )
                .map_err(|error| StorageError(format!("failed to apply migration 3: {error}")))?;
        }
        transaction
            .commit()
            .map_err(|error| StorageError(format!("failed to commit migrations: {error}")))
    }

    pub fn path(&self) -> &str {
        &self.path
    }

    pub(crate) fn connection(&self) -> Result<MutexGuard<'_, Connection>, StorageError> {
        self.connection
            .lock()
            .map_err(|_| StorageError("database connection mutex was poisoned".to_owned()))
    }
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
            .join(format!("db-migration-test-{}-{id}.db", std::process::id()))
    }

    #[test]
    fn open_runs_idempotent_migrations() {
        let database = Database::open(":memory:").unwrap();
        assert_eq!(database.path(), ":memory:");
        database.migrate().unwrap();

        let connection = database.connection().unwrap();
        let version: i64 = connection
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap();
        assert_eq!(version, 3);
    }

    #[test]
    fn migration_2_updates_the_previous_default_wake_word() {
        let path = database_path();
        let connection = Connection::open(&path).unwrap();
        connection
            .execute_batch(
                "
                CREATE TABLE config (
                    singleton INTEGER PRIMARY KEY CHECK(singleton = 1),
                    input_device_id TEXT NOT NULL,
                    evoke_word TEXT NOT NULL,
                    sensitivity REAL NOT NULL
                );
                INSERT INTO config(singleton, input_device_id, evoke_word, sensitivity)
                VALUES(1, '', '小助手', 0.65);
                PRAGMA user_version = 1;
                ",
            )
            .unwrap();
        drop(connection);

        let database = Database::open(path.to_str().unwrap()).unwrap();
        let connection = database.connection().unwrap();
        let wake_word: String = connection
            .query_row(
                "SELECT evoke_word FROM config WHERE singleton = 1",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(wake_word, "你好");
        drop(connection);
        drop(database);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn migration_3_preserves_a_custom_wake_phrase() {
        let path = database_path();
        let connection = Connection::open(&path).unwrap();
        connection
            .execute_batch(
                "
                CREATE TABLE config (
                    singleton INTEGER PRIMARY KEY CHECK(singleton = 1),
                    input_device_id TEXT NOT NULL,
                    evoke_word TEXT NOT NULL,
                    sensitivity REAL NOT NULL
                );
                INSERT INTO config(singleton, input_device_id, evoke_word, sensitivity)
                VALUES(1, '', '天气助手', 0.65);
                PRAGMA user_version = 2;
                ",
            )
            .unwrap();
        drop(connection);

        let database = Database::open(path.to_str().unwrap()).unwrap();
        let connection = database.connection().unwrap();
        let (profile_id, phrase): (String, String) = connection
            .query_row(
                "
                SELECT c.active_evoke_profile_id, p.phrase
                FROM config c
                JOIN evoke_profiles p ON p.id = c.active_evoke_profile_id
                WHERE c.singleton = 1
                ",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(profile_id, "migrated-text-v3");
        assert_eq!(phrase, "天气助手");
        drop(connection);
        drop(database);
        std::fs::remove_file(path).unwrap();
    }
}
