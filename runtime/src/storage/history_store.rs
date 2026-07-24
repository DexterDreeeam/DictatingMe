//! History Store（见 brainstrom/plan.md §9、§6.4）。
//!
//! 持久化存储最近 20 条听写记录（音频 + 文本 + 时间戳），FIFO 队列，
//! 超过 20 条自动淘汰最旧一条；支持复制文本，不支持删除/搜索（v1 范围明确排除）。

use std::sync::Arc;
use super::db::{Database, StorageError};

/// FIFO 队列容量，固定为 20（见 plan.md §6.4）。
pub const HISTORY_CAPACITY: usize = 20;

/// 一条历史听写记录。
#[derive(Debug, Clone, PartialEq)]
pub struct HistoryEntry {
    pub id: String,
    /// Unix 毫秒时间戳
    pub timestamp_ms: u64,
    pub text: String,
    /// 录音文件路径（Tauri app_data_dir 下）
    pub audio_path: String,
}

/// History Store：对 `HISTORY_CAPACITY` 条记录的 FIFO 持久化封装。
pub struct HistoryStore {
    db: Arc<Database>,
}

impl HistoryStore {
    pub fn new(db: Arc<Database>) -> Self {
        todo!()
    }

    /// 写入一条新记录（Unloading 收尾阶段调用）；
    /// 若超过 `HISTORY_CAPACITY` 条，自动淘汰最旧的一条。
    pub fn append(&mut self, entry: HistoryEntry) -> Result<(), StorageError> {
        todo!()
    }

    /// 按时间倒序返回全部记录（最多 `HISTORY_CAPACITY` 条），供 History 二级页展示。
    pub fn list(&self) -> Result<Vec<HistoryEntry>, StorageError> {
        todo!()
    }

    pub fn count(&self) -> Result<usize, StorageError> {
        todo!()
    }
}
