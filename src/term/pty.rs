//! PTY: открывает pseudo-terminal и запускает shell.
//!
//! Использует libc::openpty + fork+exec. Это надёжнее чем nix-обёртки,
//! так как nix::pty::openpty возвращает OwnedFd, а нам нужен raw fd.

use anyhow::Result;
use std::os::unix::io::RawFd;
use std::ffi::CString;

pub struct Pty {
    pub master_fd: RawFd,
    pub pid: i32,
}

impl Pty {
    pub fn spawn(cols: u16, rows: u16, shell: Option<&str>) -> Result<Self> {
        let shell = shell.unwrap_or_else(|| {
            // Используем zsh по умолчанию (пользователь настраивает свой .zshrc).
            // Fallback на bash если zsh не установлен.
            if std::path::Path::new("/usr/bin/zsh").exists() { "zsh" }
            else if std::path::Path::new("/bin/zsh").exists() { "zsh" }
            else { "bash" }
        });

        let mut master: i32 = -1;
        let mut slave: i32 = -1;
        let winsize = libc::winsize {
            ws_row: rows,
            ws_col: cols,
            ws_xpixel: 0,
            ws_ypixel: 0,
        };
        let ret = unsafe {
            libc::openpty(&mut master, &mut slave, std::ptr::null_mut(),
                          std::ptr::null(), &winsize)
        };
        if ret < 0 {
            anyhow::bail!("openpty: {}", std::io::Error::last_os_error());
        }

        // Fork.
        let pid = unsafe { libc::fork() };
        if pid < 0 {
            anyhow::bail!("fork: {}", std::io::Error::last_os_error());
        }

        if pid == 0 {
            // Child.
            unsafe {
                libc::setsid();
                // TIOCSCTTY = 0x540E — make slave our controlling terminal.
                const TIOCSCTTY: libc::c_ulong = 0x540E;
                let _ = libc::ioctl(slave, TIOCSCTTY, 0);
                libc::dup2(slave, 0);
                libc::dup2(slave, 1);
                libc::dup2(slave, 2);
                if slave > 2 { libc::close(slave); }
                libc::close(master);

                // SECURITY: Close ALL inherited file descriptors except stdin/stdout/stderr.
                // Without this, the spawned shell inherits DRM master fd,
                // /dev/input/event* fds, IPC socket, X11 connection, etc.
                // A malicious shell script could read these to escalate privileges.
                //
                // close_range() is Linux 5.9+. Fallback: iterate /proc/self/fd.
                const CLOSE_RANGE_UNSET: u32 = 0;
                let ret = libc::syscall(
                    436, // SYS_close_range on x86_64
                    3u32,
                    u32::MAX,
                    CLOSE_RANGE_UNSET,
                );
                if ret != 0 {
                    if let Ok(dir) = std::fs::read_dir("/proc/self/fd") {
                        for entry in dir.flatten() {
                            if let Ok(name) = entry.file_name().into_string() {
                                if let Ok(fd) = name.parse::<i32>() {
                                    if fd > 2 {
                                        libc::close(fd);
                                    }
                                }
                            }
                        }
                    }
                }

                std::env::set_var("TERM", "xterm-256color");
                std::env::set_var("COLORTERM", "truecolor");
                if std::env::var("LANG").is_err() {
                    std::env::set_var("LANG", "en_US.UTF-8");
                }
                // Подсказываем zsh что мы в superhot-tty (для темы оформления).
                std::env::set_var("SUPERHOT_TTY", "1");

                let shell_c = CString::new(shell).unwrap();
                let arg0 = if shell.ends_with("bash") || shell.ends_with("zsh") {
                    format!("-{}", std::path::Path::new(shell).file_name().unwrap().to_string_lossy())
                } else {
                    shell.to_string()
                };
                let arg0_c = CString::new(arg0).unwrap();
                // Запускаем zsh с interactive login shell.
                let argv: [*const libc::c_char; 2] = [arg0_c.as_ptr(), std::ptr::null()];
                libc::execvp(shell_c.as_ptr(), argv.as_ptr() as *const *const _);
                libc::perror(CString::new("execvp").unwrap().as_ptr());
                libc::_exit(127);
            }
        }

        // Parent.
        unsafe { libc::close(slave); }
        log::info!("spawned shell '{}' (pid={}, master_fd={})", shell, pid, master);
        Ok(Pty {
            master_fd: master,
            pid,
        })
    }

    /// Resize PTY window. Sends TIOCSWINSZ to the slave so the child
    /// process receives SIGWINCH and updates its terminal dimensions.
    #[allow(dead_code)] // currently unused, kept for future resize support
    pub fn resize(&mut self, cols: u16, rows: u16) -> Result<()> {
        let ws = libc::winsize { ws_row: rows, ws_col: cols, ws_xpixel: 0, ws_ypixel: 0 };
        const TIOCSWINSZ: libc::c_ulong = 0x5414;
        let ret = unsafe { libc::ioctl(self.master_fd, TIOCSWINSZ, &ws) };
        if ret < 0 {
            anyhow::bail!("TIOCSWINSZ: {}", std::io::Error::last_os_error());
        }
        Ok(())
    }

    pub fn read(&self, buf: &mut [u8]) -> Result<usize> {
        let n = unsafe { libc::read(self.master_fd, buf.as_mut_ptr() as *mut _, buf.len()) };
        if n < 0 {
            // EIO = child exited.
            if std::io::Error::last_os_error().raw_os_error() == Some(libc::EIO) {
                return Ok(0);
            }
            anyhow::bail!("pty read: {}", std::io::Error::last_os_error());
        }
        Ok(n as usize)
    }

    pub fn write(&self, buf: &[u8]) -> Result<usize> {
        let n = unsafe { libc::write(self.master_fd, buf.as_ptr() as *const _, buf.len()) };
        if n < 0 {
            anyhow::bail!("pty write: {}", std::io::Error::last_os_error());
        }
        Ok(n as usize)
    }
}

impl Drop for Pty {
    fn drop(&mut self) {
        unsafe {
            libc::kill(self.pid, libc::SIGHUP);
            libc::close(self.master_fd);
        }
    }
}
