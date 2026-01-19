//! Main TUI application for dual-pane mode

use std::io::{self, Stdout};
use std::sync::mpsc::{self, Receiver, Sender};
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

/// Output message for a pane
#[derive(Debug, Clone)]
pub struct PaneOutput {
    pub text: String,
    pub is_deadloop: bool,
}

/// Focus state for input
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Focus {
    Deadloop,
    Interactive,
}

/// Main TUI application state
pub struct App {
    /// Left pane output lines
    deadloop_output: Vec<String>,
    /// Right pane output lines
    interactive_output: Vec<String>,
    /// Current input text (for interactive pane)
    input: String,
    /// Which pane is focused
    focus: Focus,
    /// Scroll offset for deadloop pane
    deadloop_scroll: u16,
    /// Scroll offset for interactive pane
    interactive_scroll: u16,
    /// Channel to send user input
    input_tx: Sender<String>,
    /// Channel to receive output
    output_rx: Receiver<PaneOutput>,
    /// Whether to quit
    should_quit: bool,
}

impl App {
    /// Create a new App with channels for I/O
    pub fn new(input_tx: Sender<String>, output_rx: Receiver<PaneOutput>) -> Self {
        Self {
            deadloop_output: vec!["[Deadloop - Autonomous Worker]".to_string()],
            interactive_output: vec!["[Interactive - Press Enter to send]".to_string()],
            input: String::new(),
            focus: Focus::Interactive,
            deadloop_scroll: 0,
            interactive_scroll: 0,
            input_tx,
            output_rx,
            should_quit: false,
        }
    }

    /// Run the TUI main loop
    pub fn run(&mut self) -> io::Result<()> {
        // Setup terminal
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen)?;
        let backend = CrosstermBackend::new(stdout);
        let mut terminal = Terminal::new(backend)?;

        // Main loop
        while !self.should_quit {
            // Process any pending output
            self.process_output();

            // Draw UI
            terminal.draw(|f| self.draw(f))?;

            // Handle input with timeout
            if event::poll(Duration::from_millis(50))? {
                if let Event::Key(key) = event::read()? {
                    self.handle_key(key.code, key.modifiers);
                }
            }
        }

        // Restore terminal
        disable_raw_mode()?;
        execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
        Ok(())
    }

    /// Process pending output from channel
    fn process_output(&mut self) {
        while let Ok(output) = self.output_rx.try_recv() {
            if output.is_deadloop {
                self.deadloop_output.push(output.text);
                // Auto-scroll to bottom
                if self.deadloop_output.len() > 100 {
                    self.deadloop_scroll = (self.deadloop_output.len() - 100) as u16;
                }
            } else {
                self.interactive_output.push(output.text);
                // Auto-scroll to bottom
                if self.interactive_output.len() > 100 {
                    self.interactive_scroll = (self.interactive_output.len() - 100) as u16;
                }
            }
        }
    }

    /// Handle keyboard input
    fn handle_key(&mut self, code: KeyCode, modifiers: KeyModifiers) {
        // Global shortcuts
        if modifiers.contains(KeyModifiers::CONTROL) {
            match code {
                KeyCode::Char('c') => {
                    self.should_quit = true;
                }
                KeyCode::Char('l') => {
                    self.focus = Focus::Deadloop;
                }
                KeyCode::Char('r') => {
                    self.focus = Focus::Interactive;
                }
                _ => {}
            }
            return;
        }

        // Input handling (only in interactive focus)
        if self.focus == Focus::Interactive {
            match code {
                KeyCode::Enter => {
                    if !self.input.is_empty() {
                        let input = std::mem::take(&mut self.input);
                        self.interactive_output.push(format!("> {}", input));
                        let _ = self.input_tx.send(input);
                    }
                }
                KeyCode::Char(c) => {
                    self.input.push(c);
                }
                KeyCode::Backspace => {
                    self.input.pop();
                }
                KeyCode::Up => {
                    if self.interactive_scroll > 0 {
                        self.interactive_scroll -= 1;
                    }
                }
                KeyCode::Down => {
                    self.interactive_scroll += 1;
                }
                KeyCode::PageUp => {
                    self.interactive_scroll = self.interactive_scroll.saturating_sub(20);
                }
                KeyCode::PageDown => {
                    self.interactive_scroll += 20;
                }
                KeyCode::Esc => {
                    self.input.clear();
                }
                _ => {}
            }
        } else {
            // Scroll controls for deadloop pane
            match code {
                KeyCode::Up => {
                    if self.deadloop_scroll > 0 {
                        self.deadloop_scroll -= 1;
                    }
                }
                KeyCode::Down => {
                    self.deadloop_scroll += 1;
                }
                KeyCode::PageUp => {
                    self.deadloop_scroll = self.deadloop_scroll.saturating_sub(20);
                }
                KeyCode::PageDown => {
                    self.deadloop_scroll += 20;
                }
                _ => {}
            }
        }
    }

    /// Draw the UI
    fn draw(&self, frame: &mut Frame) {
        let area = frame.area();

        // Split into status bar and main content
        let main_layout = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(0), Constraint::Length(1)])
            .split(area);

        // Split main content into two panes
        let panes = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(main_layout[0]);

        // Draw left pane (deadloop)
        self.draw_deadloop_pane(frame, panes[0]);

        // Draw right pane (interactive)
        self.draw_interactive_pane(frame, panes[1]);

        // Draw status bar
        self.draw_status_bar(frame, main_layout[1]);
    }

    /// Draw the deadloop (left) pane
    fn draw_deadloop_pane(&self, frame: &mut Frame, area: Rect) {
        let border_style = if self.focus == Focus::Deadloop {
            Style::default().fg(Color::Yellow)
        } else {
            Style::default().fg(Color::Gray)
        };

        let block = Block::default()
            .title(" Deadloop (Ctrl+L) ")
            .borders(Borders::ALL)
            .border_style(border_style);

        let inner = block.inner(area);
        frame.render_widget(block, area);

        // Render output
        let output_text = self.deadloop_output.join("\n");
        let paragraph = Paragraph::new(output_text)
            .wrap(Wrap { trim: false })
            .scroll((self.deadloop_scroll, 0));
        frame.render_widget(paragraph, inner);
    }

    /// Draw the interactive (right) pane
    fn draw_interactive_pane(&self, frame: &mut Frame, area: Rect) {
        let border_style = if self.focus == Focus::Interactive {
            Style::default().fg(Color::Cyan)
        } else {
            Style::default().fg(Color::Gray)
        };

        let block = Block::default()
            .title(" Interactive (Ctrl+R) ")
            .borders(Borders::ALL)
            .border_style(border_style);

        let inner = block.inner(area);
        frame.render_widget(block, area);

        // Split inner area for output and input
        let layout = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(0), Constraint::Length(3)])
            .split(inner);

        // Render output
        let output_text = self.interactive_output.join("\n");
        let paragraph = Paragraph::new(output_text)
            .wrap(Wrap { trim: false })
            .scroll((self.interactive_scroll, 0));
        frame.render_widget(paragraph, layout[0]);

        // Render input area
        let input_block = Block::default()
            .title(" Input ")
            .borders(Borders::ALL)
            .border_style(if self.focus == Focus::Interactive {
                Style::default().fg(Color::Green)
            } else {
                Style::default().fg(Color::DarkGray)
            });

        let input_inner = input_block.inner(layout[1]);
        frame.render_widget(input_block, layout[1]);

        let input_text = format!("{}_", self.input);
        let input_paragraph = Paragraph::new(input_text);
        frame.render_widget(input_paragraph, input_inner);
    }

    /// Draw the status bar
    fn draw_status_bar(&self, frame: &mut Frame, area: Rect) {
        let focus_text = match self.focus {
            Focus::Deadloop => "DEADLOOP",
            Focus::Interactive => "INTERACTIVE",
        };

        let status = format!(
            " Focus: {} | Ctrl+L/R: Switch | PgUp/PgDn: Scroll | Ctrl+C: Quit ",
            focus_text
        );

        let paragraph = Paragraph::new(status)
            .style(Style::default().bg(Color::DarkGray).fg(Color::White));
        frame.render_widget(paragraph, area);
    }

    /// Add output to deadloop pane
    pub fn add_deadloop_output(&mut self, text: String) {
        self.deadloop_output.push(text);
    }

    /// Add output to interactive pane
    pub fn add_interactive_output(&mut self, text: String) {
        self.interactive_output.push(text);
    }
}

/// Create channels for TUI communication
pub fn create_channels() -> (Sender<String>, Receiver<String>, Sender<PaneOutput>, Receiver<PaneOutput>) {
    let (input_tx, input_rx) = mpsc::channel();
    let (output_tx, output_rx) = mpsc::channel();
    (input_tx, input_rx, output_tx, output_rx)
}
