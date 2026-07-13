//! PTY: открывает pseudo-terminal и запускает shell.
//!
//! Использует libc::openpty + fork+exec. Это надёжнее чем nix-обёртки,
//! так как nix::pty::openpty возвращает OwnedFd, а нам нужен raw fd.

use anyhow::{Context, Result};
use std::os::unix::io::RawFd;
use std::ffi::CString;

pub struct Pty {
    pub master_fd: RawFd,
    pub slave_fd: RawFd,
    pub pid: i32,
    pub cols: u16,
    pub rows: u16,
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
                // TIOCSCTTY = 0x540E
                libc::ioctl(slave, 0x540E, 0);
                libc::dup2(slave, 0);
                libc::dup2(slave, 1);
                libc::dup2(slave, 2);
                if slave > 2 { libc::close(slave); }
                libc::close(master);

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
            slave_fd: -1,
            pid,
            cols,
            rows,
        })
    }

    pub fn resize(&mut self, cols: u16, rows: u16) -> Result<()> {
        self.cols = cols;
        self.rows = rows;
        let ws = libc::winsize { ws_row: rows, ws_col: cols, ws_xpixel: 0, ws_ypixel: 0 };
        let ret = unsafe { libc::ioctl(self.master_fd, 0x5414, &ws) }; // TIOCSWINSZ
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

    pub fn is_alive(&self) -> bool {
        let mut status: i32 = 0;
        let ret = unsafe { libc::waitpid(self.pid, &mut status, libc::WNOHANG) };
        ret == 0
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
