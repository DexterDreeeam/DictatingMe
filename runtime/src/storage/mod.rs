//! Storage 模块：History Store + Config Store（见 brainstrom/plan.md §9）。

pub mod app_paths;
pub mod asset_manager;
pub mod config_store;
pub mod db;
pub mod history_store;
pub mod settings_store;

pub use app_paths::AppPaths;
pub use asset_manager::{
    verify_asset_directory, AssetCatalog, AssetDescriptor, AssetFormat, AssetGroup,
    AssetInstallRequest, AssetKind, AssetManager, AssetPhase, AssetProgress, AssetSummary,
    ProgressCallback,
};
pub use config_store::{AppConfig, ConfigStore};
pub use db::{Database, StorageError};
pub use history_store::{HistoryEntry, HistoryStore, HISTORY_CAPACITY};
pub use settings_store::{
    now_ms, AppReadiness, OperationKind, OperationPhase, OperationProgress, SettingsSnapshot,
    SettingsStore, StoredRecording,
};
