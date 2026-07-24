//! DictatingMe 可执行入口：组装具体实现、注册 Tauri 命令、启动 Runtime。
//!
//! 具体的模块编排细节见 `dictatingme_runtime::runtime::Runtime`；
//! 本文件只负责"最外层的依赖注入 + Tauri Builder wiring"，不包含业务逻辑。

use std::sync::Mutex;

use dictatingme_runtime::commands;
use dictatingme_runtime::runtime::Runtime;

fn main() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            commands::get_state,
            commands::list_devices,
            commands::set_input_device,
            commands::get_config,
            commands::set_evoke_word,
            commands::set_sensitivity,
            commands::list_history,
            commands::copy_history_text,
            commands::play_history_audio,
            commands::request_background,
            commands::quit_app,
        ])
        .setup(|_app| {
            todo!(
                "组装各模块具体实现（CpalAudioCapture / WindowsInputMonitor / WindowsTextInjector / \
                 EvokeModelEngine / DictationModelEngine / Database / TauriTrayManager / TauriWindowManager），\
                 构造 Runtime，调用 runtime.start()，并通过 app.manage(Mutex::new(runtime)) 托管；\
                 另需创建 MainWindow(隐藏) / HudWindow(隐藏) 与系统托盘。"
            )
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

// 类型提示：Tauri 托管状态的实际类型（供 setup 内部 app.manage 使用，避免遗忘 Mutex 包裹）。
#[allow(dead_code)]
type ManagedRuntime = Mutex<Runtime>;
