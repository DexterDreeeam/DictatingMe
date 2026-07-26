#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

//! DictatingMe 可执行入口：组装具体实现、注册 Tauri 命令、启动 Runtime。
//!
//! 具体的模块编排细节见 `dictatingme_runtime::runtime::RuntimeCore`；
//! 本文件只负责"最外层的依赖注入 + Tauri Builder wiring"，不包含业务逻辑。

fn main() {
    dictatingme_runtime::run();
}
