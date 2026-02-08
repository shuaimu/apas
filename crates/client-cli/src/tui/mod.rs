//! Tab-based TUI module for Claude CLI
//!
//! Provides a tabbed terminal interface where each tab is an independent
//! Claude session with its own output and input.

mod app;

pub use app::{App, PaneOutput, TuiCommand, TuiEvent};
