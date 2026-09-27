//! The same pinned xterm engine as the web UI, embedded without Node or I/O.
//! The serializer saves screen cells; PendingSequence saves an unfinished ANSI
//! sequence/UTF-8 codepoint so the next PTY read continues parsing correctly.
use base64::Engine;
use rquickjs::{Context, Function, Runtime, TypedArray};
use shared::{TerminalCheckpoint, TerminalScreenInfo};
use std::cell::Cell;

pub const MAX_COLS: u16 = 300;
pub const MAX_ROWS: u16 = 120;
pub const CHECKPOINT_INTERVAL_BYTES: usize = 128 * 1024;
pub const MAX_CHECKPOINT_BYTES: usize = 16 * 1024 * 1024;

pub struct TerminalScreen {
    context: Option<Context>,
    cols: u16,
    rows: u16,
    pending: PendingSequence,
    failed: Cell<bool>,
}
impl TerminalScreen {
    pub fn new(cols: u16, rows: u16) -> Self {
        let context = Self::create_engine(cols, rows).ok();
        if context.is_none() {
            tracing::warn!("terminal screen recovery engine unavailable; retaining raw output");
        }
        Self {
            context,
            cols,
            rows,
            pending: PendingSequence::default(),
            failed: Cell::new(false),
        }
    }
    fn create_engine(cols: u16, rows: u16) -> rquickjs::Result<Context> {
        let runtime = Runtime::new()?;
        runtime.set_memory_limit(64 * 1024 * 1024);
        runtime.set_max_stack_size(1024 * 1024);
        let context = Context::full(&runtime)?;
        context.with(|ctx| -> rquickjs::Result<()> {
            ctx.eval::<(), _>(
                "globalThis.self = globalThis; var exports = {}; var module = {exports: {}};",
            )?;
            ctx.eval::<(), _>(include_str!("terminal_engine/bridge.js"))?;
            ctx.eval::<(), _>(include_str!("terminal_engine/xterm.js"))?;
            ctx.eval::<(), _>(include_str!("terminal_engine/serialize.js"))?;
            ctx.globals()
                .get::<_, Function>("createTerminal")?
                .call::<_, ()>((cols, rows))
        })?;
        Ok(context)
    }
    pub fn resize(&mut self, cols: u16, rows: u16) {
        self.cols = cols.clamp(1, MAX_COLS);
        self.rows = rows.clamp(1, MAX_ROWS);
        let Some(context) = &self.context else {
            return;
        };
        if context
            .with(|ctx| {
                ctx.globals()
                    .get::<_, Function>("resizeTerminal")?
                    .call::<_, ()>((self.cols, self.rows))
            })
            .is_err()
        {
            self.failed.set(true);
        }
    }
    pub fn process(&mut self, bytes: &[u8]) {
        if self.failed.get() {
            return;
        }
        self.pending.process(bytes);
        let Some(context) = &self.context else {
            return;
        };
        if context
            .with(|ctx| {
                let data = TypedArray::<u8>::new_copy(ctx.clone(), bytes)?;
                ctx.globals()
                    .get::<_, Function>("processOutput")?
                    .call::<_, ()>((data,))
            })
            .is_err()
        {
            self.failed.set(true);
        }
    }
    pub fn checkpoint(&self, seq: u64) -> Option<TerminalCheckpoint> {
        if self.failed.get() || self.pending.lost {
            return None;
        }
        let context = self.context.as_ref()?;
        let pending = context
            .with(|ctx| {
                ctx.globals()
                    .get::<_, Function>("hasPendingSequence")?
                    .call::<_, bool>(())
            })
            .ok()?;
        if pending && self.pending.sequence.is_empty() && self.pending.utf8.is_empty() {
            return None;
        }
        let data = context
            .with(|ctx| {
                ctx.globals()
                    .get::<_, Function>("serializeTerminal")?
                    .call::<_, String>(())
            })
            .map_err(|_| self.failed.set(true))
            .ok()?;
        let mut bytes = data.into_bytes();
        if pending {
            bytes.extend_from_slice(&self.pending.sequence);
            bytes.extend_from_slice(&self.pending.utf8);
        }
        if bytes.len() > MAX_CHECKPOINT_BYTES {
            return None;
        }
        Some(TerminalCheckpoint {
            screen: TerminalScreenInfo {
                cols: self.cols,
                rows: self.rows,
                checkpoint_seq: seq,
            },
            data_b64: base64::engine::general_purpose::STANDARD.encode(bytes),
        })
    }
}

#[derive(Default, Clone, Copy)]
enum ParseState {
    #[default]
    Ground,
    Escape,
    Csi,
    String {
        osc: bool,
        escaped: bool,
    },
}
#[derive(Default)]
struct PendingSequence {
    state: ParseState,
    sequence: Vec<u8>,
    utf8: Vec<u8>,
    lost: bool,
}
impl PendingSequence {
    fn clear(&mut self) {
        self.state = ParseState::Ground;
        self.sequence.clear();
        self.lost = false;
    }
    fn begin(&mut self, state: ParseState, bytes: &[u8]) {
        self.clear();
        self.state = state;
        self.sequence.extend_from_slice(bytes);
    }
    fn process(&mut self, bytes: &[u8]) {
        for &byte in bytes {
            if self.utf8.is_empty() && byte < 128 {
                self.advance(byte as char, &[byte]);
                continue;
            }
            if !self.utf8.is_empty() && byte & 0xc0 != 0x80 {
                let invalid = std::mem::take(&mut self.utf8);
                self.advance('\u{fffd}', &invalid);
                if byte < 128 {
                    self.advance(byte as char, &[byte]);
                    continue;
                }
            }
            self.utf8.push(byte);
            match std::str::from_utf8(&self.utf8) {
                Ok(text) => {
                    let ch = text.chars().next().unwrap();
                    let encoded = std::mem::take(&mut self.utf8);
                    self.advance(ch, &encoded);
                }
                Err(error) if error.error_len().is_some() => {
                    let invalid = std::mem::take(&mut self.utf8);
                    self.advance('\u{fffd}', &invalid);
                }
                _ => {}
            }
        }
    }
    fn advance(&mut self, ch: char, bytes: &[u8]) {
        if matches!(ch, '\x18' | '\x1a' | '\u{9c}') {
            self.clear();
            return;
        }
        if let ParseState::String { osc, escaped } = self.state {
            if (osc && ch == '\x07') || (escaped && ch == '\\') {
                self.clear();
                return;
            }
            if escaped && ch != '\x1b' {
                self.begin(ParseState::Escape, b"\x1b");
                self.advance(ch, bytes);
                return;
            }
            self.state = ParseState::String {
                osc,
                escaped: ch == '\x1b',
            };
        } else {
            match ch {
                '\x1b' => {
                    self.begin(ParseState::Escape, bytes);
                    return;
                }
                '\u{9b}' => {
                    self.begin(ParseState::Csi, bytes);
                    return;
                }
                '\u{90}' | '\u{98}' | '\u{9d}' | '\u{9e}' | '\u{9f}' => {
                    self.begin(
                        ParseState::String {
                            osc: ch == '\u{9d}',
                            escaped: false,
                        },
                        bytes,
                    );
                    return;
                }
                // C0 actions inside CSI/ESC already ran in xterm. Do not replay them.
                ch if ch < ' ' || ch == '\x7f' => return,
                _ => {}
            }
            match self.state {
                ParseState::Ground => return,
                ParseState::Escape => match ch {
                    '[' => self.state = ParseState::Csi,
                    ']' | 'P' | 'X' | '^' | '_' => {
                        self.state = ParseState::String {
                            osc: ch == ']',
                            escaped: false,
                        }
                    }
                    ' '..='/' => {}
                    _ => {
                        self.clear();
                        return;
                    }
                },
                ParseState::Csi if ('@'..='~').contains(&ch) => {
                    self.clear();
                    return;
                }
                _ => {}
            }
        }
        if !self.lost {
            self.sequence.extend_from_slice(bytes);
            if self.sequence.len() > 64 * 1024 {
                self.sequence.clear();
                self.lost = true;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn inspect(screen: &TerminalScreen) -> String {
        screen.context.as_ref().unwrap().with(|ctx| {
            ctx.globals()
                .get::<_, Function>("inspectTerminal")
                .unwrap()
                .call::<_, String>(())
                .unwrap()
        })
    }
    #[test]
    fn checkpoint_at_every_byte_boundary_preserves_utf8_and_escape_continuations() {
        let text =
            "normal\r\n\x1b[?1049h\x1b[3;4H\x1b[38;2;42;55;66mé日本語 🤖\x1b[?1006h\x1b[?1049lfin"
                .as_bytes();
        for split in 0..text.len() {
            let mut original = TerminalScreen::new(80, 24);
            original.process(&text[..split]);
            let checkpoint = original.checkpoint(5).expect("checkpoint");
            let mut restored = TerminalScreen::new(80, 24);
            restored.process(
                &base64::engine::general_purpose::STANDARD
                    .decode(checkpoint.data_b64)
                    .unwrap(),
            );
            original.process(&text[split..]);
            restored.process(&text[split..]);
            assert_eq!(inspect(&original), inspect(&restored), "split {split}");
        }
    }
}
