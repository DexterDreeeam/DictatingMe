//! Runtime 状态机的 5 个状态（见 brainstrom/plan.md §4.1）。
//!
//! 颜色语义（贯穿状态图 / HUD 灯光）：
//!   - 黄 = 可被唤醒（EvokeModel 监听中）
//!   - 绿 = 正在记录用户输入（录音缓冲或流式转写）
//!   - 灰 = 无响应状态（不监听也不记录）

/// DictatingMe Runtime 的状态机状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum State {
    /// MainWindow 已打开（首页三卡片或任意二级页），全局锁定。EvokeModel 停止，显示窗口=MainWindow。
    Configure,
    /// 默认待机状态，等待唤醒词。EvokeModel 运行中。HUD 黄灯。
    Listening,
    /// 唤醒词已触发，DictationModel 正在异步加载；同时立即开始录音并写入 Audio Ring Buffer。HUD 绿灯。
    Loading,
    /// 模型加载完成，流式转写进行中；Ring Buffer 内容无缝作为第一批输入。HUD 绿灯。无静音超时。
    Dictating,
    /// 收尾清理：停止喂音频、丢弃未转换内容、卸载 DictationModel、写入 History。HUD 灰/瞬时。
    Unloading,
}

/// 状态对应显示哪个窗口（MainWindow 与 HudWindow 严格互斥，见 plan.md §3.2）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowKind {
    MainWindow,
    HudWindow,
}

/// HUD 灯光颜色（Configure 态无 HUD，故不含灰色的“显示态”，灰色用 `None` 表达瞬时/隐藏）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HudLight {
    Yellow,
    Green,
    /// 收尾瞬时熄灭 / 无需提示
    Off,
}

impl State {
    /// 该状态下 EvokeModel 是否应运行（仅 `Listening` 为 true，见 plan.md §3.1 互斥运行）。
    pub fn evoke_model_active(&self) -> bool {
        todo!("仅 State::Listening 返回 true，其余状态返回 false")
    }

    /// 该状态下 DictationModel 是否应处于加载/运行状态（Loading/Dictating 为 true）。
    pub fn dictation_model_active(&self) -> bool {
        todo!("Loading 和 Dictating 返回 true，其余返回 false")
    }

    /// 该状态应显示哪个窗口（Configure -> MainWindow，其余 -> HudWindow）。
    pub fn visible_window(&self) -> WindowKind {
        todo!("见 plan.md §4.1 显示窗口列")
    }

    /// 该状态下 HUD 应显示的灯光颜色（仅在 visible_window() == HudWindow 时有意义）。
    pub fn hud_light(&self) -> HudLight {
        todo!("Listening=Yellow, Loading/Dictating=Green, Unloading=Off")
    }
}
