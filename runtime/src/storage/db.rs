//! 持久化存储的底层连接封装（见 brainstrom/plan.md §9），供 HistoryStore / ConfigStore 共用。
//! 具体实现待定（如 rusqlite），此处仅定义接口形状。

use std::sync::Arc;

/// 存储层错误。
#[derive(Debug, Clone, PartialEq)]
pub struct StorageError(pub String);

/// 数据库连接句柄（内部实现细节留待具体实现，如 rusqlite::Connection 的封装）。
pub struct Database {
    /// 数据文件路径（Tauri app_data_dir 下，如 "history.db"）
    path: String,
}

impl Database {
    /// 打开（不存在则创建）数据库文件。
    pub fn open(path: &str) -> Result<Arc<Self>, StorageError> {
        todo!()
    }

    /// 执行建表/升级脚本（History、Config 两张表，见 plan.md §9）。
    pub fn migrate(&self) -> Result<(), StorageError> {
        todo!()
    }

    pub fn path(&self) -> &str {
        todo!()
    }
}
