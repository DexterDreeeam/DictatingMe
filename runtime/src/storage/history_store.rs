//! History Store（见 brainstrom/plan.md §9、§6.4）。
//!
//! 持久化存储最近 20 条听写记录（音频 + 文本 + 时间戳），FIFO 队列，
//! 超过 20 条自动淘汰最旧一条；支持复制文本，不支持删除/搜索（v1 范围明确排除）。

use super::db::{Database, StorageError};
use rusqlite::{params, OptionalExtension, TransactionBehavior};
use std::sync::Arc;
use std::{fs, io};

/// FIFO 队列容量，固定为 20（见 plan.md §6.4）。
pub const HISTORY_CAPACITY: usize = 20;

/// 一条历史听写记录。
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryEntry {
    pub id: String,
    /// Unix 毫秒时间戳
    pub timestamp_ms: u64,
    pub text: String,
    /// 录音文件路径（Tauri app_data_dir 下）
    #[serde(skip_serializing)]
    pub audio_path: String,
}

/// History Store：对 `HISTORY_CAPACITY` 条记录的 FIFO 持久化封装。
pub struct HistoryStore {
    db: Arc<Database>,
}

impl HistoryStore {
    pub fn new(db: Arc<Database>) -> Self {
        Self { db }
    }

    /// 写入一条新记录（Unloading 收尾阶段调用）；
    /// 若超过 `HISTORY_CAPACITY` 条，自动淘汰最旧的一条。
    pub fn append(&mut self, entry: HistoryEntry) -> Result<(), StorageError> {
        let timestamp_ms = i64::try_from(entry.timestamp_ms).map_err(|_| {
            StorageError(format!(
                "history timestamp {} exceeds SQLite INTEGER range",
                entry.timestamp_ms
            ))
        })?;

        let mut connection = self.db.connection()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| StorageError(format!("failed to begin history append: {error}")))?;
        transaction
            .execute(
                "
                INSERT INTO history(id, timestamp_ms, text, audio_path)
                VALUES(?1, ?2, ?3, ?4)
                ",
                params![entry.id, timestamp_ms, entry.text, entry.audio_path],
            )
            .map_err(|error| StorageError(format!("failed to insert history entry: {error}")))?;

        let evicted: Vec<(i64, String)> = {
            let mut statement = transaction
                .prepare(
                    "
                    SELECT sequence, audio_path
                    FROM history
                    ORDER BY sequence DESC
                    LIMIT -1 OFFSET ?1
                    ",
                )
                .map_err(|error| {
                    StorageError(format!("failed to prepare history eviction query: {error}"))
                })?;
            let rows = statement
                .query_map(params![HISTORY_CAPACITY as i64], |row| {
                    Ok((row.get(0)?, row.get(1)?))
                })
                .map_err(|error| {
                    StorageError(format!(
                        "failed to query history eviction candidates: {error}"
                    ))
                })?;
            rows.collect::<Result<Vec<_>, _>>().map_err(|error| {
                StorageError(format!(
                    "failed to read history eviction candidates: {error}"
                ))
            })?
        };

        for (sequence, _) in &evicted {
            transaction
                .execute("DELETE FROM history WHERE sequence = ?1", params![sequence])
                .map_err(|error| {
                    StorageError(format!("failed to evict history entry {sequence}: {error}"))
                })?;
        }

        let mut audio_paths_to_remove = Vec::new();
        for (_, audio_path) in evicted {
            let remaining_references: i64 = transaction
                .query_row(
                    "SELECT COUNT(*) FROM history WHERE audio_path = ?1",
                    params![audio_path],
                    |row| row.get(0),
                )
                .map_err(|error| {
                    StorageError(format!(
                        "failed to check retained audio path references: {error}"
                    ))
                })?;
            if remaining_references == 0 && !audio_paths_to_remove.contains(&audio_path) {
                audio_paths_to_remove.push(audio_path);
            }
        }

        transaction
            .commit()
            .map_err(|error| StorageError(format!("failed to commit history append: {error}")))?;

        for audio_path in audio_paths_to_remove {
            match fs::remove_file(&audio_path) {
                Ok(()) => {}
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => {
                    tracing::warn!(
                        %audio_path,
                        %error,
                        "history committed; deferred cleanup could not remove evicted audio"
                    );
                }
            }
        }
        Ok(())
    }

    /// 获取历史记录（按 ID）。
    pub fn get(&self, id: &str) -> Result<Option<HistoryEntry>, StorageError> {
        let connection = self.db.connection()?;
        let raw = connection
            .query_row(
                "
                SELECT id, timestamp_ms, text, audio_path
                FROM history
                WHERE id = ?1
                ",
                params![id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                    ))
                },
            )
            .optional()
            .map_err(|error| StorageError(format!("failed to get history entry {id}: {error}")))?;
        raw.map(history_entry_from_raw).transpose()
    }

    /// 按时间倒序返回全部记录（最多 `HISTORY_CAPACITY` 条），供 History 二级页展示。
    pub fn list(&self) -> Result<Vec<HistoryEntry>, StorageError> {
        let connection = self.db.connection()?;
        let mut statement = connection
            .prepare(
                "
                SELECT id, timestamp_ms, text, audio_path
                FROM history
                ORDER BY timestamp_ms DESC, sequence DESC
                LIMIT ?1
                ",
            )
            .map_err(|error| StorageError(format!("failed to prepare history list: {error}")))?;
        let rows = statement
            .query_map(params![HISTORY_CAPACITY as i64], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                ))
            })
            .map_err(|error| StorageError(format!("failed to query history list: {error}")))?;

        rows.map(|row| {
            row.map_err(|error| StorageError(format!("failed to read history row: {error}")))
                .and_then(history_entry_from_raw)
        })
        .collect()
    }

    pub fn count(&self) -> Result<usize, StorageError> {
        let connection = self.db.connection()?;
        let count: i64 = connection
            .query_row("SELECT COUNT(*) FROM history", [], |row| row.get(0))
            .map_err(|error| StorageError(format!("failed to count history entries: {error}")))?;
        usize::try_from(count)
            .map_err(|_| StorageError(format!("invalid history count returned by SQLite: {count}")))
    }
}

fn history_entry_from_raw(
    (id, timestamp_ms, text, audio_path): (String, i64, String, String),
) -> Result<HistoryEntry, StorageError> {
    let timestamp_ms = u64::try_from(timestamp_ms).map_err(|_| {
        StorageError(format!(
            "history entry {id} has invalid negative timestamp {timestamp_ms}"
        ))
    })?;
    Ok(HistoryEntry {
        id,
        timestamp_ms,
        text,
        audio_path,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(index: usize, timestamp_ms: u64) -> HistoryEntry {
        HistoryEntry {
            id: format!("entry-{index:02}"),
            timestamp_ms,
            text: format!("text {index}"),
            audio_path: format!("missing-audio-{index:02}.wav"),
        }
    }

    #[test]
    fn enforces_fifo_capacity_and_supports_get_list_and_count() {
        let database = Database::open(":memory:").unwrap();
        let mut store = HistoryStore::new(database);
        for index in 0..=HISTORY_CAPACITY {
            store.append(entry(index, index as u64)).unwrap();
        }

        assert_eq!(store.count().unwrap(), HISTORY_CAPACITY);
        assert_eq!(store.get("entry-00").unwrap(), None);
        assert_eq!(store.get("entry-20").unwrap().unwrap().text, "text 20");
        let listed = store.list().unwrap();
        assert_eq!(listed.first().unwrap().id, "entry-20");
        assert_eq!(listed.last().unwrap().id, "entry-01");
    }

    #[test]
    fn list_order_is_deterministic_when_timestamps_match() {
        let database = Database::open(":memory:").unwrap();
        let mut store = HistoryStore::new(database);
        store.append(entry(1, 100)).unwrap();
        store.append(entry(2, 100)).unwrap();
        assert_eq!(
            store
                .list()
                .unwrap()
                .into_iter()
                .map(|entry| entry.id)
                .collect::<Vec<_>>(),
            vec!["entry-02", "entry-01"]
        );
    }

    #[test]
    fn duplicate_ids_propagate_an_error_without_evicting() {
        let database = Database::open(":memory:").unwrap();
        let mut store = HistoryStore::new(database);
        store.append(entry(1, 1)).unwrap();
        assert!(store.append(entry(1, 2)).is_err());
        assert_eq!(store.count().unwrap(), 1);
    }
}
