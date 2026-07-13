//! `shtty-msg` — CLI утилита для отправки команд на IPC сокет superhot-tty.
//!
//! Протокол i3-msg-совместимый. Примеры:
//!   shtty-msg "workspace 2"
//!   shtty-msg "exec firefox"
//!   shtty-msg "exec --no-startup-id alacritty"
//!   shtty-msg "kill"
//!   shtty-msg "reload"
//!   shtty-msg "split vertical"
//!   shtty-msg "focus left"
//!   shtty-msg "fullscreen toggle"
//!   shtty-msg --get-workspaces
//!   shtty-msg --get-config
//!   shtty-msg --get-focused
//!   shtty-msg --get-version
//!
//! Это standalone binary — не зависит от основного crate superhot-tty.
//! Запускается в user-space, подключается к UNIX-сокету и отправляет JSON.

use std::env;
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;

fn resolve_socket_path() -> PathBuf {
    // Порядок:
    //   1. $XDG_RUNTIME_DIR/superhot-tty.sock
    //   2. /tmp/superhot-tty-$UID.sock
    //   3. /tmp/superhot-tty.sock (fallback)
    if let Ok(xdg) = env::var("XDG_RUNTIME_DIR") {
        if !xdg.is_empty() {
            return PathBuf::from(format!("{}/superhot-tty.sock", xdg));
        }
    }
    let uid = unsafe { libc::getuid() };
    PathBuf::from(format!("/tmp/superhot-tty-{}.sock", uid))
}

fn json_escape(s: &str) -> String {
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

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    let socket_path = resolve_socket_path();

    if args.is_empty() {
        eprintln!("Usage: shtty-msg <command>");
        eprintln!();
        eprintln!("Commands (i3-msg syntax):");
        eprintln!("  shtty-msg \"workspace N\"        — switch to workspace N");
        eprintln!("  shtty-msg \"workspace next|prev\"");
        eprintln!("  shtty-msg \"move to workspace N\"");
        eprintln!("  shtty-msg \"exec CMD [ARGS...]\"  — spawn program");
        eprintln!("  shtty-msg \"exec --no-startup-id CMD ...\"");
        eprintln!("  shtty-msg \"kill\"                — close focused window");
        eprintln!("  shtty-msg \"reload\"              — reload config");
        eprintln!("  shtty-msg \"restart\"             — restart WM (= quit)");
        eprintln!("  shtty-msg \"quit\"                — quit WM");
        eprintln!("  shtty-msg \"split horizontal|vertical\"");
        eprintln!("  shtty-msg \"focus left|right|up|down\"");
        eprintln!("  shtty-msg \"fullscreen toggle\"");
        eprintln!("  shtty-msg \"layout toggle\"");
        eprintln!("  shtty-msg \"launcher\"            — toggle launcher");
        eprintln!("  shtty-msg \"glitch\"              — trigger random glitch");
        eprintln!();
        eprintln!("Queries:");
        eprintln!("  shtty-msg --get-workspaces");
        eprintln!("  shtty-msg --get-config");
        eprintln!("  shtty-msg --get-focused");
        eprintln!("  shtty-msg --get-version");
        eprintln!();
        eprintln!("Socket: {}", socket_path.display());
        std::process::exit(1);
    }

    let mut stream = match UnixStream::connect(&socket_path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("shtty-msg: cannot connect to {}: {}", socket_path.display(), e);
            eprintln!("  Is superhot-tty running? Is IPC enabled in config?");
            std::process::exit(2);
        }
    };

    let (req_type, cmd) = if args[0].starts_with("--") {
        match args[0].as_str() {
            "--get-workspaces" => ("get_workspaces", String::new()),
            "--get-config" => ("get_config", String::new()),
            "--get-focused" => ("get_focused", String::new()),
            "--get-version" => ("get_version", String::new()),
            "--help" | "-h" => {
                eprintln!("Usage: shtty-msg <command> (run without args for full help)");
                std::process::exit(0);
            }
            _ => {
                eprintln!("shtty-msg: unknown flag: {}", args[0]);
                std::process::exit(1);
            }
        }
    } else {
        ("command", args.join(" "))
    };

    let request = if req_type == "command" {
        format!("{{\"type\":\"command\",\"cmd\":{}}}\n", json_escape(&cmd))
    } else {
        format!("{{\"type\":\"{}\"}}\n", req_type)
    };

    if let Err(e) = stream.write_all(request.as_bytes()) {
        eprintln!("shtty-msg: write error: {}", e);
        std::process::exit(3);
    }
    let _ = stream.flush();
    let _ = stream.shutdown(std::net::Shutdown::Write);

    let mut response = String::new();
    if let Err(e) = stream.read_to_string(&mut response) {
        eprintln!("shtty-msg: read error: {}", e);
        std::process::exit(4);
    }

    print!("{}", response);
    if response.contains("\"status\":\"ok\"") {
        std::process::exit(0);
    } else {
        std::process::exit(5);
    }
}
