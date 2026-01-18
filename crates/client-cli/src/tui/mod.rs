//! TUI module for dual-pane Claude CLI
//!
//! Provides a split-screen terminal interface with:
//! - Left pane: Deadloop (autonomous) output
//! - Right pane: Interactive session output and input

mod app;

pub use app::{App, PaneOutput};
