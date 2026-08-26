//! Attaching a controller CLI to a running project worker.
//!
//! A worker owns the project; a controller only draws it. That asymmetry is
//! what makes this small: `App` consumes `output_rx: Receiver<PaneOutput>` and
//! `command_rx: Receiver<TuiCommand>` and nothing else — the `input_tx` and
//! `event_tx` handed to `App::new` are dropped immediately — so the TUI is
//! read-only and attaching is a snapshot plus two one-way streams. Input
//! already reaches panes from the server, not from the terminal.
//!
//! Headless mode used to `drop` those two receivers because nobody read them.
//! It now fans them out to whoever is attached instead, which is the whole of
//! the worker side.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex};

use anyhow::{bail, Context, Result};

use crate::supervisor::{AttachedTab, ControllerToWorker, WorkerToController};
use crate::tui::{PaneOutput, TuiCommand};

/// Controllers currently attached to this worker.
///
/// A project can be attached to more than once — two terminals, or a terminal
/// and a reconnect — so this is a list, and a write failure removes only the
/// controller that went away.
#[derive(Clone, Default)]
pub struct Attachments {
    inner: Arc<Mutex<Vec<UnixStream>>>,
}

impl Attachments {
    pub fn new() -> Self {
        Self::default()
    }

    fn add(&self, stream: UnixStream) {
        if let Ok(mut streams) = self.inner.lock() {
            streams.push(stream);
        }
    }

    /// Send to every attached controller, dropping any that has gone away.
    ///
    /// A controller exiting is ordinary — someone closed a terminal — so its
    /// broken pipe is not an error for the project, which keeps running.
    pub fn broadcast(&self, message: &WorkerToController) {
        let Ok(mut streams) = self.inner.lock() else {
            return;
        };
        if streams.is_empty() {
            return;
        }
        let Ok(mut line) = serde_json::to_string(message) else {
            return;
        };
        line.push('\n');
        streams.retain_mut(|stream| stream.write_all(line.as_bytes()).is_ok());
    }

    pub fn count(&self) -> usize {
        self.inner.lock().map(|streams| streams.len()).unwrap_or(0)
    }
}

/// Serve attachments for a worker.
///
/// Binds the worker's socket and, for each controller that authenticates,
/// sends the current tabs and then adds it to the broadcast set. The snapshot
/// is taken per connection so a controller attaching to a long-running project
/// starts with the panes that exist rather than with whatever is said next.
pub fn serve(
    socket: &Path,
    credential: String,
    attachments: Attachments,
    snapshot: impl Fn() -> Vec<AttachedTab> + Send + 'static,
) -> Result<()> {
    // A leftover socket file from a crashed worker would make bind fail; it
    // names nothing once that process is gone.
    let _ = std::fs::remove_file(socket);
    let listener = UnixListener::bind(socket)
        .with_context(|| format!("bind worker socket {}", socket.display()))?;
    restrict(socket)?;

    let socket_owned = socket.to_path_buf();
    std::thread::spawn(move || {
        for incoming in listener.incoming() {
            let Ok(mut stream) = incoming else { continue };
            match authenticate(&mut stream, &credential) {
                Ok(()) => {}
                Err(err) => {
                    tracing::warn!(%err, socket = %socket_owned.display(), "attach refused");
                    continue;
                }
            }
            let welcome = WorkerToController::Snapshot { tabs: snapshot() };
            let Ok(mut line) = serde_json::to_string(&welcome) else {
                continue;
            };
            line.push('\n');
            if stream.write_all(line.as_bytes()).is_err() {
                continue;
            }
            attachments.add(stream);
            tracing::info!(attached = attachments.count(), "controller attached");
        }
    });
    Ok(())
}

fn authenticate(stream: &mut UnixStream, expected: &str) -> Result<()> {
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut line = String::new();
    reader.read_line(&mut line)?;
    match serde_json::from_str::<ControllerToWorker>(line.trim()) {
        Ok(ControllerToWorker::Attach { credential }) if credential == expected => Ok(()),
        Ok(_) => bail!("attach credential rejected"),
        Err(err) => bail!("malformed attach request: {err}"),
    }
}

#[cfg(unix)]
fn restrict(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    Ok(())
}

/// Forward a worker's TUI channels to attached controllers.
///
/// This replaces the `drop(output_rx); drop(command_rx)` a headless worker
/// used to do. The receivers are consumed either way; the difference is
/// whether anyone can watch.
pub fn forward_channels(
    attachments: Attachments,
    output_rx: Receiver<PaneOutput>,
    command_rx: Receiver<TuiCommand>,
) {
    let outputs = attachments.clone();
    std::thread::spawn(move || {
        while let Ok(output) = output_rx.recv() {
            outputs.broadcast(&WorkerToController::Output(output));
        }
    });
    std::thread::spawn(move || {
        while let Ok(command) = command_rx.recv() {
            attachments.broadcast(&WorkerToController::Command(command));
        }
    });
}

/// A controller's live connection to a worker.
pub struct Attachment {
    reader: BufReader<UnixStream>,
}

impl Attachment {
    /// Connect and authenticate, returning the connection and the worker's
    /// current tabs.
    pub fn connect(socket: &PathBuf, credential: &str) -> Result<(Self, Vec<AttachedTab>)> {
        let stream = UnixStream::connect(socket)
            .with_context(|| format!("connect worker socket {}", socket.display()))?;
        let mut writer = stream.try_clone()?;
        let mut request = serde_json::to_string(&ControllerToWorker::Attach {
            credential: credential.to_string(),
        })?;
        request.push('\n');
        writer.write_all(request.as_bytes())?;

        let mut reader = BufReader::new(stream);
        let mut line = String::new();
        reader.read_line(&mut line)?;
        match serde_json::from_str::<WorkerToController>(line.trim()) {
            Ok(WorkerToController::Snapshot { tabs }) => Ok((Self { reader }, tabs)),
            Ok(WorkerToController::Ending { reason }) => bail!("worker is ending: {reason}"),
            Ok(_) => bail!("worker did not send a snapshot first"),
            Err(err) => bail!("malformed worker greeting: {err}"),
        }
    }

    /// Next message from the worker, or `None` when the worker has gone.
    pub fn next(&mut self) -> Option<WorkerToController> {
        let mut line = String::new();
        match self.reader.read_line(&mut line) {
            Ok(0) => None,
            Ok(_) => serde_json::from_str(line.trim()).ok(),
            Err(_) => None,
        }
    }
}

/// Run the TUI against a worker instead of against in-process channels.
///
/// The controller owns nothing: it renders what the worker sends and exits
/// when the worker says it is ending or the connection drops. Its own exit
/// never stops the project.
pub fn run_attached_tui(
    mut attachment: Attachment,
    tabs: Vec<AttachedTab>,
    project_dir: String,
) -> Result<()> {
    use std::sync::mpsc::channel;

    let (output_tx, output_rx) = channel::<PaneOutput>();
    let (command_tx, command_rx) = channel::<TuiCommand>();
    let (input_tx, _input_rx) = channel::<(u32, String)>();
    let (event_tx, _event_rx) = channel::<crate::tui::TuiEvent>();

    // The reader thread turns the socket back into the two channels `App`
    // already expects, so the TUI itself needs no knowledge of attachment.
    let ending = Arc::new(Mutex::new(None::<String>));
    let ending_writer = ending.clone();
    std::thread::spawn(move || {
        while let Some(message) = attachment.next() {
            match message {
                WorkerToController::Output(output) => {
                    if output_tx.send(output).is_err() {
                        break;
                    }
                }
                WorkerToController::Command(command) => {
                    if command_tx.send(command).is_err() {
                        break;
                    }
                }
                WorkerToController::Ending { reason } => {
                    if let Ok(mut slot) = ending_writer.lock() {
                        *slot = Some(reason);
                    }
                    break;
                }
                WorkerToController::Snapshot { .. } => {}
            }
        }
        // Dropping the senders ends the TUI's receive loops, which is how a
        // worker going away closes the attachment rather than leaving a
        // screen that looks live.
    });

    let initial = tabs
        .into_iter()
        .map(|tab| (tab.pane_id, tab.label, tab.mode))
        .collect::<Vec<_>>();
    let mut app = crate::tui::App::new(input_tx, output_rx, event_tx, command_rx, initial)
        .with_project_dir(project_dir);
    if let Err(err) = app.run() {
        tracing::error!(%err, "attached TUI error");
    }

    // Reported rather than silent: an attachment that ended because the
    // project was rebooted is not the same as someone closing a terminal.
    if let Ok(slot) = ending.lock() {
        if let Some(reason) = slot.as_ref() {
            println!("Attachment ended: {reason}");
            println!("The project is still managed by this host. Run `apas` again to reattach.");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared::PaneMode;

    fn temp_socket(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("apas-attach-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join(name)
    }

    fn tabs() -> Vec<AttachedTab> {
        vec![AttachedTab {
            pane_id: 7,
            label: "manager".into(),
            mode: PaneMode::Interactive,
        }]
    }

    #[test]
    fn an_attachment_receives_the_panes_that_already_exist() {
        // A project running for hours must not present as empty to someone
        // attaching now.
        let socket = temp_socket("w.sock");
        let attachments = Attachments::new();
        serve(&socket, "secret".into(), attachments.clone(), tabs).unwrap();

        let (mut attachment, snapshot) = Attachment::connect(&socket, "secret").unwrap();
        assert_eq!(snapshot, tabs());

        attachments.broadcast(&WorkerToController::Output(PaneOutput {
            text: "hello".into(),
            pane_id: 7,
        }));
        match attachment.next() {
            Some(WorkerToController::Output(output)) => assert_eq!(output.text, "hello"),
            other => panic!("expected output, got {other:?}"),
        }
    }

    #[test]
    fn two_controllers_watch_the_same_worker() {
        let socket = temp_socket("w.sock");
        let attachments = Attachments::new();
        serve(&socket, "secret".into(), attachments.clone(), tabs).unwrap();

        let (mut first, _) = Attachment::connect(&socket, "secret").unwrap();
        let (mut second, _) = Attachment::connect(&socket, "secret").unwrap();
        // Give the listener thread a moment to register both.
        for _ in 0..50 {
            if attachments.count() == 2 {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert_eq!(
            attachments.count(),
            2,
            "both are attached, neither is a copy"
        );

        attachments.broadcast(&WorkerToController::Command(TuiCommand::RemoveTab {
            pane_id: 7,
        }));
        for attachment in [&mut first, &mut second] {
            match attachment.next() {
                Some(WorkerToController::Command(TuiCommand::RemoveTab { pane_id })) => {
                    assert_eq!(pane_id, 7)
                }
                other => panic!("expected the command, got {other:?}"),
            }
        }
    }

    #[test]
    fn a_wrong_credential_gets_nothing() {
        let socket = temp_socket("w.sock");
        serve(&socket, "secret".into(), Attachments::new(), tabs).unwrap();
        assert!(Attachment::connect(&socket, "guess").is_err());
    }

    #[test]
    fn a_controller_that_goes_away_does_not_take_the_worker_with_it() {
        // Closing a terminal is ordinary; the project keeps running and the
        // worker keeps broadcasting to whoever is left.
        let socket = temp_socket("w.sock");
        let attachments = Attachments::new();
        serve(&socket, "secret".into(), attachments.clone(), tabs).unwrap();

        let (staying, _) = Attachment::connect(&socket, "secret").unwrap();
        let (leaving, _) = Attachment::connect(&socket, "secret").unwrap();
        for _ in 0..50 {
            if attachments.count() == 2 {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        drop(leaving);

        // The first broadcast after the drop is what discovers the closed
        // pipe; the second proves the survivor is unaffected.
        for _ in 0..10 {
            attachments.broadcast(&WorkerToController::Ending {
                reason: "probe".into(),
            });
            if attachments.count() == 1 {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert_eq!(attachments.count(), 1);
        drop(staying);
    }
}
