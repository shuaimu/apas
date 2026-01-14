//! PTY (pseudo-terminal) support for interactive terminal sessions
//!
//! This module provides functionality to spawn processes in a PTY,
//! which preserves interactive terminal features while allowing
//! output capture.

#[cfg(unix)]
mod unix {
    use anyhow::Result;
    use nix::fcntl::{fcntl, FcntlArg, OFlag};
    use nix::pty::{openpty, Winsize};
    use nix::sys::termios::{self, SetArg, Termios};
    use nix::unistd::{close, dup2, read, setsid, write};
    use std::ffi::CString;
    use std::io::stdin;
    use std::os::fd::{AsRawFd, OwnedFd, RawFd};
    use std::path::Path;

    pub struct PtyProcess {
        master_fd: OwnedFd,
        child_pid: u32,
    }

    impl PtyProcess {
        /// Spawn a process in a PTY
        pub fn spawn(program: &str, working_dir: &Path) -> Result<Self> {
            // Get current terminal size
            let winsize = get_terminal_size().unwrap_or(Winsize {
                ws_row: 24,
                ws_col: 80,
                ws_xpixel: 0,
                ws_ypixel: 0,
            });

            // Open a PTY pair
            let pty = openpty(&winsize, None)?;
            let master_fd = pty.master;
            let slave_fd = pty.slave;

            // Fork and exec
            let program_cstr = CString::new(program)?;
            let working_dir_cstr = CString::new(working_dir.to_string_lossy().as_ref())?;

            unsafe {
                match libc::fork() {
                    -1 => {
                        return Err(anyhow::anyhow!("fork failed"));
                    }
                    0 => {
                        // Child process
                        // Close master fd in child
                        let _ = close(master_fd.as_raw_fd());

                        // Create new session and set controlling terminal
                        let _ = setsid();

                        // Set slave as controlling terminal
                        libc::ioctl(slave_fd.as_raw_fd(), libc::TIOCSCTTY, 0);

                        // Duplicate slave fd to stdin, stdout, stderr
                        let _ = dup2(slave_fd.as_raw_fd(), 0);
                        let _ = dup2(slave_fd.as_raw_fd(), 1);
                        let _ = dup2(slave_fd.as_raw_fd(), 2);

                        // Close the original slave fd if it's not one of 0, 1, 2
                        if slave_fd.as_raw_fd() > 2 {
                            let _ = close(slave_fd.as_raw_fd());
                        }

                        // Change to working directory
                        libc::chdir(working_dir_cstr.as_ptr());

                        // Set TERM environment variable
                        let term = CString::new("xterm-256color").unwrap();
                        libc::setenv(
                            CString::new("TERM").unwrap().as_ptr(),
                            term.as_ptr(),
                            1,
                        );

                        // Exec the program
                        libc::execlp(
                            program_cstr.as_ptr(),
                            program_cstr.as_ptr(),
                            std::ptr::null::<libc::c_char>(),
                        );

                        // If exec fails, exit
                        libc::_exit(1);
                    }
                    child_pid => {
                        // Parent process
                        // Close slave fd in parent
                        drop(slave_fd);

                        // Set master to non-blocking
                        let flags = fcntl(master_fd.as_raw_fd(), FcntlArg::F_GETFL)?;
                        let new_flags = OFlag::from_bits_truncate(flags) | OFlag::O_NONBLOCK;
                        fcntl(master_fd.as_raw_fd(), FcntlArg::F_SETFL(new_flags))?;

                        return Ok(Self {
                            master_fd,
                            child_pid: child_pid as u32,
                        });
                    }
                }
            }
        }

        /// Get the master file descriptor for async I/O
        pub fn master_fd(&self) -> RawFd {
            self.master_fd.as_raw_fd()
        }

        /// Write data to the PTY (sends to the child process)
        pub fn write(&self, data: &[u8]) -> Result<usize> {
            match write(&self.master_fd, data) {
                Ok(n) => Ok(n),
                Err(nix::errno::Errno::EAGAIN) => Ok(0),
                Err(e) => Err(e.into()),
            }
        }

        /// Read data from the PTY (output from the child process)
        pub fn read(&self, buf: &mut [u8]) -> Result<usize> {
            match read(self.master_fd.as_raw_fd(), buf) {
                Ok(n) => Ok(n),
                Err(nix::errno::Errno::EAGAIN) => Ok(0),
                Err(nix::errno::Errno::EIO) => Ok(0), // PTY closed
                Err(e) => Err(e.into()),
            }
        }

        /// Check if child process has exited
        pub fn try_wait(&self) -> Option<i32> {
            unsafe {
                let mut status: libc::c_int = 0;
                let result = libc::waitpid(self.child_pid as i32, &mut status, libc::WNOHANG);
                if result > 0 {
                    if libc::WIFEXITED(status) {
                        Some(libc::WEXITSTATUS(status))
                    } else {
                        Some(-1)
                    }
                } else {
                    None
                }
            }
        }

        /// Get the child PID
        pub fn pid(&self) -> u32 {
            self.child_pid
        }
    }

    impl Drop for PtyProcess {
        fn drop(&mut self) {
            // Kill child process if still running
            unsafe {
                libc::kill(self.child_pid as i32, libc::SIGTERM);
            }
        }
    }

    /// Get the current terminal size
    fn get_terminal_size() -> Option<Winsize> {
        unsafe {
            let mut ws: Winsize = std::mem::zeroed();
            if libc::ioctl(0, libc::TIOCGWINSZ, &mut ws) == 0 {
                Some(ws)
            } else {
                None
            }
        }
    }

    /// Set terminal to raw mode and return the original settings
    pub fn set_raw_mode() -> Result<Termios> {
        let stdin_handle = stdin();
        let original = termios::tcgetattr(&stdin_handle)?;
        let mut raw = original.clone();
        termios::cfmakeraw(&mut raw);
        termios::tcsetattr(&stdin_handle, SetArg::TCSANOW, &raw)?;
        Ok(original)
    }

    /// Restore terminal settings
    pub fn restore_terminal(original: &Termios) -> Result<()> {
        let stdin_handle = stdin();
        termios::tcsetattr(&stdin_handle, SetArg::TCSANOW, original)?;
        Ok(())
    }
}

#[cfg(unix)]
pub use unix::*;

#[cfg(not(unix))]
compile_error!("PTY support is only available on Unix systems");
