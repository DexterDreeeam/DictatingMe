//! State Machine 模块：Runtime 的调度中枢（见 brainstrom/plan.md §4、§5）。

pub mod state;
pub mod event;
pub mod transitions;

pub use event::StateEvent;
pub use state::{HudLight, State, WindowKind};
pub use transitions::{InvalidTransition, StateMachine, Transition, TransitionEffect};
