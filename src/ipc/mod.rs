//! IPC сервер (i3-msg-совместимый протокол).
//!
//! Архитектура:
//!   1. WM создаёт UNIX-доменный сокет по пути из cfg.ipc.socket_path
//!      (по умолчанию $XDG_RUNTIME_DIR/superhot-tty.sock или /tmp/superhot-tty-$UID.sock).
//!   2. Слушает в отдельном потоке, принимает соединения, читает JSON-запросы,
//!      отправляет JSON-ответы.
//!   3. Запросы складываются в mpsc канал (`IpcRequest`). Главный цикл WM
//!      опрашивает канал каждый кадр и исполняет команды.
//!
//! Поддерживаемые команды:
//!   { "type": "command", "cmd": "workspace 2" }            — выполнить действие
//!   { "type": "command", "cmd": "exec firefox" }           — запустить программу
//!   { "type": "command", "cmd": "reload" }                 — reload config
//!   { "type": "command", "cmd": "quit" }                   — выйти из WM
//!   { "type": "get_workspaces" }                           — список ws
//!   { "type": "get_config" }                               — текущий конфиг (TOML)
//!   { "type": "get_focused" }                              — какой тайл в фокусе
//!
//! Команды (поле cmd) — i3-msg-совместимый синтаксис:
//!   workspace N            — переключиться на ws N (1..10)
//!   workspace next|prev    — следующее/предыдущее
//!   move to workspace N    — переместить окно на ws N
//!   exec CMD               — запустить CMD в фоне
//!   exec --no-startup-id CMD
//!   kill                   — закрыть сфокусированное окно
//!   reload                 — перечитать конфиг
//!   restart                — перезапустить WM (в нашей реализации = quit)
//!   quit                   — выйти
//!   split horizontal|vertical
//!   layout toggle          — переключить layout
//!   focus left|right|up|down
//!   fullscreen toggle
//!
//! Ответы:
//!   { "status": "ok", "result": <value> }
//!   { "status": "error", "error": "message" }

use crate::config::IpcCfg;
use anyhow::{Context, Result};
use std::io::{Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::mpsc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// Команда от IPC клиента, передаваемая в главный цикл WM.
#[derive(Debug, Clone)]
pub enum IpcRequest {
    /// Выполнить команду (i3-msg синтаксис).
    Command(String),
    /// Запросить текущий список workspaces.
    GetWorkspaces,
    /// Запросить текущий конфиг как TOML строку.
    GetConfig,
    /// Запросить info о сфокусированном тайле.
    GetFocused,
    /// Запросить версию WM.
    GetVersion,
}

/// Ответ от WM на IPC запрос (отправляется обратно клиенту).
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
    /// Запускает IPC сервер. Возвращает receiver для приёма запросов.
    pub fn start(cfg: &IpcCfg) -> Result<Self> {
        if !cfg.enabled {
            anyhow::bail!("IPC disabled in config");
        }
        let socket_path = resolve_socket_path(cfg)?;
        // Удаляем старый сокет если есть.
        let _ = std::fs::remove_file(&socket_path);
        // Создаём родительскую директорию.
        if let Some(parent) = socket_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }

        let listener = UnixListener::bind(&socket_path)
            .with_context(|| format!("binding IPC socket at {}", socket_path.display()))?;
        // Права доступа.
        let mode = cfg.socket_mode & 0o777;
        if mode != 0 {
            use std::os::unix::fs::PermissionsExt;
            let perms = std::fs::Permissions::from_mode(mode);
            let _ = std::fs::set_permissions(&socket_path, perms);
        }

        // Делаем listener non-blocking.
        use std::os::unix::io::AsRawFd;
        let raw_fd = listener.as_raw_fd();
        unsafe {
            let flags = libc::fcntl(raw_fd, libc::F_GETFL, 0);
            libc::fcntl(raw_fd, libc::F_SETFL, flags | libc::O_NONBLOCK);
        }

        log::info!("IPC socket listening at {} (mode={:o})", socket_path.display(), mode);

        let (tx, rx) = mpsc::channel::<(IpcRequest, mpsc::Sender<IpcResponse>)>();
        let running = Arc::new(AtomicBool::new(true));
        let running_clone = running.clone();
        let path_clone = socket_path.clone();

        std::thread::Builder::new()
            .name("ipc-server".into())
            .spawn(move || {
                let mut buf = [0u8; 65536];
                while running_clone.load(Ordering::SeqCst) {
                    // poll with timeout 100ms.
                    let mut fds = [libc::pollfd { fd: raw_fd, events: libc::POLLIN, revents: 0 }];
                    let r = unsafe { libc::poll(fds.as_mut_ptr(), 1, 100) };
                    if r <= 0 { continue; }
                    if fds[0].revents & libc::POLLIN == 0 { continue; }
                    // Принимаем соединение.
                    let mut stream = match listener.accept() {
                        Ok((s, _)) => s,
                        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => continue,
                        Err(e) => {
                            log::warn!("ipc accept error: {}", e);
                            std::thread::sleep(std::time::Duration::from_millis(50));
                            continue;
                        }
                    };
                    // Читаем запрос.
                    let n = match stream.read(&mut buf) {
                        Ok(0) => continue,
                        Ok(n) => n,
                        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => continue,
                        Err(_) => continue,
                    };
                    let raw = String::from_utf8_lossy(&buf[..n]).to_string();
                    let request = parse_request(&raw);
                    // Подготавливаем stream для ответа.
                    let (resp_tx, resp_rx) = mpsc::channel::<IpcResponse>();
                    if let Some(req) = request {
                        if tx.send((req, resp_tx)).is_err() {
                            // WM упал — выходим.
                            break;
                        }
                        // Ждём ответ от WM (с таймаутом 5 сек).
                        let response = match resp_rx.recv_timeout(std::time::Duration::from_secs(5)) {
                            Ok(r) => r,
                            Err(_) => IpcResponse::Error("timeout waiting for WM".into()),
                        };
                        let serialized = serialize_response(&response);
                        let _ = stream.write_all(serialized.as_bytes());
                        let _ = stream.write_all(b"\n");
                    } else {
                        let response = IpcResponse::Error(format!("invalid request: {}", raw));
                        let serialized = serialize_response(&response);
                        let _ = stream.write_all(serialized.as_bytes());
                        let _ = stream.write_all(b"\n");
                    }
                }
                log::info!("IPC server thread exit");
                // Удаляем сокет.
                let _ = std::fs::remove_file(&path_clone);
            })?;

        Ok(IpcServer {
            rx,
            socket_path,
            running,
        })
    }

    /// Неблокирующе опрашивает очередь запросов. Возвращает None если запросов нет.
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

/// Разбирает JSON запрос.
fn parse_request(raw: &str) -> Option<IpcRequest> {
    // Упрощённый JSON парсер — без внешних зависимостей.
    // Ищем поля "type" и "cmd".
    let trimmed = raw.trim();
    if !trimmed.starts_with('{') || !trimmed.ends_with('}') { return None; }
    let inner = &trimmed[1..trimmed.len()-1];
    let typ = json_extract_string(inner, "type")?;
    match typ.as_str() {
        "command" => {
            let cmd = json_extract_string(inner, "cmd").unwrap_or_default();
            Some(IpcRequest::Command(cmd))
        }
        "get_workspaces" => Some(IpcRequest::GetWorkspaces),
        "get_config" => Some(IpcRequest::GetConfig),
        "get_focused" => Some(IpcRequest::GetFocused),
        "get_version" => Some(IpcRequest::GetVersion),
        _ => None,
    }
}

fn json_extract_string(s: &str, key: &str) -> Option<String> {
    let needle = format!("\"{}\"", key);
    let pos = s.find(&needle)?;
    let after = &s[pos + needle.len()..];
    // Пропускаем пробелы и двоеточие.
    let after = after.trim_start();
    let after = after.strip_prefix(':')?;
    let after = after.trim_start();
    if !after.starts_with('"') { return None; }
    let after = &after[1..];
    let end = after.find('"')?;
    Some(after[..end].to_string())
}

fn serialize_response(resp: &IpcResponse) -> String {
    match resp {
        IpcResponse::Ok(s) => format!("{{\"status\":\"ok\",\"result\":{}}}", json_escape_string(s)),
        IpcResponse::Error(e) => format!("{{\"status\":\"error\",\"error\":{}}}", json_escape_string(e)),
    }
}

fn json_escape_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Определяет путь к сокету.
/// 1. Если cfg.socket_path задан — используем его.
/// 2. Иначе $XDG_RUNTIME_DIR/superhot-tty.sock.
/// 3. Иначе /tmp/superhot-tty-$UID.sock.
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
/// Возвращает (action_name, args).
pub fn parse_i3_command(cmd: &str) -> (String, Vec<String>) {
    let parts: Vec<String> = shell_split(cmd);
    if parts.is_empty() { return (String::new(), vec![]); }
    (parts[0].clone(), parts[1..].to_vec())
}

/// Простой shell- splitter (без полноценного POSIX-shell парсера, но покрывает
/// кавычки и базовые escape).
pub fn shell_split(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut in_squote = false;
    let mut in_dquote = false;
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if in_squote {
            if c == '\'' { in_squote = false; }
            else { cur.push(c); }
        } else if in_dquote {
            if c == '"' { in_dquote = false; }
            else if c == '\\' {
                if let Some(&nc) = chars.peek() {
                    cur.push(nc);
                    chars.next();
                }
            } else { cur.push(c); }
        } else {
            match c {
                '\'' => in_squote = true,
                '"' => in_dquote = true,
                ' ' | '\t' | '\n' => {
                    if !cur.is_empty() {
                        out.push(std::mem::take(&mut cur));
                    }
                }
                '\\' => {
                    if let Some(&nc) = chars.peek() {
                        cur.push(nc);
                        chars.next();
                    }
                }
                _ => cur.push(c),
            }
        }
    }
    if !cur.is_empty() { out.push(cur); }
    out
}

/// CLI утилита `shtty-msg` для отправки команд на IPC сокет.
/// Запускается пользователем как `shtty-msg "workspace 2"` или
/// `shtty-msg --get-workspaces`.
pub fn cli_main(args: &[String]) -> i32 {
    let cfg = crate::config::Config::load();
    let socket_path = resolve_socket_path(&cfg.ipc).unwrap_or_else(|_| {
        PathBuf::from("/tmp/superhot-tty.sock")
    });
    if args.is_empty() {
        eprintln!("Usage: shtty-msg <command>");
        eprintln!("  shtty-msg \"workspace 2\"");
        eprintln!("  shtty-msg \"exec firefox\"");
        eprintln!("  shtty-msg --get-workspaces");
        eprintln!("  shtty-msg --get-config");
        eprintln!("  shtty-msg --get-focused");
        eprintln!("  shtty-msg --get-version");
        eprintln!("Socket: {}", socket_path.display());
        return 1;
    }
    let mut stream = match UnixStream::connect(&socket_path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("cannot connect to {}: {}", socket_path.display(), e);
            return 2;
        }
    };
    let (req_type, cmd) = if args[0].starts_with("--") {
        match args[0].as_str() {
            "--get-workspaces" => ("get_workspaces", String::new()),
            "--get-config" => ("get_config", String::new()),
            "--get-focused" => ("get_focused", String::new()),
            "--get-version" => ("get_version", String::new()),
            _ => {
                eprintln!("unknown flag: {}", args[0]);
                return 1;
            }
        }
    } else {
        ("command", args.join(" "))
    };
    let request = if req_type == "command" {
        format!("{{\"type\":\"command\",\"cmd\":{}}}\n", json_escape_string(&cmd))
    } else {
        format!("{{\"type\":\"{}\"}}\n", req_type)
    };
    if let Err(e) = stream.write_all(request.as_bytes()) {
        eprintln!("write error: {}", e);
        return 3;
    }
    let _ = stream.flush();
    // Закрываем write side.
    let _ = stream.shutdown(std::net::Shutdown::Write);
    let mut response = String::new();
    if let Err(e) = stream.read_to_string(&mut response) {
        eprintln!("read error: {}", e);
        return 4;
    }
    print!("{}", response);
    if response.contains("\"status\":\"ok\"") { 0 } else { 5 }
}
