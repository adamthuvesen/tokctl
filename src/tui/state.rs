//! TUI application state: navigation, actions, persistence.

#[path = "state/apply.rs"]
mod apply;
#[path = "state/persist.rs"]
mod persist;
#[path = "state/refresh.rs"]
mod refresh;
#[path = "state/types.rs"]
mod types;

#[cfg(test)]
#[path = "state/tests.rs"]
mod tests;

pub use persist::{load, save};
pub use refresh::*;
pub use types::*;
