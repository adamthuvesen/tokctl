//! TUI rendering.

mod chrome;
mod core;
mod layout;
mod tables;

pub use core::draw;

#[cfg(test)]
pub(crate) use crate::tui::projection::{display_session_rows, display_trend_rows};
#[cfg(test)]
pub(crate) use chrome::{context_text, detail_lines, footer_messages, header_scope};
#[cfg(test)]
pub(crate) use core::breadcrumb_title;
#[cfg(test)]
pub(crate) use layout::BAR_WIDTH;
#[cfg(test)]
pub(crate) use tables::{provider_cost_text, render_bar};

#[cfg(test)]
#[path = "../view_tests.rs"]
mod tests;
