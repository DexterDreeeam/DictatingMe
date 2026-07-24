//! Text 模块：流式文字的增量计算与模拟输入（见 brainstrom/plan.md §8.1）。

pub mod diff_engine;
pub mod injector;

pub use diff_engine::TextDiffEngine;
pub use injector::{InjectorError, TextInjector};
