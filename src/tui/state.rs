//! TUI application state: navigation, actions, persistence.

mod apply;
mod persist;
mod refresh;
mod types;

#[cfg(test)]
mod tests;

pub use persist::{load, save};
pub use refresh::*;
pub use types::*;
