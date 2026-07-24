//! Storage 模块：History Store + Config Store（见 brainstrom/plan.md §9）。

pub mod db;
pub mod history_store;
pub mod config_store;

pub use db::{Database, StorageError};
pub use history_store::{HistoryEntry, HistoryStore, HISTORY_CAPACITY};
pub use config_store::{AppConfig, ConfigStore};
