//! TUI rendering.

mod chrome;
mod core;
mod layout;
mod tables;

pub use core::draw;

#[cfg(test)]
pub(crate) use chrome::{context_text, detail_lines, footer_messages};
#[cfg(test)]
pub(crate) use core::breadcrumb_title;
#[cfg(test)]
pub(crate) use layout::BAR_WIDTH;
#[cfg(test)]
pub(crate) use tables::{display_session_rows, display_trend_rows, render_bar};

#[cfg(test)]
#[path = "../view_tests.rs"]
mod tests;
