//! Privilege separation: login screen runs as unprivileged `superhot-tty`
//! user, PAM authentication happens in the root parent process.
//!
//! Architecture:
//!   1. main() starts as root, opens DRM/input fds (need root for DRM master
//!      and /dev/input/event* access).
//!   2. fork() — child drops to "superhot-tty" system user, runs login UI
//!      using inherited fds. Parent stays root.
//!   3. Child sends credentials to parent via socketpair; parent does PAM
//!      auth and sends back result.
//!   4. On success: child signals parent and exits. Parent waits for child,
//!      then drops to the target user (setuid/setgid/initgroups) and runs WM.
//!   5. IPC server is started AFTER user switch, listening on a socket
//!      accessible only to the logged-in user (mode 0600 + SO_PEERCRED check).
//!
//! The child never runs as root, so a vulnerability in the login UI
//! (rendering, input parsing) cannot directly escalate privileges.
//! The parent runs minimal privileged code (only PAM auth + uid switch).

use anyhow::{Context, Result};
use std::io::{Read, Write};
use std::os::unix::io::RawFd;
use std::os::unix::net::UnixStream;

/// Default unprivileged user for the login screen.
pub const LOGIN_USER: &str = "superhot-tty";

/// Message types exchanged over the privsep socket.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub enum PrivsepMessage {
    /// Child → Parent: authenticate this user with this password.
    AuthRequest {
        username: String,
        password: String,
    },
    /// Parent → Child: auth result.
    AuthResult {
        success: bool,
        error: Option<String>,
        uid: Option<u32>,
        gid: Option<u32>,
        home_dir: Option<String>,
        shell: Option<String>,
    },
    /// Child → Parent: user has closed login screen, exit WM.
    Quit,
}

/// Drop privileges to the `superhot-tty` system user.
///
/// Must be called in the child process immediately after fork, BEFORE any
/// untrusted code runs (rendering, input parsing, etc.).
///
/// Order matters: setgroups() must be called before setgid() to clear
/// supplementary groups inherited from root.
pub fn drop_to_login_user() -> Result<()> {
    drop_to_user(LOGIN_USER)
}

/// Drop privileges to an arbitrary system user by name.
pub fn drop_to_user(username: &str) -> Result<()> {
    let user_c = std::ffi::CString::new(username)
        .map_err(|e| anyhow::anyhow!("invalid username: {}", e))?;
    let pw = unsafe { libc::getpwnam(user_c.as_ptr()) };
    if pw.is_null() {
        anyhow::bail!("system user '{}' not found — install.sh must create it", username);
    }
    // SAFETY: pw is valid pointer returned by getpwnam, points to static storage.
    let pw_ref = unsafe { &*pw };
    let uid = pw_ref.pw_uid;
    let gid = pw_ref.pw_gid;

    // 1. Clear supplementary groups, then set the target group.
    // setgroups(0, NULL) clears the supplementary group list.
    // MUST be done before setgid() to avoid leaving root groups.
    unsafe {
        if libc::setgroups(0, std::ptr::null()) != 0 {
            // EPERM means we're already not root — that's OK in nested drops.
            let err = std::io::Error::last_os_error();
            if err.raw_os_error() != Some(libc::EPERM) {
                anyhow::bail!("setgroups failed: {}", err);
            }
        }
        if libc::setgid(gid) != 0 {
            anyhow::bail!("setgid({}) failed: {}", gid, std::io::Error::last_os_error());
        }
        // initgroups needs the username to load the user's supplementary groups
        // from /etc/group. We pass the actual username, NOT null.
        let init_c = std::ffi::CString::new(username).unwrap();
        if libc::initgroups(init_c.as_ptr(), gid) != 0 {
            log::warn!("initgroups for {} failed: {}", username, std::io::Error::last_os_error());
        }
        if libc::setuid(uid) != 0 {
            anyhow::bail!("setuid({}) failed: {}", uid, std::io::Error::last_os_error());
        }
    }

    // Verify the drop worked — defense in depth.
    let euid = unsafe { libc::geteuid() };
    let egid = unsafe { libc::getegid() };
    if euid != uid || egid != gid {
        anyhow::bail!("privilege drop failed: euid={} (want {}), egid={} (want {})",
            euid, uid, egid, gid);
    }

    // Clear sensitive environment variables inherited from root.
    std::env::remove_var("SUDO_COMMAND");
    std::env::remove_var("SUDO_USER");
    std::env::remove_var("SUDO_UID");
    std::env::remove_var("SUDO_GID");
    std::env::set_var("USER", username);
    std::env::set_var("LOGNAME", username);

    log::info!("dropped privileges to '{}' (uid={}, gid={})", username, uid, gid);
    Ok(())
}

/// Close all file descriptors except the ones in `keep`.
///
/// Used after fork in the child to ensure the unprivileged process cannot
/// access root-only fds (DRM master fd is intentionally kept — it was opened
/// by root and we want to share it; the kernel tracks DRM master per-fd).
///
/// On Linux, uses close_range() if available (Linux 5.9+), otherwise falls
/// back to iterating /proc/self/fd.
#[allow(dead_code)]
pub fn close_all_fds_except(keep: &[RawFd]) {
    let max_keep = keep.iter().copied().max().unwrap_or(2);
    // Try close_range first (Linux 5.9+).
    // close_range(max_keep+1, ~0u32, CLOSE_RANGE_UNSET)
    const CLOSE_RANGE_UNSET: u32 = 0;
    let ret = unsafe {
        libc::syscall(
            436, // SYS_close_range on x86_64
            (max_keep + 1) as u32,
            u32::MAX,
            CLOSE_RANGE_UNSET,
        )
    };
    if ret == 0 {
        return;
    }
    // Fallback: iterate /proc/self/fd.
    let dir = match std::fs::read_dir("/proc/self/fd") {
        Ok(d) => d,
        Err(_) => return,
    };
    for entry in dir.flatten() {
        if let Ok(name) = entry.file_name().into_string() {
            if let Ok(fd) = name.parse::<RawFd>() {
                if !keep.contains(&fd) {
                    unsafe { libc::close(fd); }
                }
            }
        }
    }
}

// === Socketpair message helpers ===

/// Send a length-prefixed message.
fn send_msg(stream: &mut UnixStream, data: &[u8]) -> Result<()> {
    let len = data.len() as u32;
    stream.write_all(&len.to_le_bytes())?;
    stream.write_all(data)?;
    stream.flush()?;
    Ok(())
}

/// Receive a length-prefixed message.
fn recv_msg(stream: &mut UnixStream) -> Result<Vec<u8>> {
    let mut len_buf = [0u8; 4];
    stream.read_exact(&mut len_buf)?;
    let len = u32::from_le_bytes(len_buf) as usize;
    if len > 1024 * 1024 {
        anyhow::bail!("privsep message too large: {} bytes", len);
    }
    let mut buf = vec![0u8; len];
    stream.read_exact(&mut buf)?;
    Ok(buf)
}

/// Send a PrivsepMessage.
pub fn send_message(stream: &mut UnixStream, msg: &PrivsepMessage) -> Result<()> {
    let data = serde_json::to_vec(msg).context("serializing privsep message")?;
    send_msg(stream, &data)
}

/// Receive a PrivsepMessage.
pub fn recv_message(stream: &mut UnixStream) -> Result<PrivsepMessage> {
    let data = recv_msg(stream)?;
    serde_json::from_slice(&data).context("deserializing privsep message")
}
