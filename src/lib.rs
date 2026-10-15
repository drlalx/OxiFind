#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

#[cfg(feature = "egui-ui")]
pub mod app;
pub mod backend;
pub mod crawler;
pub mod engine;
pub mod ntfs;
pub mod platform;
pub mod search;
#[cfg(feature = "tui")]
pub mod tui;
pub mod watcher;
