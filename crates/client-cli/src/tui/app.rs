//! Minimal status TUI.
//!
//! Replaces the previous tabbed chat / input UI with a single-screen status
//! report. The web app is the canonical surface for chat and pane control;
//! the CLI runs in the background and only surfaces what a user might miss
//! from the web — version, per-pane error counts, the last error line.
//!
//! The public types (`PaneOutput`, `TuiEvent`, `TuiCommand`, `App`) are
//! preserved so the rest of the crate continues to compile unchanged. The
//! interactive plumbing the old App had (keyboard handling, scrollback,
//! tab navigation, input forwarding) is gone.

use std::collections::BTreeMap;
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crossterm::{
    event::{self, Event, KeyCode, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    prelude::*,
    widgets::{Block, Borders, Paragraph, Row, Table, Wrap},
};
use shared::PaneMode;

const CURRENT_VERSION: &str = env!("APAS_VERSION");

/// Output message routed by pane_id.
///
/// Other modules still emit `PaneOutput`s for status lines and stderr
/// captures; the status app counts errors and keeps the most recent text
/// per pane, but throws away the full scrollback (the web has the real
/// chat).
#[derive(Debug, Clone)]
pub struct PaneOutput {
    pub text: String,
    pub pane_id: u32,
}

/// Callback for tab management events from the TUI (kept for ABI
/// compatibility with the rest of the crate — the new status app emits
/// none of these, but `AddTabWithConfig` is still the canonical pane
/// creation event used by web/server spawn paths).
#[derive(Debug, Clone)]
pub enum TuiEvent {
    AddTab,
    AddTabWithConfig {
        pane_id: u32,
        label: String,
        claude_session_id: uuid::Uuid,
        mode: PaneMode,
        provider: shared::Provider,
        prompt: Option<String>,
        min_iteration_interval_minutes: Option<u64>,
        model: Option<String>,
        effort: Option<String>,
        worktree_path: Option<String>,
        initial_input: Option<String>,
        role: Option<String>,
        goal: Option<String>,
        backstory: Option<String>,
        plan_review_mode: shared::PlanReviewMode,
        managed: bool,
        try_resume_first: bool,
        /// Agent (headless worker) vs Terminal (pty-hosted TUI). Terminal
        /// tabs skip the agent spawn entirely — see `handle_tui_events`.
        kind: shared::PaneKind,
    },
    CloseTab {
        pane_id: u32,
        cleanup_action: Option<shared::PaneCleanupAction>,
    },
    StartBot {
        pane_id: u32,
        prompt: Option<String>,
        min_iteration_interval_minutes: Option<u64>,
        effort: Option<String>,
    },
    StopBot {
        pane_id: u32,
    },
    FinalizeStopBot {
        pane_id: u32,
        stop_flag: Arc<AtomicBool>,
    },
}

/// Commands sent into the TUI from event handlers — used by dual_pane to
/// reflect pane lifecycle (add / remove / mode change) into the status
/// display.
#[derive(Debug)]
pub enum TuiCommand {
    AddTab {
        pane_id: u32,
        label: String,
        mode: PaneMode,
    },
    RemoveTab {
        pane_id: u32,
    },
    SetMode {
        pane_id: u32,
        mode: PaneMode,
    },
}

/// Per-pane summary tracked by the status app. We deliberately do NOT
/// keep the scrollback — only the count of error lines and the most
/// recent text, so memory stays bounded regardless of how chatty the
/// agents are.
struct PaneSummary {
    label: String,
    mode: PaneMode,
    error_count: u32,
    last_text: String,
    last_activity: Instant,
}

impl PaneSummary {
    fn new(label: String, mode: PaneMode) -> Self {
        Self {
            label,
            mode,
            error_count: 0,
            last_text: String::new(),
            last_activity: Instant::now(),
        }
    }
}

/// Classify a PaneOutput line as "looks like an error" so we can count
/// it. Conservative — bare `error` mentions in normal agent text would
/// otherwise inflate the count. The patterns below match how the rest
/// of the crate actually surfaces errors in PaneOutput:
///   - `format!("[Error spawning … ]", …)` from format_spawn_error
///   - `[Failed to send input to agent: …]` / `[Failed to spawn agent: …]`
///   - `[stderr] Error:` from the codex/claude stderr reader
fn looks_like_error(text: &str) -> bool {
    let t = text.trim_start();
    t.starts_with("[Error")
        || t.starts_with("[Failed")
        || t.starts_with("[stderr] Error")
        || t.starts_with("[stderr] error")
}

/// Status TUI. Public methods preserved from the old App so the call
/// site in dual_pane.rs continues to compile without changes:
///   `App::new(input_tx, output_rx, event_tx, command_rx, initial_tabs)`
///   `.with_shutdown(shutdown)`
///   `.run()`
pub struct App {
    panes: BTreeMap<u32, PaneSummary>,
    output_rx: Receiver<PaneOutput>,
    command_rx: Receiver<TuiCommand>,
    /// Last error line seen across any pane, with the pane id it came
    /// from. Surfaced in the footer so the user can spot a recent
    /// failure at a glance.
    last_error: Option<(u32, String, Instant)>,
    shutdown: Option<Arc<AtomicBool>>,
    /// Wall-clock at which the app started — used to render uptime.
    started_at: Instant,
    /// Cumulative output-line counter, just for the header so the
    /// user can confirm activity is flowing.
    output_lines_total: u64,
}

impl App {
    /// Create a new status App. The `input_tx` and `event_tx` arguments
    /// are kept for signature compatibility with the old tabbed TUI but
    /// are no longer used — `input_tx` would carry user-typed text from
    /// the TUI (no chat input here), and the status app never produces
    /// `TuiEvent`s itself. Both are dropped immediately.
    pub fn new(
        input_tx: Sender<(u32, String)>,
        output_rx: Receiver<PaneOutput>,
        event_tx: Sender<TuiEvent>,
        command_rx: Receiver<TuiCommand>,
        initial_tabs: Vec<(u32, String, PaneMode)>,
    ) -> Self {
        drop(input_tx);
        drop(event_tx);
        let mut panes = BTreeMap::new();
        for (id, label, mode) in initial_tabs {
            panes.insert(id, PaneSummary::new(label, mode));
        }
        Self {
            panes,
            output_rx,
            command_rx,
            last_error: None,
            shutdown: None,
            started_at: Instant::now(),
            output_lines_total: 0,
        }
    }

    pub fn with_shutdown(mut self, shutdown: Arc<AtomicBool>) -> Self {
        self.shutdown = Some(shutdown);
        self
    }

    /// Drive the status loop until shutdown is requested or the user
    /// presses Ctrl+C / q. Render tick is ~250 ms which is plenty for
    /// a status display and keeps CPU near zero.
    pub fn run(&mut self) -> io::Result<()> {
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen)?;
        let backend = CrosstermBackend::new(stdout);
        let mut terminal = Terminal::new(backend)?;

        let mut last_render = Instant::now() - Duration::from_secs(1);
        let render_interval = Duration::from_millis(250);
        let input_poll = Duration::from_millis(50);

        loop {
            if self.is_shutting_down() {
                break;
            }

            // Drain channels without blocking so the render stays fresh.
            self.drain_commands();
            self.drain_outputs();

            // Keyboard: Ctrl+C / q / Esc quits and flips the shared
            // shutdown flag so the server thread tears down cleanly.
            if event::poll(input_poll)? {
                if let Event::Key(key) = event::read()? {
                    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
                    let quit = matches!(key.code, KeyCode::Char('q') | KeyCode::Esc)
                        || (ctrl && matches!(key.code, KeyCode::Char('c')));
                    if quit {
                        if let Some(flag) = &self.shutdown {
                            flag.store(true, Ordering::SeqCst);
                        }
                        break;
                    }
                }
            }

            if last_render.elapsed() >= render_interval {
                terminal.draw(|f| self.render(f))?;
                last_render = Instant::now();
            }
        }

        // Restore terminal state.
        disable_raw_mode()?;
        execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
        terminal.show_cursor()?;
        Ok(())
    }

    fn is_shutting_down(&self) -> bool {
        self.shutdown
            .as_ref()
            .map(|f| f.load(Ordering::SeqCst))
            .unwrap_or(false)
    }

    fn drain_commands(&mut self) {
        while let Ok(cmd) = self.command_rx.try_recv() {
            match cmd {
                TuiCommand::AddTab {
                    pane_id,
                    label,
                    mode,
                } => {
                    let entry = self
                        .panes
                        .entry(pane_id)
                        .or_insert_with(|| PaneSummary::new(label.clone(), mode));
                    entry.label = label;
                }
                TuiCommand::RemoveTab { pane_id } => {
                    self.panes.remove(&pane_id);
                }
                TuiCommand::SetMode { pane_id, mode } => {
                    if let Some(p) = self.panes.get_mut(&pane_id) {
                        p.mode = mode;
                    }
                }
            }
        }
    }

    fn drain_outputs(&mut self) {
        while let Ok(out) = self.output_rx.try_recv() {
            self.output_lines_total = self.output_lines_total.saturating_add(1);
            let entry = self.panes.entry(out.pane_id).or_insert_with(|| {
                PaneSummary::new(format!("pane {}", out.pane_id), PaneMode::Interactive)
            });
            entry.last_activity = Instant::now();
            entry.last_text = out.text.clone();
            if looks_like_error(&out.text) {
                entry.error_count = entry.error_count.saturating_add(1);
                self.last_error = Some((out.pane_id, out.text, Instant::now()));
            }
        }
    }

    fn render(&self, f: &mut Frame) {
        let area = f.area();
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3), // header
                Constraint::Min(1),    // pane table
                Constraint::Length(4), // footer
            ])
            .split(area);

        // ---- Header
        let pane_count = self.panes.len();
        let errs: u32 = self.panes.values().map(|p| p.error_count).sum();
        let uptime = humanize_duration(self.started_at.elapsed());
        let header_lines = vec![
            Line::from(vec![
                Span::styled(
                    format!("apas v{}", CURRENT_VERSION),
                    Style::default().add_modifier(Modifier::BOLD),
                ),
                Span::raw("   "),
                Span::styled(
                    format!("uptime {}", uptime),
                    Style::default().fg(Color::DarkGray),
                ),
            ]),
            Line::from(vec![
                Span::raw(format!("panes: {}   ", pane_count)),
                Span::styled(
                    format!("errors: {}   ", errs),
                    if errs > 0 {
                        Style::default().fg(Color::Red)
                    } else {
                        Style::default().fg(Color::Green)
                    },
                ),
                Span::styled(
                    format!("output lines: {}", self.output_lines_total),
                    Style::default().fg(Color::DarkGray),
                ),
            ]),
        ];
        let header = Paragraph::new(header_lines).block(Block::default().borders(Borders::BOTTOM));
        f.render_widget(header, chunks[0]);

        // ---- Pane table
        let header_row = Row::new(vec!["id", "label", "mode", "errs", "last activity"]).style(
            Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::BOLD),
        );
        let rows: Vec<Row> = self
            .panes
            .iter()
            .map(|(id, p)| {
                let mode = match p.mode {
                    PaneMode::Interactive => "interactive",
                    PaneMode::Deadloop => "deadloop",
                };
                let last = humanize_age(p.last_activity.elapsed());
                let err_style = if p.error_count > 0 {
                    Style::default().fg(Color::Red)
                } else {
                    Style::default().fg(Color::Green)
                };
                Row::new(vec![
                    ratatui::widgets::Cell::from(id.to_string()),
                    ratatui::widgets::Cell::from(p.label.clone()),
                    ratatui::widgets::Cell::from(mode),
                    ratatui::widgets::Cell::from(format!("{}", p.error_count)).style(err_style),
                    ratatui::widgets::Cell::from(last),
                ])
            })
            .collect();
        let widths = [
            Constraint::Length(6),
            Constraint::Min(16),
            Constraint::Length(12),
            Constraint::Length(6),
            Constraint::Length(14),
        ];
        let table = Table::new(rows, widths)
            .header(header_row)
            .block(Block::default().borders(Borders::NONE));
        f.render_widget(table, chunks[1]);

        // ---- Footer: last error + hint
        let footer_lines = if let Some((pid, text, at)) = &self.last_error {
            vec![
                Line::from(vec![
                    Span::styled("last error: ", Style::default().fg(Color::Red)),
                    Span::raw(format!("pane {} · {} · ", pid, humanize_age(at.elapsed()))),
                    Span::raw(truncate(text, 200)),
                ]),
                Line::from(Span::styled(
                    "Ctrl+C / q  quit · web UI is the canonical surface",
                    Style::default().fg(Color::DarkGray),
                )),
            ]
        } else {
            vec![
                Line::from(Span::styled(
                    "no errors recorded",
                    Style::default().fg(Color::Green),
                )),
                Line::from(Span::styled(
                    "Ctrl+C / q  quit · web UI is the canonical surface",
                    Style::default().fg(Color::DarkGray),
                )),
            ]
        };
        let footer = Paragraph::new(footer_lines)
            .wrap(Wrap { trim: true })
            .block(Block::default().borders(Borders::TOP));
        f.render_widget(footer, chunks[2]);
    }
}

fn humanize_duration(d: Duration) -> String {
    let secs = d.as_secs();
    if secs < 60 {
        format!("{}s", secs)
    } else if secs < 3600 {
        format!("{}m{:02}s", secs / 60, secs % 60)
    } else if secs < 86400 {
        format!("{}h{:02}m", secs / 3600, (secs % 3600) / 60)
    } else {
        format!("{}d{}h", secs / 86400, (secs % 86400) / 3600)
    }
}

fn humanize_age(d: Duration) -> String {
    let secs = d.as_secs();
    if secs < 1 {
        "now".to_string()
    } else if secs < 60 {
        format!("{}s ago", secs)
    } else if secs < 3600 {
        format!("{}m ago", secs / 60)
    } else if secs < 86400 {
        format!("{}h ago", secs / 3600)
    } else {
        format!("{}d ago", secs / 86400)
    }
}

fn truncate(s: &str, max_bytes: usize) -> &str {
    if s.len() <= max_bytes {
        return s;
    }
    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn looks_like_error_matches_known_patterns() {
        assert!(looks_like_error("[Error spawning claude]"));
        assert!(looks_like_error("[Failed to spawn agent: foo]"));
        assert!(looks_like_error("[stderr] Error: thread/resume failed"));
        assert!(!looks_like_error("> hi"));
        assert!(!looks_like_error("[Thinking...]"));
        assert!(!looks_like_error(
            "normal output mentioning error in passing"
        ));
    }

    #[test]
    fn humanize_age_buckets() {
        assert_eq!(humanize_age(Duration::from_millis(500)), "now");
        assert_eq!(humanize_age(Duration::from_secs(45)), "45s ago");
        assert_eq!(humanize_age(Duration::from_secs(120)), "2m ago");
        assert_eq!(humanize_age(Duration::from_secs(7200)), "2h ago");
    }

    #[test]
    fn truncate_respects_char_boundary() {
        // 3-byte ellipsis at byte index 1..4; truncate at 3 would land
        // mid-codepoint without the boundary walk.
        let s = "x…y";
        let t = truncate(s, 3);
        assert!(s.starts_with(t));
        assert!(t.len() <= 3);
    }
}
