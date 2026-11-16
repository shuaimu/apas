//! What this host can tell about another process, on Linux and macOS alike.
//!
//! Everything here used to read `/proc`, which macOS does not have. Every
//! check then answered "no such process": `detect_running_daemon` never found
//! the running daemon, so every `apas` started another one, and on the first
//! Mac three daemons with one machine id knocked each other off the server
//! dozens of times an hour. Linux keeps reading `/proc` (cheap, exact argv);
//! elsewhere the same answers come from `kill(pid, 0)` and `ps`, which BSD and
//! procps both provide with the flags used here. The `ps` path is also what
//! the tests drive on Linux, so it cannot rot unseen.

use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// How long one `ps` may take before its answer is given up on. A process
/// table read is fast; this only bounds a wedged `ps`.
const PS_TIMEOUT: Duration = Duration::from_secs(5);

/// Whether `pid` names a live process (a zombie counts until it is reaped).
/// `EPERM` means it exists but belongs to someone else.
pub fn alive(pid: u32) -> bool {
    let Ok(pid) = i32::try_from(pid) else {
        return false;
    };
    if pid <= 0 {
        return false;
    }
    if unsafe { libc::kill(pid, 0) } == 0 {
        return true;
    }
    std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

/// Whether `pid` exists and leads its own process group, as the direct child
/// of a PTY does. A zombie still does on Linux; see [`exited_unreaped`].
pub fn leads_own_group(pid: i32) -> bool {
    // SAFETY: getpgid only reads the process table.
    pid > 0 && unsafe { libc::getpgid(pid) } == pid
}

/// Whether `pid` has exited and only waits to be reaped (a zombie).
///
/// Linux keeps such a process in its process group, so [`alive`] and
/// [`leads_own_group`] both still say yes. `/proc` tells them apart. Where
/// `/proc` cannot be read this answers `false`: nothing may depend on it,
/// and macOS does not need it, since its `getpgid` fails for a zombie.
pub fn exited_unreaped(pid: i32) -> bool {
    cfg!(target_os = "linux") && proc_exited_unreaped(Path::new("/proc"), pid)
}

fn proc_exited_unreaped(root: &Path, pid: i32) -> bool {
    let Ok(stat) = std::fs::read_to_string(root.join(pid.to_string()).join("stat")) else {
        return false;
    };
    // The command name (field 2) may hold spaces and parentheses; the state
    // is the first field after its closing parenthesis.
    stat.rsplit_once(") ")
        .and_then(|(_, fields)| fields.split_whitespace().next())
        .is_some_and(|state| matches!(state, "Z" | "X" | "x"))
}

/// The argument vector of `pid`, or `None` when it is gone or unreadable.
pub fn args(pid: u32) -> Option<Vec<String>> {
    if cfg!(target_os = "linux") {
        if let Some(args) = proc_args(Path::new("/proc"), pid) {
            return Some(args);
        }
        // A /proc that cannot be read (hidepid, a container) still has ps.
    }
    ps_args(pid)
}

/// Every process this user can see, as `(pid, argv)`.
pub fn list() -> Vec<(u32, Vec<String>)> {
    if cfg!(target_os = "linux") {
        if let Some(list) = proc_list(Path::new("/proc")) {
            return list;
        }
    }
    ps_list().unwrap_or_default()
}

/// Resident set size of `pid` in KiB.
pub fn rss_kb(pid: u32) -> Option<u64> {
    if cfg!(target_os = "linux") {
        if let Some(rss) = proc_rss_kb(Path::new("/proc"), pid) {
            return Some(rss);
        }
    }
    ps_rss_kb(pid)
}

/// Whether an argument vector is an `apas … daemon` process: some argument
/// names apas, and one argument is exactly `daemon`.
pub fn is_apas_daemon_args(args: &[String]) -> bool {
    args.iter().any(|arg| arg.contains("apas")) && args.iter().any(|arg| arg == "daemon")
}

/// Whether `pid` is a live `apas … daemon`.
pub fn is_apas_daemon(pid: u32) -> bool {
    alive(pid) && args(pid).is_some_and(|args| is_apas_daemon_args(&args))
}

/// Whether this process may signal `pid`: it is this user's (or this is
/// root). Unlike [`alive`], another user's process does not count.
pub fn is_own(pid: u32) -> bool {
    i32::try_from(pid).is_ok_and(|pid| pid > 0 && unsafe { libc::kill(pid, 0) } == 0)
}

/// The `apas … daemon` processes in `list` other than `me` that `is_own`
/// accepts, by pid. Given the list so it is testable without real daemons.
pub fn other_daemons(
    list: &[(u32, Vec<String>)],
    me: u32,
    is_own: impl Fn(u32) -> bool,
) -> Vec<u32> {
    let mut pids: Vec<u32> = list
        .iter()
        .filter(|(pid, args)| *pid != me && is_apas_daemon_args(args) && is_own(*pid))
        .map(|(pid, _)| *pid)
        .collect();
    pids.sort_unstable();
    pids
}

fn proc_args(root: &Path, pid: u32) -> Option<Vec<String>> {
    let raw = std::fs::read(root.join(pid.to_string()).join("cmdline")).ok()?;
    let args: Vec<String> = raw
        .split(|byte| *byte == 0)
        .filter(|segment| !segment.is_empty())
        .map(|segment| String::from_utf8_lossy(segment).into_owned())
        .collect();
    (!args.is_empty()).then_some(args)
}

fn proc_list(root: &Path) -> Option<Vec<(u32, Vec<String>)>> {
    let entries = std::fs::read_dir(root).ok()?;
    let mut list = Vec::new();
    for entry in entries.flatten() {
        let Some(pid) = entry.file_name().to_str().and_then(|s| s.parse().ok()) else {
            continue;
        };
        if let Some(args) = proc_args(root, pid) {
            list.push((pid, args));
        }
    }
    Some(list)
}

fn proc_rss_kb(root: &Path, pid: u32) -> Option<u64> {
    let status = std::fs::read_to_string(root.join(pid.to_string()).join("status")).ok()?;
    status
        .lines()
        .find_map(|line| line.strip_prefix("VmRSS:"))
        .and_then(|rest| rest.split_whitespace().next())
        .and_then(|value| value.parse().ok())
}

/// `ps` with `args`, from the system locations only: a `ps` earlier on a
/// user's PATH must not decide which process is the daemon.
fn run_ps(args: &[&str]) -> Option<String> {
    let binary = ["/bin/ps", "/usr/bin/ps"]
        .into_iter()
        .find(|path| Path::new(path).is_file())?;
    let mut child = Command::new(binary)
        .args(args)
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    // Read on a thread so a full pipe cannot stall the wait below.
    let mut stdout = child.stdout.take()?;
    let reader = std::thread::spawn(move || {
        let mut out = Vec::new();
        let _ = std::io::Read::read_to_end(&mut stdout, &mut out);
        out
    });
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if start.elapsed() < PS_TIMEOUT => {
                std::thread::sleep(Duration::from_millis(10));
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                tracing::warn!(?args, "ps did not answer; treating the process as unknown");
                return None;
            }
        }
    }
    // `ps -p` exits 1 when the pid does not exist; its empty output says so.
    let out = reader.join().ok()?;
    Some(String::from_utf8_lossy(&out).into_owned())
}

/// `ps` prints the command line joined by spaces, so an argument containing a
/// space is split here. The daemon and headless checks only match exact words
/// (`daemon`, `--headless`, `-d <path>`), and a path with a space is handled by
/// [`ps_args_match_window`].
fn split_ps_args(line: &str) -> Vec<String> {
    line.split_whitespace().map(str::to_string).collect()
}

fn ps_args(pid: u32) -> Option<Vec<String>> {
    let out = run_ps(&["-ww", "-o", "args=", "-p", &pid.to_string()])?;
    let line = out.lines().find(|line| !line.trim().is_empty())?;
    let args = split_ps_args(line);
    (!args.is_empty()).then_some(args)
}

fn ps_list() -> Option<Vec<(u32, Vec<String>)>> {
    let out = run_ps(&["-axww", "-o", "pid=,args="])?;
    Some(
        out.lines()
            .filter_map(|line| {
                let line = line.trim_start();
                let (pid, rest) = line.split_once(char::is_whitespace)?;
                let pid = pid.parse().ok()?;
                let args = split_ps_args(rest);
                (!args.is_empty()).then_some((pid, args))
            })
            .collect(),
    )
}

fn ps_rss_kb(pid: u32) -> Option<u64> {
    run_ps(&["-o", "rss=", "-p", &pid.to_string()])?
        .split_whitespace()
        .next()?
        .parse()
        .ok()
}

/// Whether `args` holds `flag value`. `ps` splits a value containing spaces
/// into several words, so the value may also be the following words joined
/// by single spaces. That leaves one ambiguity a split cannot resolve: a value
/// followed by a positional word. It is read as a longer value, so a
/// `/a/My` project is not mistaken for a `/a/My Project` run.
pub fn args_have_flag_value(args: &[String], flag: &str, value: &str) -> bool {
    args.iter().enumerate().any(|(index, arg)| {
        if arg != flag {
            return false;
        }
        let rest = &args[index + 1..];
        let mut joined = String::new();
        for (used, word) in rest.iter().enumerate() {
            if !joined.is_empty() {
                joined.push(' ');
            }
            joined.push_str(word);
            if joined == value {
                // A following word that is not a flag continues a longer
                // value (`/a/My Project` when looking for `/a/My`).
                return rest.get(used + 1).is_none_or(|next| next.starts_with('-'));
            }
            if joined.len() >= value.len() {
                break;
            }
        }
        false
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Child(std::process::Child);

    impl Drop for Child {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    /// `sh -c '…; :' apas daemon` keeps sh itself alive (the trailing `:`
    /// stops it exec'ing the sleep), with an argv of
    /// `sh -c "sleep 30; :" apas daemon`.
    fn spawn_shell(args: &[&str]) -> Child {
        let child = Command::new("/bin/sh")
            .arg("-c")
            .arg("sleep 30; :")
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn sh");
        // Give exec a moment so ps sees sh's argv, not the forked test.
        let pid = child.id();
        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(5) {
            if ps_args(pid).is_some_and(|args| args.iter().any(|arg| arg == "sleep")) {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        Child(child)
    }

    #[test]
    fn ps_finds_a_daemon_by_its_arguments_without_proc() {
        let daemon = spawn_shell(&["apas", "daemon"]);
        let pid = daemon.0.id();
        assert!(alive(pid));
        let args = ps_args(pid).expect("ps reports a live process");
        assert!(is_apas_daemon_args(&args), "{args:?}");
        // The /proc reader agrees, so Linux and macOS decide alike.
        if cfg!(target_os = "linux") {
            assert!(is_apas_daemon_args(
                &proc_args(Path::new("/proc"), pid).unwrap()
            ));
        }
        let listed = ps_list().expect("ps lists processes");
        assert!(listed
            .iter()
            .any(|(listed_pid, args)| *listed_pid == pid && is_apas_daemon_args(args)));
        assert!(ps_rss_kb(pid).is_some_and(|rss| rss > 0));
    }

    #[test]
    fn ps_does_not_mistake_other_processes_for_the_daemon() {
        let other = spawn_shell(&["apas", "--headless"]);
        let args = ps_args(other.0.id()).unwrap();
        assert!(!is_apas_daemon_args(&args), "{args:?}");
        let unrelated = spawn_shell(&["something", "daemon"]);
        assert!(!is_apas_daemon_args(&ps_args(unrelated.0.id()).unwrap()));
    }

    #[test]
    fn a_reaped_pid_is_neither_alive_nor_a_daemon() {
        let mut child = Command::new("/bin/sh")
            .arg("-c")
            .arg("exit 0")
            .spawn()
            .unwrap();
        let pid = child.id();
        child.wait().unwrap();
        assert!(!alive(pid));
        assert!(ps_args(pid).is_none());
        assert!(!is_apas_daemon(pid));
        assert!(!alive(0));
        assert!(!alive(u32::MAX));
    }

    #[test]
    fn an_exited_process_nobody_reaped_is_told_from_a_running_one() {
        let dir = tempfile::tempdir().unwrap();
        let write = |pid: i32, stat: &str| {
            let at = dir.path().join(pid.to_string());
            std::fs::create_dir_all(&at).unwrap();
            std::fs::write(at.join("stat"), stat).unwrap();
        };
        // A command name with spaces and a parenthesis does not shift the state.
        write(11, "11 (omp) S) Z 1 11 11 0 -1 4194304 0 0\n");
        write(12, "12 (omp (x) R) S 1 12 12 0 -1 4194304 0 0\n");
        write(13, "13 (omp) X 1 13 13 0\n");
        write(14, "garbage");
        assert!(proc_exited_unreaped(dir.path(), 11));
        assert!(!proc_exited_unreaped(dir.path(), 12));
        assert!(proc_exited_unreaped(dir.path(), 13));
        assert!(!proc_exited_unreaped(dir.path(), 14));
        // No /proc entry, or no /proc at all: not known to have exited.
        assert!(!proc_exited_unreaped(dir.path(), 15));
        assert!(!proc_exited_unreaped(Path::new("/nonexistent-proc"), 11));
        assert!(!exited_unreaped(std::process::id() as i32));
        assert!(leads_own_group(unsafe { libc::getpgrp() }));
        assert!(!leads_own_group(0));
        assert!(!leads_own_group(-1));
    }

    #[test]
    fn other_daemons_of_this_user_are_named_and_nothing_else() {
        let words = |line: &str| split_ps_args(line);
        // What `ps` showed on the first Mac, plus this daemon and strangers.
        let list = vec![
            (
                75711,
                words("/Users/me/.local/bin/apas --server wss://x daemon"),
            ),
            (
                41863,
                words("/Users/me/.local/bin/apas --server wss://x daemon"),
            ),
            (
                65324,
                words("/Users/me/.local/bin/apas --server wss://x daemon"),
            ),
            (500, words("/Users/me/.local/bin/apas daemon")),
            (600, words("/Users/me/.local/bin/apas --headless -d /p")),
            (700, words("/usr/local/bin/apas daemon")),
            (800, words("/usr/bin/vim daemon.rs")),
        ];
        let theirs = |pid: u32| pid == 700;
        assert_eq!(
            other_daemons(&list, 500, |pid| !theirs(pid)),
            vec![41863, 65324, 75711]
        );
        assert!(other_daemons(&list[3..4], 500, |_| true).is_empty());
        // The real check: a child of this test is ours, pid 1 is not
        // (unless the tests run as root).
        let child = spawn_shell(&["apas", "daemon"]);
        assert!(is_own(child.0.id()));
        if unsafe { libc::getuid() } != 0 {
            assert!(!is_own(1));
        }
        assert!(!is_own(0));
    }

    #[test]
    fn a_flag_value_with_spaces_is_found_in_ps_words() {
        let words = split_ps_args("/usr/bin/apas --headless -d /Users/me/My Project");
        assert!(args_have_flag_value(&words, "-d", "/Users/me/My Project"));
        assert!(!args_have_flag_value(&words, "-d", "/Users/me/My"));
        assert!(!args_have_flag_value(
            &words,
            "-d",
            "/Users/me/My Project/x"
        ));
        let exact = vec!["apas".to_string(), "-d".to_string(), "/srv/p".to_string()];
        assert!(args_have_flag_value(&exact, "-d", "/srv/p"));
        let flagged = split_ps_args("apas -d /srv/p --headless");
        assert!(args_have_flag_value(&flagged, "-d", "/srv/p"));
    }
}
