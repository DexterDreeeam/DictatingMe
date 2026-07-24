//! Config Store（见 brainstrom/plan.md §9）。
//!
//! 持久化保存 InputDevice / EvokeWord（唤醒词+敏感度）等设置，重启后保留。

use std::sync::Arc;
use super::db::{Database, StorageError};

/// 持久化配置项（对应 MainWindow 的 InputDevice / EvokeWord 二级页）。
#[derive(Debug, Clone, PartialEq)]
pub struct AppConfig {
    /// 当前选中的输入设备 id（对应 `crate::audio::AudioDeviceInfo::id`）
    pub input_device_id: String,
    /// 当前生效唤醒词（同一时间只能有 1 个，见 plan.md §6.3）
    pub evoke_word: String,
    /// 唤醒词敏感度 0.0-1.0
    pub sensitivity: f32,
}

/// Config Store：`AppConfig` 的持久化读写封装。
pub struct ConfigStore {
    db: Arc<Database>,
}

impl ConfigStore {
    pub fn new(db: Arc<Database>) -> Self {
        todo!()
    }

    /// 读取配置；若从未保存过，返回内置默认值。
    pub fn load(&self) -> Result<AppConfig, StorageError> {
        todo!()
    }

    pub fn save(&self, config: &AppConfig) -> Result<(), StorageError> {
        todo!()
    }
}
