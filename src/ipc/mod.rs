//! IPC сервер (i3-msg-совместимый протокол) — безопасная реализация.
//!
//! Безопасность:
//!   1. IPC запускается ТОЛЬКО после успешного входа пользователя, в процессе
//!      запущенном от имени вошедшего пользователя (не root, не superhot-tty).
//!   2. Сокет создаётся с правами 0600 (только владелец).
//!   3. Каждое входящее соединение проверяется через SO_PEERCRED — принимаются
//!      только соединения от того же uid, под которым крутится WM.
//!   4. JSON парсинг через serde_json (замена самописного парсера).
//!   5. Сплиттинг аргументов команд через shell-words crate.

use crate::config::IpcCfg;
use anyhow::{Context, Result};
use std::io::{Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::mpsc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(tag = "type")]
pub enum IpcRequestRaw {
    #[serde(rename = "command")]
    Command { cmd: String },
    #[serde(rename = "get_workspaces")]
    GetWorkspaces,
    #[serde(rename = "get_config")]
    GetConfig,
    #[serde(rename = "get_focused")]
    GetFocused,
    #[serde(rename = "get_version")]
    GetVersion,
}

#[derive(Debug, Clone)]
pub enum IpcRequest {
    Command(String),
    GetWorkspaces,
    GetConfig,
    GetFocused,
    GetVersion,
}

#[derive(Debug, Clone)]
pub enum IpcResponse {
    Ok(String),
    Error(String),
}

pub struct IpcServer {
    pub rx: mpsc::Receiver<(IpcRequest, std::sync::mpsc::Sender<IpcResponse>)>,
    pub socket_path: PathBuf,
    pub running: Arc<AtomicBool>,
}

impl IpcServer {
    /// Запускает IPC сервер.
    ///
    /// ВАЖНО: должен вызываться ТОЛЬКО после login::switch_to_user(), когда
    /// процесс уже работает от имени вошедшего пользователя.
    pub fn start(cfg: &IpcCfg) -> Result<Self> {
        if !cfg.enabled {
            anyhow::bail!("IPC disabled in config");
        }
        let socket_path = resolve_socket_path(cfg)?;
        if socket_path.exists() {
            if let Ok(meta) = std::fs::metadata(&socket_path) {
                use std::os::unix::fs::MetadataExt;
                if meta.uid() == unsafe { libc::getuid() } {
                    let _ = std::fs::remove_file(&socket_path);
                } else {
                    anyhow::bail!("socket {} owned by another uid — refusing to overwrite",
                        socket_path.display());
                }
            }
        }
        if let Some(parent) = socket_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }

        let listener = UnixListener::bind(&socket_path)
            .with_context(|| format!("binding IPC socket at {}", socket_path.display()))?;
        // Принудительно 0600 — даже если cfg.socket_mode задаёт иначе.
        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::Permissions::from_mode(0o600);
        let _ = std::fs::set_permissions(&socket_path, perms);

        let our_uid = unsafe { libc::getuid() };

        use std::os::unix::io::AsRawFd;
        let raw_fd = listener.as_raw_fd();
        unsafe {
            let flags = libc::fcntl(raw_fd, libc::F_GETFL, 0);
            libc::fcntl(raw_fd, libc::F_SETFL, flags | libc::O_NONBLOCK);
        }

        log::info!("IPC socket listening at {} (mode=0600, uid={})",
            socket_path.display(), our_uid);

        let (tx, rx) = mpsc::channel::<(IpcRequest, mpsc::Sender<IpcResponse>)>();
        let running = Arc::new(AtomicBool::new(true));
        let running_clone = running.clone();
        let path_clone = socket_path.clone();

        std::thread::Builder::new()
            .name("ipc-server".into())
            .spawn(move || {
                let mut buf = [0u8; 65536];
                while running_clone.load(Ordering::SeqCst) {
                    let mut fds = [libc::pollfd { fd: raw_fd, events: libc::POLLIN, revents: 0 }];
                    let r = unsafe { libc::poll(fds.as_mut_ptr(), 1, 100) };
                    if r <= 0 { continue; }
                    if fds[0].revents & libc::POLLIN == 0 { continue; }
                    let mut stream = match listener.accept() {
                        Ok((s, _addr)) => {
                            // SO_PEERCRED check — reject if not from our uid.
                            let cred = peer_cred(&s);
                            match cred {
                                Ok(peer_uid) if peer_uid == our_uid => s,
                                Ok(peer_uid) => {
                                    log::warn!("IPC reject: connection from uid={} (our uid={})",
                                        peer_uid, our_uid);
                                    continue;
                                }
                                Err(e) => {
                                    log::warn!("IPC reject: cannot determine peer credentials: {}", e);
                                    let _ = s.shutdown(std::net::Shutdown::Both);
                                    continue;
                                }
                            }
                        }
                        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => continue,
                        Err(e) => {
                            log::warn!("ipc accept error: {}", e);
                            std::thread::sleep(std::time::Duration::from_millis(50));
                            continue;
                        }
                    };
                    let n = match stream.read(&mut buf) {
                        Ok(0) => continue,
                        Ok(n) => n,
                        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => continue,
                        Err(_) => continue,
                    };
                    let raw = String::from_utf8_lossy(&buf[..n]).to_string();
                    let request = parse_request(&raw);
                    let (resp_tx, resp_rx) = mpsc::channel::<IpcResponse>();
                    match request {
                        Ok(req) => {
                            if tx.send((req, resp_tx)).is_err() {
                                break;
                            }
                            let response = match resp_rx.recv_timeout(std::time::Duration::from_secs(5)) {
                                Ok(r) => r,
                                Err(_) => IpcResponse::Error("timeout waiting for WM".into()),
                            };
                            let serialized = serialize_response(&response);
                            let _ = stream.write_all(serialized.as_bytes());
                            let _ = stream.write_all(b"\n");
                        }
                        Err(e) => {
                            let response = IpcResponse::Error(format!("invalid request: {}", e));
                            let serialized = serialize_response(&response);
                            let _ = stream.write_all(serialized.as_bytes());
                            let _ = stream.write_all(b"\n");
                        }
                    }
                }
                log::info!("IPC server thread exit");
                let _ = std::fs::remove_file(&path_clone);
            })?;

        Ok(IpcServer {
            rx,
            socket_path,
            running,
        })
    }

    pub fn poll(&self) -> Option<(IpcRequest, mpsc::Sender<IpcResponse>)> {
        self.rx.try_recv().ok()
    }

    pub fn shutdown(&self) {
        self.running.store(false, Ordering::SeqCst);
    }
}

impl Drop for IpcServer {
    fn drop(&mut self) {
        self.shutdown();
        let _ = std::fs::remove_file(&self.socket_path);
    }
}

/// Get peer credentials via SO_PEERCRED (Linux).
fn peer_cred(stream: &UnixStream) -> Result<u32> {
    use std::os::unix::io::AsRawFd;
    let fd = stream.as_raw_fd();
    let mut cred = libc::ucred { pid: 0, uid: 0, gid: 0 };
    let mut len = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    let ret = unsafe {
        libc::getsockopt(
            fd,
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            &mut cred as *mut _ as *mut _,
            &mut len,
        )
    };
    if ret != 0 {
        anyhow::bail!("getsockopt(SO_PEERCRED) failed: {}", std::io::Error::last_os_error());
    }
    Ok(cred.uid)
}

/// Разбирает JSON запрос через serde_json.
fn parse_request(raw: &str) -> Result<IpcRequest> {
    let parsed: IpcRequestRaw = serde_json::from_str(raw.trim())
        .context("invalid JSON")?;
    Ok(match parsed {
        IpcRequestRaw::Command { cmd } => IpcRequest::Command(cmd),
        IpcRequestRaw::GetWorkspaces => IpcRequest::GetWorkspaces,
        IpcRequestRaw::GetConfig => IpcRequest::GetConfig,
        IpcRequestRaw::GetFocused => IpcRequest::GetFocused,
        IpcRequestRaw::GetVersion => IpcRequest::GetVersion,
    })
}

fn serialize_response(resp: &IpcResponse) -> String {
    match resp {
        IpcResponse::Ok(s) => format!("{{\"status\":\"ok\",\"result\":{}}}", json_escape_string(s)),
        IpcResponse::Error(e) => format!("{{\"status\":\"error\",\"error\":{}}}", json_escape_string(e)),
    }
}

fn json_escape_string(s: &str) -> String {
    serde_json::Value::String(s.to_string()).to_string()
}

/// Определяет путь к сокету.
fn resolve_socket_path(cfg: &IpcCfg) -> Result<PathBuf> {
    if let Some(p) = &cfg.socket_path {
        if !p.is_empty() {
            return Ok(PathBuf::from(crate::config::expand_tilde(p)));
        }
    }
    if let Ok(xdg) = std::env::var("XDG_RUNTIME_DIR") {
        if !xdg.is_empty() {
            return Ok(PathBuf::from(format!("{}/superhot-tty.sock", xdg)));
        }
    }
    let uid = unsafe { libc::getuid() };
    Ok(PathBuf::from(format!("/tmp/superhot-tty-{}.sock", uid)))
}

/// Парсер i3-msg-совместимых команд в Action WM.
pub fn parse_i3_command(cmd: &str) -> (String, Vec<String>) {
    // Use shell-words crate for safe POSIX-ish splitting.
    match shell_words::split(cmd) {
        Ok(parts) => {
            if parts.is_empty() { return (String::new(), vec![]); }
            (parts[0].clone(), parts[1..].to_vec())
        }
        Err(e) => {
            log::warn!("IPC command parse error: {} (cmd={:?})", e, cmd);
            (String::new(), vec![])
        }
    }
}
