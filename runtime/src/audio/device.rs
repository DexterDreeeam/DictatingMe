//! 音频输入设备管理（对应 MainWindow 的 InputDevice 设置页，见 plan.md §6.2）。

/// 一个可用的音频输入设备。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioDeviceInfo {
    /// 平台相关的设备唯一标识
    pub id: String,
    /// 展示名称，如 "麦克风阵列（Realtek Audio）"
    pub name: String,
    pub is_default: bool,
}

/// 通用音频错误。
#[derive(Debug, Clone, PartialEq)]
pub struct AudioError(pub String);

/// 设备枚举/选择能力（供 InputDevice 页调用）。
pub trait AudioDeviceProvider {
    fn list_devices(&self) -> Result<Vec<AudioDeviceInfo>, AudioError>;
    fn select_device(&mut self, device_id: &str) -> Result<(), AudioError>;
    fn current_device(&self) -> Option<AudioDeviceInfo>;
}
