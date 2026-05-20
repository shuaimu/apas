//! Tab-based TUI application
//!
//! Supports N tabs, each with its own output buffer, scroll state, and input.
//! Only the active tab is rendered. Tab bar at top, status bar at bottom.

use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::time::Duration;

use crossterm::{
    event::{self, Event, KeyCode, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    prelude::*,
    widgets::{Block, Borders, Paragraph, Wrap},
};
use shared::PaneMode;

/// Output message routed by pane_id
#[derive(Debug, Clone)]
pub struct PaneOutput {
    pub text: String,
    pub pane_id: u32,
}

/// Per-tab state
struct TabState {
    pane_id: u32,
    label: String,
    mode: PaneMode,
    output: Vec<String>,
    input: String,
    scroll: u16,
    auto_scroll: bool,
}

impl TabState {
    fn new(pane_id: u32, label: String, mode: PaneMode) -> Self {
        let init_msg = format!("[{} initialized]", label);
        Self {
            pane_id,
            label,
            mode,
            output: vec![init_msg],
            input: String::new(),
            scroll: 0,
            auto_scroll: true,
        }
    }
}

/// Callback for tab management events from the TUI
#[derive(Debug, Clone)]
pub enum TuiEvent {
    AddTab,
    /// Add a tab with a specific pane_id, label, mode, provider, prompt, and model (from web/server)
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
    },
    CloseTab {
        pane_id: u32,
        /// What to do with the pane's isolated worktree (and its branch) on
        /// close. None means leave both alone — legacy behaviour and the
        /// default for TUI-initiated close (which doesn't currently prompt).
        cleanup_action: Option<shared::PaneCleanupAction>,
    },
    /// Start bot (deadloop) on an existing interactive pane
    StartBot {
        pane_id: u32,
        prompt: Option<String>,
        min_iteration_interval_minutes: Option<u64>,
        effort: Option<String>,
    },
    /// Stop bot on a deadloop pane (revert to interactive)
    StopBot {
        pane_id: u32,
    },
    /// Finalize a graceful stop — deadloop finished its current iteration.
    /// Carries the stop_flag Arc so the handler can verify it belongs to the
    /// current deadloop (prevents stale finalize from killing a newer deadloop).
    FinalizeStopBot {
        pane_id: u32,
        stop_flag: Arc<AtomicBool>,
    },
}

/// Commands sent back to the TUI from event handlers
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

/// Main TUI application state
pub struct App {
    /// All tabs, keyed by pane_id
    tabs: Vec<TabState>,
    /// Index of active tab in `tabs` vec
    active_tab: usize,
    /// Channel to send user input (with pane_id)
    input_tx: Sender<(u32, String)>,
    /// Channel to receive output
    output_rx: Receiver<PaneOutput>,
    /// Channel to send tab management events
    event_tx: Sender<TuiEvent>,
    /// Channel to receive commands (add/remove tabs from event handler)
    command_rx: Receiver<TuiCommand>,
    /// Whether to quit
    should_quit: bool,
    /// Shared shutdown flag (from main)
    shutdown: Option<Arc<AtomicBool>>,
}

impl App {
    /// Create a new App with channels for I/O
    pub fn new(
        input_tx: Sender<(u32, String)>,
        output_rx: Receiver<PaneOutput>,
        event_tx: Sender<TuiEvent>,
        command_rx: Receiver<TuiCommand>,
        initial_tabs: Vec<(u32, String, PaneMode)>,
    ) -> Self {
        let tabs: Vec<TabState> = initial_tabs
            .into_iter()
            .map(|(id, label, mode)| TabState::new(id, label, mode))
            .collect();

        Self {
            tabs,
            active_tab: 0,
            input_tx,
            output_rx,
            event_tx,
            command_rx,
            should_quit: false,
            shutdown: None,
        }
    }

    /// Set the shared shutdown flag
    pub fn with_shutdown(mut self, shutdown: Arc<AtomicBool>) -> Self {
        self.shutdown = Some(shutdown);
        self
    }

    /// Add a new tab dynamically (called from outside the TUI loop)
    pub fn add_tab(&mut self, pane_id: u32, label: String, mode: PaneMode) {
        // Don't add duplicate
        if self.tabs.iter().any(|t| t.pane_id == pane_id) {
            return;
        }
        self.tabs.push(TabState::new(pane_id, label, mode));
    }

    /// Remove a tab dynamically
    pub fn remove_tab(&mut self, pane_id: u32) {
        if let Some(idx) = self.tabs.iter().position(|t| t.pane_id == pane_id) {
            self.tabs.remove(idx);
            // Adjust active_tab if needed
            if self.tabs.is_empty() {
                self.active_tab = 0;
            } else if self.active_tab >= self.tabs.len() {
                self.active_tab = self.tabs.len() - 1;
            }
        }
    }

    /// Check if shutdown has been requested
    fn is_shutdown(&self) -> bool {
        self.shutdown
            .as_ref()
            .map(|s| s.load(Ordering::SeqCst))
            .unwrap_or(false)
    }

    /// Run the TUI main loop
    pub fn run(&mut self) -> io::Result<()> {
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen)?;
        let backend = CrosstermBackend::new(stdout);
        let mut terminal = Terminal::new(backend)?;

        while !self.should_quit && !self.is_shutdown() {
            self.process_commands();
            self.process_output();
            terminal.draw(|f| self.draw(f))?;

            if event::poll(Duration::from_millis(50))? {
                if let Event::Key(key) = event::read()? {
                    self.handle_key(key.code, key.modifiers);
                }
            }
        }

        disable_raw_mode()?;
        execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
        Ok(())
    }

    /// Process pending commands (add/remove tabs from event handler)
    fn process_commands(&mut self) {
        while let Ok(cmd) = self.command_rx.try_recv() {
            match cmd {
                TuiCommand::AddTab {
                    pane_id,
                    label,
                    mode,
                } => {
                    self.add_tab(pane_id, label, mode);
                    // Switch to newly created tab
                    self.active_tab = self.tabs.len() - 1;
                }
                TuiCommand::RemoveTab { pane_id } => {
                    self.remove_tab(pane_id);
                }
                TuiCommand::SetMode { pane_id, mode } => {
                    if let Some(tab) = self.tabs.iter_mut().find(|t| t.pane_id == pane_id) {
                        tab.mode = mode;
                    }
                }
            }
        }
    }

    /// Process pending output from channel
    fn process_output(&mut self) {
        while let Ok(output) = self.output_rx.try_recv() {
            // Find the tab for this pane_id
            if let Some(tab) = self.tabs.iter_mut().find(|t| t.pane_id == output.pane_id) {
                tab.output.push(output.text);
                if tab.auto_scroll {
                    tab.scroll = u16::MAX;
                }
            }
            // If no tab found, output is silently dropped (pane may have been closed)
        }
    }

    /// Handle keyboard input
    fn handle_key(&mut self, code: KeyCode, modifiers: KeyModifiers) {
        // Global shortcuts (Ctrl+*)
        if modifiers.contains(KeyModifiers::CONTROL) {
            match code {
                KeyCode::Char('c') => {
                    self.should_quit = true;
                }
                KeyCode::Char('t') => {
                    // Create new tab
                    let _ = self.event_tx.send(TuiEvent::AddTab);
                }
                KeyCode::Char('w') => {
                    // Close current tab (only if more than 1 tab)
                    if self.tabs.len() > 1 {
                        if let Some(tab) = self.tabs.get(self.active_tab) {
                            let pane_id = tab.pane_id;
                            let _ = self.event_tx.send(TuiEvent::CloseTab {
                                pane_id,
                                cleanup_action: None,
                            });
                        }
                    }
                }
                _ => {}
            }
            return;
        }

        // Tab switching with Alt+number or Tab/Shift+Tab
        if modifiers.contains(KeyModifiers::ALT) {
            if let KeyCode::Char(c) = code {
                if let Some(digit) = c.to_digit(10) {
                    let idx = if digit == 0 { 9 } else { (digit - 1) as usize };
                    if idx < self.tabs.len() {
                        self.active_tab = idx;
                    }
                    return;
                }
            }
        }

        // Tab/Shift+Tab to cycle tabs
        match code {
            KeyCode::BackTab => {
                // Shift+Tab: previous tab
                if !self.tabs.is_empty() {
                    if self.active_tab == 0 {
                        self.active_tab = self.tabs.len() - 1;
                    } else {
                        self.active_tab -= 1;
                    }
                }
                return;
            }
            _ => {}
        }

        // Active tab input
        if let Some(tab) = self.tabs.get_mut(self.active_tab) {
            match code {
                KeyCode::Tab => {
                    // Next tab
                    if !self.tabs.is_empty() {
                        self.active_tab = (self.active_tab + 1) % self.tabs.len();
                    }
                }
                KeyCode::Enter => {
                    // Deadloop tabs don't accept user input
                    if tab.mode == PaneMode::Deadloop {
                        return;
                    }
                    if !tab.input.is_empty() {
                        let input = std::mem::take(&mut tab.input);
                        let _ = self.input_tx.send((tab.pane_id, input));
                    }
                }
                KeyCode::Char(c) => {
                    if tab.mode == PaneMode::Deadloop {
                        return;
                    }
                    tab.input.push(c);
                }
                KeyCode::Backspace => {
                    if tab.mode == PaneMode::Deadloop {
                        return;
                    }
                    tab.input.pop();
                }
                KeyCode::Up => {
                    tab.auto_scroll = false;
                    tab.scroll = tab.scroll.saturating_sub(1);
                }
                KeyCode::Down => {
                    tab.scroll = tab.scroll.saturating_add(1);
                }
                KeyCode::PageUp => {
                    tab.auto_scroll = false;
                    tab.scroll = tab.scroll.saturating_sub(20);
                }
                KeyCode::PageDown => {
                    tab.scroll = tab.scroll.saturating_add(20);
                }
                KeyCode::End => {
                    tab.auto_scroll = true;
                    tab.scroll = u16::MAX;
                }
                KeyCode::Esc => {
                    tab.input.clear();
                }
                _ => {}
            }
        }
    }

    /// Draw the UI
    fn draw(&mut self, frame: &mut Frame) {
        let area = frame.area();

        // Layout: tab bar (1 line) + content + status bar (1 line)
        let layout = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1), // Tab bar
                Constraint::Min(0),    // Content
                Constraint::Length(1), // Status bar
            ])
            .split(area);

        self.draw_tab_bar(frame, layout[0]);
        self.draw_active_tab(frame, layout[1]);
        self.draw_status_bar(frame, layout[2]);
    }

    /// Draw the tab bar
    fn draw_tab_bar(&self, frame: &mut Frame, area: Rect) {
        let mut spans = Vec::new();

        for (i, tab) in self.tabs.iter().enumerate() {
            let is_active = i == self.active_tab;
            let is_bot = tab.mode == PaneMode::Deadloop;

            // Tab number prefix
            let num = format!(" {}:", i + 1);
            let label = if is_bot {
                format!("{} (Bot) ", tab.label)
            } else {
                format!("{} ", tab.label)
            };

            if is_active {
                spans.push(Span::styled(
                    num,
                    Style::default().fg(Color::Black).bg(Color::White).bold(),
                ));
                spans.push(Span::styled(
                    label,
                    Style::default().fg(Color::Black).bg(Color::White).bold(),
                ));
            } else {
                spans.push(Span::styled(num, Style::default().fg(Color::Gray)));
                spans.push(Span::styled(label, Style::default().fg(Color::Gray)));
            }

            // Separator
            spans.push(Span::styled("│", Style::default().fg(Color::DarkGray)));
        }

        // Add "+" hint
        spans.push(Span::styled(" + ", Style::default().fg(Color::DarkGray)));

        let line = Line::from(spans);
        let paragraph = Paragraph::new(line).style(Style::default().bg(Color::DarkGray));
        frame.render_widget(paragraph, area);
    }

    /// Draw the active tab's content
    fn draw_active_tab(&mut self, frame: &mut Frame, area: Rect) {
        if self.tabs.is_empty() {
            let paragraph = Paragraph::new("No tabs open. Press Ctrl+T to create one.")
                .style(Style::default().fg(Color::Gray));
            frame.render_widget(paragraph, area);
            return;
        }

        let tab = &mut self.tabs[self.active_tab];

        let border_style = Style::default().fg(Color::Cyan);
        let block = Block::default()
            .title(format!(" {} ", tab.label))
            .borders(Borders::ALL)
            .border_style(border_style);

        let inner = block.inner(area);
        frame.render_widget(block, area);

        // Split inner area: output + input box
        let content_layout = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(0), Constraint::Length(3)])
            .split(inner);

        // --- Output area ---
        let viewport_width = content_layout[0].width as usize;
        let content_lines: u16 = tab
            .output
            .iter()
            .map(|line| {
                if line.is_empty() || viewport_width == 0 {
                    1
                } else {
                    ((line.chars().count() + viewport_width - 1) / viewport_width).max(1) as u16
                }
            })
            .sum();

        let viewport_height = content_layout[0].height;
        let max_scroll = content_lines.saturating_sub(viewport_height);

        let scroll = if tab.auto_scroll {
            tab.scroll = max_scroll;
            max_scroll
        } else {
            tab.scroll = tab.scroll.min(max_scroll);
            tab.scroll
        };

        let output_text = tab.output.join("\n");
        let paragraph = Paragraph::new(output_text)
            .wrap(Wrap { trim: false })
            .scroll((scroll, 0));
        frame.render_widget(paragraph, content_layout[0]);

        // --- Input area ---
        let is_deadloop = tab.mode == PaneMode::Deadloop;
        let input_block = Block::default()
            .title(if is_deadloop { " Bot Mode " } else { " Input " })
            .borders(Borders::ALL)
            .border_style(Style::default().fg(if is_deadloop {
                Color::DarkGray
            } else {
                Color::Green
            }));

        let input_inner = input_block.inner(content_layout[1]);
        frame.render_widget(input_block, content_layout[1]);

        let input_text = if is_deadloop {
            "Bot running autonomously...".to_string()
        } else {
            format!("{}_", tab.input)
        };
        let input_paragraph = Paragraph::new(input_text).style(if is_deadloop {
            Style::default().fg(Color::DarkGray)
        } else {
            Style::default()
        });
        frame.render_widget(input_paragraph, input_inner);
    }

    /// Draw the status bar
    fn draw_status_bar(&self, frame: &mut Frame, area: Rect) {
        let tab_info = if let Some(tab) = self.tabs.get(self.active_tab) {
            format!("{} ({})", tab.label, tab.pane_id)
        } else {
            "No tabs".to_string()
        };

        let status = format!(
            " Tab: {} | Tab/Shift+Tab: Switch | Alt+N: Go to tab | Ctrl+T: New | Ctrl+W: Close | Ctrl+C: Quit ",
            tab_info
        );

        let paragraph =
            Paragraph::new(status).style(Style::default().bg(Color::DarkGray).fg(Color::White));
        frame.render_widget(paragraph, area);
    }
}

/// Create channels for TUI communication
pub fn create_channels() -> (
    Sender<(u32, String)>,   // input_tx: (pane_id, text)
    Receiver<(u32, String)>, // input_rx
    Sender<PaneOutput>,      // output_tx
    Receiver<PaneOutput>,    // output_rx
    Sender<TuiEvent>,        // event_tx
    Receiver<TuiEvent>,      // event_rx
    Sender<TuiCommand>,      // command_tx
    Receiver<TuiCommand>,    // command_rx
) {
    let (input_tx, input_rx) = mpsc::channel();
    let (output_tx, output_rx) = mpsc::channel();
    let (event_tx, event_rx) = mpsc::channel();
    let (command_tx, command_rx) = mpsc::channel();
    (
        input_tx, input_rx, output_tx, output_rx, event_tx, event_rx, command_tx, command_rx,
    )
}
