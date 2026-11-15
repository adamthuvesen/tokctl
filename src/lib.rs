#[cfg(any(test, feature = "test-fixtures"))]
pub use store::test_support;

pub mod cli;
pub mod compare;
pub mod cursor_sync;
pub mod dates;
pub mod demo;
pub mod discovery;
pub mod doctor;
pub mod format;
pub mod ingest;
pub mod models;
pub mod paths;
pub mod pricing;
pub mod render;
pub mod repo;
pub mod sources;
pub mod store;
pub mod tui;
pub mod types;
pub mod windows;

pub mod calendar;
