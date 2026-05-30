//! Per-project file watcher driving event-based deadloop wake-ups.
//!
//! Each project gets one `ProjectFileWatcher` shared across all panes
//! in that project. A pane's between-iteration wait calls
//! [`ProjectFileWatcher::wait_until`] with its own `cursor` (the
//! `Instant` it last finished an iteration). The call returns as soon
//! as any watched file has changed AFTER the cursor, or when the
//! timeout fires — whichever first.
//!
//! Without this the codex/claude deadloop panes burn tokens every
//! `min_iteration_interval_minutes` even when nothing has changed; the
//! Tech-Lead / Reviewer / Developer loops in particular spend most
//! ticks saying "Idle; waiting". After this change, the agent only
//! wakes when there is real work — file edits from the user, from web
//! Approve / Reject, or from a sibling pane's update.
//!
//! Cross-FS robustness: `notify` (inotify on Linux, FSEvents on macOS)
//! doesn't fire reliably on NFS or sshfs mounts. The watcher also runs
//! a coarse 30s mtime poll as a fallback, so loops still wake within
//! ~30 s on networked mounts. The agent's own `timeout` parameter is
//! the final safety net for external state (e.g., PR-status polling)
//! that has no local file proxy.
//!
//! The cursor is the pane's last-iteration-end timestamp, so a pane's
//! own writes don't re-trigger its next iteration — but OTHER panes
//! that haven't iterated past that change DO wake. Tech Lead writing
//! to `team-todo.md` cleanly fans out to Reviewer + Developer.

use notify::{Event, RecommendedWatcher, RecursiveMode, Watcher};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant, SystemTime};

/// Files we treat as wake triggers. Relative to the project working dir.
/// Edits to any of these by any pane (or the user, or the web UI's
/// Approve / Reject which writes through the CLI) wake interested
/// panes in the project.
const WATCHED_FILES: &[&str] = &[
    "team-todo.md",
    ".apas-team.jsonl",
    "project_goal.md",
    ".apas",
];

/// Coarse fallback for filesystems where `notify` doesn't fire (NFS,
/// sshfs). Cheap — just stats a handful of files.
const POLL_INTERVAL: Duration = Duration::from_secs(30);

/// Latest detected change time across all watched files for one project.
/// Shared between the watcher thread, the poll thread, and the per-pane
/// `wait_until` callers (which read it under the lock and block on the
/// condvar for updates).
#[derive(Default)]
struct WatcherState {
    /// Instant of the most recent observed change. `None` means nothing
    /// has changed since the watcher started. Compared against each
    /// pane's cursor.
    last_change: Option<Instant>,
    /// Path of the most recent change, purely for diagnostics in the
    /// returned [`WakeReason`].
    last_change_path: Option<PathBuf>,
    /// Polling-thread cache of (path → mtime). Used to detect mtime
    /// movements between polls when `notify` misses an event.
    mtimes: HashMap<PathBuf, SystemTime>,
    /// Set true on drop so the poll thread can exit cleanly.
    shutdown: bool,
}

pub struct ProjectFileWatcher {
    state: Arc<(Mutex<WatcherState>, Condvar)>,
    /// Keep the watcher alive for the lifetime of this struct; dropped
    /// when the project's last pane is removed and we drop the watcher.
    _watcher: Option<RecommendedWatcher>,
    /// Background poll thread join handle. Set to None after drop.
    _poll_thread: Option<std::thread::JoinHandle<()>>,
}

/// Why a [`ProjectFileWatcher::wait_until`] call returned.
#[derive(Debug, Clone)]
pub enum WakeReason {
    /// A watched file changed after the supplied cursor. `at` is the
    /// detected change time (use as the next cursor).
    FileChanged { at: Instant, path: Option<PathBuf> },
    /// The caller's timeout elapsed without any change.
    Timeout,
    /// Caller-supplied shutdown flag flipped while waiting.
    Shutdown,
}

impl ProjectFileWatcher {
    /// Spin up a watcher for a project working directory. Failures from
    /// the underlying `notify` backend degrade to poll-only (logged at
    /// `info` level) — we don't want a flaky filesystem to crash the CLI.
    pub fn new(working_dir: &Path) -> Self {
        let state = Arc::new((Mutex::new(WatcherState::default()), Condvar::new()));

        // Try to install an inotify/FSEvents watcher. The watched files
        // may not exist yet (e.g., team-todo.md before the first Tech
        // Lead iteration), so we watch the parent dir non-recursively
        // and filter events by filename in the handler.
        let watcher: Option<RecommendedWatcher> = {
            let state_for_cb = state.clone();
            let watched = working_dir.to_path_buf();
            match notify::recommended_watcher(move |res: Result<Event, notify::Error>| {
                let Ok(event) = res else { return };
                let touched = event
                    .paths
                    .iter()
                    .any(|p| is_watched_relative(p, &watched));
                if !touched {
                    return;
                }
                let (lock, cvar) = &*state_for_cb;
                if let Ok(mut s) = lock.lock() {
                    s.last_change = Some(Instant::now());
                    s.last_change_path = event.paths.into_iter().next();
                    cvar.notify_all();
                }
            }) {
                Ok(mut w) => match w.watch(working_dir, RecursiveMode::NonRecursive) {
                    Ok(()) => Some(w),
                    Err(err) => {
                        tracing::info!(
                            dir = %working_dir.display(),
                            "file watcher: failed to watch dir ({err}); falling back to mtime poll only",
                        );
                        None
                    }
                },
                Err(err) => {
                    tracing::info!(
                        "file watcher: backend init failed ({err}); falling back to mtime poll only",
                    );
                    None
                }
            }
        };

        // Always run the mtime poll as a fallback — cheap, covers
        // network mounts where inotify is unreliable.
        let poll_thread = {
            let state_for_poll = state.clone();
            let watched = working_dir.to_path_buf();
            std::thread::Builder::new()
                .name(format!("apas-watch-{}", working_dir.display()))
                .spawn(move || poll_loop(state_for_poll, watched))
                .ok()
        };

        Self {
            state,
            _watcher: watcher,
            _poll_thread: poll_thread,
        }
    }

    /// Block until any watched file changes after `cursor` OR until
    /// `timeout` elapses OR until `shutdown` flips. Returns the new
    /// cursor (via [`WakeReason::FileChanged::at`]) so the caller can
    /// advance.
    ///
    /// `pause` and `stop` are checked periodically so the existing
    /// pause/stop semantics still work — the caller's loop should
    /// re-check them on return.
    pub fn wait_until(
        &self,
        cursor: Option<Instant>,
        timeout: Duration,
        shutdown: &std::sync::atomic::AtomicBool,
        pause: &std::sync::atomic::AtomicBool,
        stop: &std::sync::atomic::AtomicBool,
    ) -> WakeReason {
        let deadline = Instant::now() + timeout;
        let (lock, cvar) = &*self.state;
        let mut guard = match lock.lock() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        };

        loop {
            // Fast path: change already observed past our cursor.
            if let Some(at) = guard.last_change {
                let beats_cursor = cursor.map(|c| at > c).unwrap_or(true);
                if beats_cursor {
                    return WakeReason::FileChanged {
                        at,
                        path: guard.last_change_path.clone(),
                    };
                }
            }
            if shutdown.load(std::sync::atomic::Ordering::SeqCst) {
                return WakeReason::Shutdown;
            }
            if pause.load(std::sync::atomic::Ordering::SeqCst)
                || stop.load(std::sync::atomic::Ordering::SeqCst)
            {
                // Bail out so the caller's loop can handle the pause/stop
                // state. We treat this like a timeout — the caller will
                // re-check and either return or sleep further.
                return WakeReason::Timeout;
            }
            let now = Instant::now();
            if now >= deadline {
                return WakeReason::Timeout;
            }
            // Wake at least every 500 ms so we can re-check shutdown/pause
            // even when the file system stays quiet.
            let chunk = std::cmp::min(deadline - now, Duration::from_millis(500));
            guard = match cvar.wait_timeout(guard, chunk) {
                Ok((g, _)) => g,
                Err(p) => p.into_inner().0,
            };
        }
    }
}

impl Drop for ProjectFileWatcher {
    fn drop(&mut self) {
        let (lock, cvar) = &*self.state;
        if let Ok(mut s) = lock.lock() {
            s.shutdown = true;
        }
        cvar.notify_all();
    }
}

/// Returns true iff `p` resolves to one of our watched files within
/// `project_dir`. `notify` emits absolute paths.
fn is_watched_relative(p: &Path, project_dir: &Path) -> bool {
    let Ok(rel) = p.strip_prefix(project_dir) else {
        return false;
    };
    let Some(name) = rel.to_str() else {
        return false;
    };
    WATCHED_FILES.iter().any(|w| *w == name)
}

fn poll_loop(state: Arc<(Mutex<WatcherState>, Condvar)>, project_dir: PathBuf) {
    let (lock, cvar) = &*state;
    loop {
        // Snapshot the prior mtime map under the lock, then stat outside
        // to avoid holding the mutex during filesystem IO.
        let prior = match lock.lock() {
            Ok(g) => {
                if g.shutdown {
                    return;
                }
                g.mtimes.clone()
            }
            Err(p) => {
                if p.into_inner().shutdown {
                    return;
                }
                HashMap::new()
            }
        };

        let mut current = HashMap::with_capacity(WATCHED_FILES.len());
        let mut changed_path: Option<PathBuf> = None;
        for name in WATCHED_FILES {
            let path = project_dir.join(name);
            if let Ok(meta) = std::fs::metadata(&path) {
                if let Ok(mtime) = meta.modified() {
                    if prior.get(&path) != Some(&mtime) {
                        changed_path.get_or_insert_with(|| path.clone());
                    }
                    current.insert(path, mtime);
                }
            }
        }

        if let Ok(mut g) = lock.lock() {
            // First iteration seeds the mtime map without firing —
            // otherwise we'd treat existing files as a fresh change.
            let seeded = !g.mtimes.is_empty();
            g.mtimes = current;
            if seeded {
                if let Some(p) = changed_path {
                    g.last_change = Some(Instant::now());
                    g.last_change_path = Some(p);
                    cvar.notify_all();
                }
            }
        }

        // Sleep with periodic shutdown checks so we exit promptly on Drop.
        let mut remaining = POLL_INTERVAL;
        while !remaining.is_zero() {
            if let Ok(g) = lock.lock() {
                if g.shutdown {
                    return;
                }
            }
            let chunk = std::cmp::min(remaining, Duration::from_millis(500));
            std::thread::sleep(chunk);
            remaining = remaining.saturating_sub(chunk);
        }
    }
}
