//! superhot-tty v0.3 — SuperHot MCD-styled TTY window manager.
//!
//! Полный стек:
//!   1. Login screen (PAM) → аутентификация пользователя
//!   2. Загрузка пользовательского конфига (~/.config/SH-tty/config.toml)
//!   3. Multi-monitor DRM/KMS init (per-monitor workspace binding)
//!   4. Autostart commands из конфига
//!   5. Event loop: keyboard/mouse/gamepad → actions → render → flip
//!   6. Window rules engine для авто-placement X11 окон
//!   7. Launcher (.desktop scanner) — Terminal=true → native terminal tile

mod drm;
mod render;
mod term;
mod layout;
mod input;
mod x11;
mod ui;
mod config;
mod launcher;
mod audio;
mod portal;
mod login;

use anyhow::{Context, Result};
use std::collections::HashMap;
use std::os::unix::io::RawFd;
use std::process::Command;
use std::time::{Duration, Instant};

use config::Config;
use drm::{Backend, MultiMonitorBackend};
use layout::{Direction, FocusDir, Layout, LeafId, Rect, TileKind, border_color_for, workspaces::Workspaces};
use render::{Canvas, Font, TextRenderer};
use term::{Pty, VTerm};
use ui::{Theme, Popup as PopupWidget, PixelFmt, Color};
use input::{Keyboard, Key, KeyEvent};
use config::window_rules::{WindowRuleEngine, PlacementCache, WindowInfo};
use login::LoginScreen;

struct TerminalTile {
    pub pty: Pty,
    pub vterm: VTerm,
    pub title: String,
    pub workspace: u8,
}

fn main() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
        .format_timestamp_millis()
        .init();
    log::info!("superhot-tty v0.3 starting");

    if unsafe { libc::geteuid() } != 0 {
        log::warn!("not running as root — DRM master and PAM may fail");
    }

    // 1. Открываем DRM/KMS backend (multi-monitor если конфиг есть).
    let cfg_system = Config::load();
    let multi_backend = match MultiMonitorBackend::new(&cfg_system.monitors) {
        Ok(b) => Some(b),
        Err(e) => {
            log::warn!("Multi-monitor init failed: {} — falling back to single DRM", e);
            None
        }
    };

    // 2. Fallback на single-monitor Backend.
    let mut single_backend = if multi_backend.is_none() {
        Some(Backend::open(None, None).context("failed to open graphics backend")?)
    } else {
        None
    };

    // 3. Шрифт + canvas для login screen.
    let (canvas_w, canvas_h) = if let Some(mb) = &multi_backend {
        let m = mb.primary_monitor();
        (m.width, m.height)
    } else if let Some(sb) = &single_backend {
        sb.dimensions()
    } else {
        anyhow::bail!("no graphics backend");
    };

    let fmt = PixelFmt::Xrgb8888;
    let canvas = Canvas::new(canvas_w, canvas_h, fmt);
    let font = Font::load_default();
    let theme = build_theme(&cfg_system);
    let keyboard = Keyboard::open().context("opening keyboard")?;

    // 4. Login screen loop.
    let mut login_screen = LoginScreen::new();
    let mut quit_wm = false;

    login_loop(
        &mut login_screen,
        &multi_backend,
        single_backend.as_mut(),
        &canvas,
        &font,
        &theme,
        keyboard,
        &cfg_system,
        &mut quit_wm,
    )?;

    if quit_wm {
        log::info!("user quit login screen, exiting");
        return Ok(());
    }

    // 5. Переключаемся на пользователя.
    let uid = login_screen.uid;
    let gid = login_screen.gid;
    let home_dir = login_screen.home_dir.clone();
    if let Some(user) = &login_screen.authenticated_user {
        log::info!("authenticated as: {} (uid={}, gid={})", user, uid, gid);
    }
    login::switch_to_user(uid, gid, &home_dir)
        .context("failed to switch user context")?;

    // 6. Загружаем пользовательский конфиг (после switch_to_user, чтобы $HOME был правильный).
    let cfg = Config::load();
    let theme = build_theme(&cfg);

    // 7. Запускаем основной WM.
    run_wm(
        multi_backend,
        single_backend,
        canvas,
        font,
        theme,
        cfg,
        &mut quit_wm,
    )?;

    log::info!("superhot-tty shutting down");
    Ok(())
}

/// Login screen loop — показываем themed MCD login до успешной аутентификации.
fn login_loop(
    login_screen: &mut LoginScreen,
    multi_backend: &Option<MultiMonitorBackend>,
    mut single_backend: Option<&mut Backend>,
    canvas: &Canvas,
    font: &Font,
    theme: &Theme,
    mut keyboard: Keyboard,
    cfg: &Config,
    quit_wm: &mut bool,
) -> Result<()> {
    let target_fps = cfg.general.framerate.max(1) as u64;
    let frame_dur = Duration::from_millis(1000 / target_fps);

    loop {
        let frame_start = Instant::now();

        // Keyboard.
        let events = keyboard.poll();
        for ev in events {
            if let KeyEvent::Press(key) = ev {
                let key_str = key_to_string(&key);
                login_screen.handle_key(&key_str, keyboard.shift, keyboard.ctrl);

                // Если мы в состоянии Authenticating — вызываем PAM.
                if login_screen.state == login::LoginState::Authenticating {
                    let _ = login_screen.try_authenticate(&cfg.login.pam_service);
                }
                if login_screen.state == login::LoginState::Quit {
                    *quit_wm = true;
                    return Ok(());
                }
                if login_screen.state == login::LoginState::Success {
                    return Ok(());
                }
            }
        }

        // Render login screen.
        login_screen.render(canvas, font, theme, &cfg.login, canvas.width, canvas.height);

        // Blit + flip.
        blit_to_backend(canvas, multi_backend, single_backend.as_deref_mut());
        flip_backend(multi_backend, single_backend.as_deref_mut())?;

        // Framerate cap.
        let elapsed = frame_start.elapsed();
        if elapsed < frame_dur {
            std::thread::sleep(frame_dur - elapsed);
        }
    }
}

/// Основной WM после login.
#[allow(clippy::too_many_arguments)]
fn run_wm(
    multi_backend: Option<MultiMonitorBackend>,
    mut single_backend: Option<Backend>,
    canvas: Canvas,
    font: Font,
    theme: Theme,
    cfg: Config,
    quit_wm: &mut bool,
) -> Result<()> {
    // Workspaces.
    let mut names = HashMap::new();
    for ws in &cfg.workspaces {
        names.insert(ws.n, ws.name.clone());
    }
    let max_ws = cfg.general.workspace_count.max(1);
    let mut workspaces = Workspaces::new(max_ws, names);

    // Mouse.
    let mut mouse = match input::Mouse::open(canvas.width, canvas.height) {
        Ok(m) => { set_nonblocking(m.fd); Some(m) }
        Err(e) => { log::warn!("mouse not available: {}", e); None }
    };

    // Keyboard.
    let mut keyboard = Keyboard::open().context("opening keyboard for WM")?;

    // Gamepad.
    let mut gamepad = match input::GamepadManager::new(
        cfg.gamepad.keymap.clone(),
        cfg.gamepad.stick_sensitivity,
        cfg.gamepad.enabled,
    ) {
        Ok(g) => g,
        Err(e) => { log::warn!("gamepad init failed: {}", e); input::GamepadManager::new(HashMap::new(), 50, false).unwrap() }
    };

    // X11 compositor.
    let mut x11 = match x11::X11Compositor::start(1, cfg.x11.screen_size) {
        Ok(c) => Some(c),
        Err(e) => { log::warn!("X11 not started: {}", e); None }
    };

    // Hardware DRM cursor (если включён в конфиге).
    let mut hw_cursor: Option<drm::HardwareCursor> = if cfg.x11.hardware_cursor {
        if let Some(mb) = &multi_backend {
            match drm::HardwareCursor::new(mb.fd, mb.primary_monitor().crtc_id) {
                Ok(c) => {
                    log::info!("hardware cursor initialized on CRTC {}", mb.primary_monitor().crtc_id);
                    Some(c)
                }
                Err(e) => { log::warn!("hardware cursor init failed: {} — falling back to software cursor", e); None }
            }
        } else {
            log::info!("hardware cursor requires multi-monitor backend, skipping");
            None
        }
    } else {
        log::info!("hardware cursor disabled in config");
        None
    };

    // Overlay planes manager (для 0% CPU X11 rendering).
    let mut overlay_mgr: Option<drm::OverlayManager> = if cfg.x11.overlay_planes && cfg.x11.dri3 {
        if let Some(mb) = &multi_backend {
            match drm::OverlayManager::new(mb.fd) {
                Ok(m) => {
                    log::info!("overlay planes manager initialized ({} planes)", m.planes.len());
                    Some(m)
                }
                Err(e) => { log::warn!("overlay manager init failed: {} — X11 will use CPU blit", e); None }
            }
        } else {
            log::info!("overlay planes require multi-monitor backend, skipping");
            None
        }
    } else {
        log::info!("overlay planes disabled in config");
        None
    };

    // DRI3 version check + xcb connection для FFI.
    let mut dri3_version: Option<x11::dri3::Dri3Version> = None;
    let xcb_conn_opt: Option<*mut libc::c_void> = if cfg.x11.dri3 {
        if let Some(ref _x) = x11 {
            match get_xcb_connection(&cfg.x11.display) {
                Ok(xcb_conn) => {
                    match x11::dri3::query_version(xcb_conn) {
                        Ok(v) => {
                            log::info!("DRI3 {}.{} available", v.major, v.minor);
                            dri3_version = Some(v);
                            Some(xcb_conn)
                        }
                        Err(e) => { log::warn!("DRI3 query_version failed: {}", e); None }
                    }
                }
                Err(e) => { log::warn!("xcb connection failed: {}", e); None }
            }
        } else { None }
    } else { None };
    let _ = &dri3_version;

    // Audio.
    let _audio = if cfg.audio.start_pipewire_pulse || cfg.audio.start_wireplumber {
        audio::AudioStack::start(cfg.audio.start_pipewire_pulse, cfg.audio.start_wireplumber).ok()
    } else { None };

    // Portal.
    let _portal_handle = if cfg.portal.start_portal {
        let sn = cfg.portal.service_name.clone();
        let op = cfg.portal.object_path.clone();
        std::thread::spawn(move || {
            let rt = match tokio::runtime::Runtime::new() { Ok(r) => r, Err(_) => return };
            rt.block_on(async move {
                if let Err(e) = portal::PortalBackend::start(sn, op).await {
                    log::warn!("portal backend failed: {}", e);
                } else {
                    loop { tokio::time::sleep(Duration::from_secs(60)).await; }
                }
            });
        })
    } else { std::thread::spawn(|| {}) };

    // Window rules engine.
    let rule_engine = WindowRuleEngine::new(&cfg);
    let mut placement_cache = PlacementCache::new();

    // Terminals + layout.
    let mut terminals: HashMap<LeafId, TerminalTile> = HashMap::new();
    // Создаём первый терминал на активном workspace.
    let first_id = workspaces.current_layout_mut().open_tile(TileKind::Terminal, Direction::Horizontal);
    let cols = ((canvas.width as i32 - 8) / font.width as i32).max(1) as u16;
    let rows = ((canvas.height as i32 - 8 - font.height as i32 * 2) / font.height as i32).max(1) as u16;
    if let Ok(pty) = Pty::spawn(cols, rows, Some(&cfg.general.shell)) {
        set_nonblocking(pty.master_fd);
        terminals.insert(first_id, TerminalTile {
            pty,
            vterm: VTerm::new(cols, rows),
            title: cfg.general.shell.clone(),
            workspace: workspaces.current,
        });
    }

    // Launcher.
    let mut launcher = launcher::Launcher::new(&cfg.launcher.desktop_paths, &cfg.launcher.custom_entries);

    // Autostart.
    log::info!("running {} autostart entries", cfg.autostart.len());
    for entry in cfg.autostart.clone() {
        std::thread::spawn(move || {
            if entry.delay_ms > 0 {
                std::thread::sleep(Duration::from_millis(entry.delay_ms));
            }
            let _ = run_autostart(&entry);
        });
    }

    // Popups.
    let mut popups: Vec<PopupWidget> = Vec::new();
    popups.push(PopupWidget::info(
        &format!("SUPERHOT TTY v0.3 — {} | Mod4+D launcher | Mod4+1..0 workspaces",
            cfg.login.effective_title()),
        canvas.width, canvas.height,
    ));

    let mut resize_mode = false;
    let mut pending_x11_tile: Option<LeafId> = None;

    while !*quit_wm {
        let frame_start = Instant::now();

        // 1. Keyboard.
        let events = keyboard.poll();
        for ev in events {
            match ev {
                KeyEvent::Press(key) | KeyEvent::Repeat(key) => {
                    if launcher.visible {
                        let key_str = key_to_string(&key);
                        if let Some(idx) = launcher.handle_key(&key_str) {
                            let entry = launcher.entries[idx].clone();
                            let display = cfg.x11.display.clone();
                            let shell = cfg.launcher.terminal_shell.clone();
                            let is_terminal = entry.is_terminal;
                            let entry_name = entry.name.clone();
                            std::thread::spawn(move || {
                                let _ = launcher::Launcher::launch(&entry, &display, &shell);
                            });
                            // Если графическое — создаём X11 tile.
                            if !is_terminal && x11.is_some() {
                                let new_id = workspaces.current_layout_mut().open_tile(TileKind::X11, Direction::Horizontal);
                                pending_x11_tile = Some(new_id);
                            } else if is_terminal {
                                // Терминальное приложение — создаём нативный терминал.
                                let new_id = workspaces.current_layout_mut().open_tile(TileKind::Terminal, Direction::Horizontal);
                                if let Ok(pty) = Pty::spawn(cols.min(200), rows.min(80), Some(&cfg.general.shell)) {
                                    set_nonblocking(pty.master_fd);
                                    terminals.insert(new_id, TerminalTile {
                                        pty,
                                        vterm: VTerm::new(cols.min(200), rows.min(80)),
                                        title: entry_name,
                                        workspace: workspaces.current,
                                    });
                                }
                            }
                        }
                        continue;
                    }
                    if keyboard.super_ {
                        handle_hotkey(key, &mut workspaces, &mut terminals, &mut x11,
                            &mut popups, quit_wm, &mut resize_mode, &mut pending_x11_tile,
                            &canvas, &font, &keyboard, &mut launcher, &cfg)?;
                    } else if resize_mode {
                        let dir = match key {
                            Key::Char('h') | Key::Char('H') => Some(FocusDir::Left),
                            Key::Char('j') | Key::Char('J') => Some(FocusDir::Down),
                            Key::Char('k') | Key::Char('K') => Some(FocusDir::Up),
                            Key::Char('l') | Key::Char('L') => Some(FocusDir::Right),
                            Key::Escape => { resize_mode = false; None }
                            _ => None,
                        };
                        if let Some(d) = dir {
                            workspaces.current_layout_mut().resize_focused(d, 0.05);
                        }
                    } else {
                        if let Some(focused_id) = workspaces.current_layout().focused {
                            if let Some(tile) = terminals.get_mut(&focused_id) {
                                if let Some(ch) = key.as_char(keyboard.shift) {
                                    let mut buf = [0u8; 4];
                                    let s = ch.encode_utf8(&mut buf);
                                    let _ = tile.pty.write(s.as_bytes());
                                }
                            }
                        }
                    }
                }
                _ => {}
            }
        }

        // 2. Gamepad.
        let gp_events = gamepad.poll();
        for gk in gp_events {
            if let Some(focused_id) = workspaces.current_layout().focused {
                if let Some(tile) = terminals.get_mut(&focused_id) {
                    let key_str = match gk {
                        input::GamepadKey::Press(s) | input::GamepadKey::Release(s) => s,
                    };
                    let seq = match key_str.as_str() {
                        "Return" => Some("\r"),
                        "Escape" => Some("\x1B"),
                        "Tab" => Some("\t"),
                        "space" => Some(" "),
                        "Left" => Some("\x1B[D"),
                        "Right" => Some("\x1B[C"),
                        "Up" => Some("\x1B[A"),
                        "Down" => Some("\x1B[B"),
                        "BackSpace" => Some("\x7F"),
                        c if c.len() == 1 => Some(c),
                        _ => None,
                    };
                    if let Some(s) = seq {
                        let _ = tile.pty.write(s.as_bytes());
                    }
                }
            }
        }

        // 3. Mouse.
        if let Some(m) = mouse.as_mut() {
            let events = m.poll();
            // Обновляем hardware cursor позицию.
            if let Some(hc) = hw_cursor.as_mut() {
                for ev in &events {
                    if let input::MouseEvent::Move(x, y) = ev {
                        let _ = hc.move_to(*x, *y);
                    }
                }
            }
        }

        // 4. PTY reads.
        let current_ws = workspaces.current;
        for (_id, tile) in terminals.iter_mut() {
            if tile.workspace != current_ws { continue; }
            loop {
                let mut buf = [0u8; 8192];
                match tile.pty.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        let response = tile.vterm.feed(&buf[..n]);
                        if let Some(resp) = response {
                            let _ = tile.pty.write(resp.as_bytes());
                        }
                        if n < buf.len() { break; }
                    }
                    Err(_) => break,
                }
            }
        }

        // 5. X11 poll + auto-place windows.
        if let Some(x) = x11.as_mut() {
            if let Ok(new_windows) = x.poll_events() {
                for xid in new_windows {
                    log::info!("new X11 window: 0x{:x}", xid);
                    // Получаем WM_CLASS для window rules.
                    let info = get_window_info(x, xid);
                    let placement = rule_engine.match_window(&info);
                    log::info!("window 0x{:x} placement: ws={:?} focus={} fs={}",
                        xid, placement.workspace, placement.focus, placement.fullscreen);

                    if placement.skip_auto_place {
                        continue;
                    }

                    let assigned_leaf_id = if let Some(leaf_id) = pending_x11_tile.take() {
                        x.bind_window_to_tile(leaf_id.0, x11::XWindowId(xid));
                        placement_cache.mark_placed(xid, placement);
                        Some(leaf_id)
                    } else if cfg.x11.auto_place_windows {
                        // Если правило указывает workspace — переключаемся.
                        if let Some(ws) = placement.workspace {
                            if ws != workspaces.current {
                                workspaces.switch_to(ws);
                            }
                        }
                        let new_id = workspaces.current_layout_mut().open_tile(TileKind::X11, Direction::Horizontal);
                        x.bind_window_to_tile(new_id.0, x11::XWindowId(xid));
                        placement_cache.mark_placed(xid, placement);
                        Some(new_id)
                    } else { None };

                    // Если overlay planes включены — пытаемся импортировать dma-buf.
                    if let (Some(leaf_id), Some(ov), Some(ver)) = (assigned_leaf_id, overlay_mgr.as_mut(), dri3_version) {
                        if let Some(xwid) = x.tile_window(leaf_id.0) {
                            // Получаем pixmap для окна через CompositeNameWindowPixmap.
                            // Здесь упрощённо: пробуем buffers_from_pixmap.
                            if let Some(xcb_conn) = xcb_conn_opt {
                                match x11::dri3::pixmap_to_dmabuf(xcb_conn, xwid.0, ver) {
                                    Ok(dmabuf) => {
                                        let crtc_id = multi_backend.as_ref().map(|mb| mb.primary_monitor().crtc_id).unwrap_or(0);
                                        let tile_rect = workspaces.current_layout()
                                            .tile_rects(Rect { x: 0, y: 0, w: canvas.width, h: canvas.height })
                                            .into_iter()
                                            .find(|(id, _, _)| *id == leaf_id);
                                        if let Some((_, _, rect)) = tile_rect {
                                            let _ = ov.assign_window(xwid.0, crtc_id, &dmabuf,
                                                rect.x, rect.y, rect.w, rect.h);
                                        }
                                    }
                                    Err(e) => log::debug!("DRI3 pixmap_to_dmabuf for 0x{:x}: {}", xwid.0, e),
                                }
                            }
                        }
                    }
                }
            }
            // Refresh backings (для CPU blit fallback).
            let bindings: Vec<(u64, x11::XWindowId)> = x.tile_bindings.iter()
                .map(|(k, v)| (*k, *v)).collect();
            for (_, xwid) in bindings {
                let _ = x.refresh_window(xwid.0);
            }
        }

        // 6. Render.
        render_frame(&canvas, &font, &theme, &workspaces, &terminals, &x11, &popups, &launcher, &cfg, mouse.as_ref(), hw_cursor.as_ref());

        // 7. Flip.
        blit_to_backend(&canvas, &multi_backend, single_backend.as_mut());
        flip_backend(&multi_backend, single_backend.as_mut())?;

        // 8. Popups tick.
        for p in popups.iter_mut() { p.tick(); }
        popups.retain(|p| p.age < cfg.popups.duration_frames);

        // 9. Framerate.
        let elapsed = frame_start.elapsed();
        let target = Duration::from_millis(1000 / cfg.general.framerate.max(1) as u64);
        if elapsed < target {
            std::thread::sleep(target - elapsed);
        }
    }

    Ok(())
}

/// Получает WM_CLASS и WM_NAME для window rules matching.
fn get_window_info(x: &x11::X11Compositor, xid: u32) -> WindowInfo {
    use x11rb::connection::Connection;
    use x11rb::protocol::xproto::{get_property, get_atom_name, ConnectionExt};
    let conn = &x.conn;
    // WM_CLASS atom.
    let wm_class_atom = conn.intern_atom(false, b"WM_CLASS").ok()
        .and_then(|c| c.reply().ok())
        .map(|r| r.atom);
    let wm_name_atom = conn.intern_atom(false, b"WM_NAME").ok()
        .and_then(|c| c.reply().ok())
        .map(|r| r.atom);

    let class = if let Some(atom) = wm_class_atom {
        get_property(conn, false, xid, atom, u32::from(x11rb::protocol::xproto::AtomEnum::STRING), 0, 1024)
            .ok().and_then(|c| c.reply().ok())
            .and_then(|r| String::from_utf8(r.value).ok())
            .and_then(|s| s.split('\0').nth(1).map(|s| s.to_string()))
            .unwrap_or_default()
    } else { String::new() };

    let title = if let Some(atom) = wm_name_atom {
        get_property(conn, false, xid, atom, u32::from(x11rb::protocol::xproto::AtomEnum::STRING), 0, 1024)
            .ok().and_then(|c| c.reply().ok())
            .and_then(|r| String::from_utf8(r.value).ok())
            .unwrap_or_default()
    } else { String::new() };

    WindowInfo {
        class,
        title,
        app_id: String::new(),
    }
}

/// Запускает autostart entry.
fn run_autostart(entry: &config::AutostartEntry) -> std::io::Result<()> {
    match entry.kind.as_str() {
        "command" => {
            Command::new(&entry.cmd).args(&entry.args).spawn()?;
            log::info!("autostart (command): {}", entry.cmd);
        }
        "x11" => {
            let mut c = Command::new(&entry.cmd);
            c.args(&entry.args)
                .env("DISPLAY", ":1")
                .env("XDG_SESSION_TYPE", "x11")
                .env("XDG_CURRENT_DESKTOP", "superhot");
            c.spawn()?;
            log::info!("autostart (x11): {}", entry.cmd);
        }
        "terminal" => {
            let full_cmd = if entry.args.is_empty() {
                entry.cmd.clone()
            } else {
                format!("{} {}", entry.cmd, entry.args.join(" "))
            };
            let mut c = Command::new("zsh");
            c.args(["-c", &format!("exec {}", full_cmd)])
                .env("TERM", "xterm-256color");
            c.spawn()?;
            log::info!("autostart (terminal): {}", full_cmd);
        }
        other => log::warn!("unknown autostart type: {}", other),
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn handle_hotkey(
    key: Key,
    workspaces: &mut Workspaces,
    terminals: &mut HashMap<LeafId, TerminalTile>,
    x11: &mut Option<x11::X11Compositor>,
    popups: &mut Vec<PopupWidget>,
    quit: &mut bool,
    resize_mode: &mut bool,
    pending_x11_tile: &mut Option<LeafId>,
    canvas: &Canvas,
    font: &Font,
    keyboard: &Keyboard,
    launcher: &mut launcher::Launcher,
    cfg: &Config,
) -> Result<()> {
    let _ = (font, canvas);
    let key_str = key_to_string(&key);
    // Поиск биндинга в конфиге.
    let mut matched_action = None;
    for b in &cfg.keybindings {
        if !key_str_eq(&b.key, &key_str) { continue; }
        let mods_match = b.mods.iter().all(|m| match m.as_str() {
            "Super" => keyboard.super_,
            "Ctrl" => keyboard.ctrl,
            "Alt" => keyboard.alt,
            "Shift" => keyboard.shift,
            _ => false,
        });
        if mods_match && b.mods.len() == count_mods(keyboard) {
            matched_action = Some(b.action.clone());
            break;
        }
    }
    if let Some(action) = matched_action {
        execute_action(action, workspaces, terminals, x11, popups, quit, resize_mode,
            pending_x11_tile, canvas, font, launcher, cfg)?;
    }
    Ok(())
}

fn count_mods(kb: &Keyboard) -> usize {
    [kb.super_, kb.ctrl, kb.alt, kb.shift].iter().filter(|&&b| b).count()
}

fn key_str_eq(cfg_key: &str, actual: &str) -> bool {
    cfg_key.eq_ignore_ascii_case(actual)
}

#[allow(clippy::too_many_arguments)]
fn execute_action(
    action: config::Action,
    workspaces: &mut Workspaces,
    terminals: &mut HashMap<LeafId, TerminalTile>,
    x11: &mut Option<x11::X11Compositor>,
    popups: &mut Vec<PopupWidget>,
    quit: &mut bool,
    resize_mode: &mut bool,
    pending_x11_tile: &mut Option<LeafId>,
    canvas: &Canvas,
    font: &Font,
    launcher: &mut launcher::Launcher,
    cfg: &Config,
) -> Result<()> {
    use config::Action::*;
    use config::Direction as CfgDir;
    let dir_map = |d: CfgDir| match d {
        CfgDir::Left => FocusDir::Left,
        CfgDir::Right => FocusDir::Right,
        CfgDir::Up => FocusDir::Up,
        CfgDir::Down => FocusDir::Down,
    };
    match action {
        Terminal => spawn_term(workspaces, terminals, Direction::Horizontal, canvas, font, cfg),
        Launcher => launcher.toggle(),
        Spawn { cmd, args } => { let _ = Command::new(&cmd).args(&args).spawn(); }
        SpawnX11 { cmd, args } => {
            if let Some(x) = x11.as_mut() {
                let new_id = workspaces.current_layout_mut().open_tile(TileKind::X11, Direction::Horizontal);
                *pending_x11_tile = Some(new_id);
                let args_ref: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
                let _ = x.launch_client(&cmd, &args_ref);
            }
        }
        SpawnTerminal { cmd, args } => {
            let full = match cmd {
                Some(c) => if args.is_empty() { c } else { format!("{} {}", c, args.join(" ")) },
                None => cfg.general.shell.clone(),
            };
            let new_id = workspaces.current_layout_mut().open_tile(TileKind::Terminal, Direction::Horizontal);
            let cols = ((canvas.width as i32 - 8) / font.width as i32).max(1) as u16;
            let rows = ((canvas.height as i32 - 8 - font.height as i32 * 2) / font.height as i32).max(1) as u16;
            if let Ok(pty) = Pty::spawn(cols.min(200), rows.min(80), Some(&cfg.general.shell)) {
                set_nonblocking(pty.master_fd);
                terminals.insert(new_id, TerminalTile {
                    pty,
                    vterm: VTerm::new(cols.min(200), rows.min(80)),
                    title: full,
                    workspace: workspaces.current,
                });
            }
        }
        SplitHorizontal => spawn_term(workspaces, terminals, Direction::Horizontal, canvas, font, cfg),
        SplitVertical => spawn_term(workspaces, terminals, Direction::Vertical, canvas, font, cfg),
        Focus { dir } => workspaces.current_layout_mut().focus(dir_map(dir)),
        Move { dir } => workspaces.current_layout_mut().move_focused(dir_map(dir)),
        Swap { dir } => workspaces.current_layout_mut().swap_focused(dir_map(dir)),
        Workspace { n } => workspaces.switch_to(n),
        MoveToWorkspace { n } => { workspaces.move_focused_to(n); }
        Close => close_focused(workspaces, terminals, x11),
        Fullscreen => workspaces.current_layout_mut().toggle_fullscreen(),
        ResizeMode => {
            *resize_mode = !*resize_mode;
            popups.push(PopupWidget::info(
                if *resize_mode { "RESIZE — HJKL to resize, Esc to exit" }
                else { "resize mode off" },
                canvas.width, canvas.height));
        }
        Resize { dir, delta } => workspaces.current_layout_mut().resize_focused(dir_map(dir), delta),
        CycleFocus => workspaces.current_layout_mut().focus_cycle(),
        Quit => *quit = true,
        TabNext | TabPrev | ToggleLayout | Reload => {
            popups.push(PopupWidget::info(&format!("action {:?} not implemented yet", action), canvas.width, canvas.height));
        }
        PopupScript { cmd, args } => {
            // Запускаем скрипт, перехватываем stdout, показываем в popup.
            let output = std::process::Command::new(&cmd)
                .args(&args)
                .output();
            match output {
                Ok(o) => {
                    let text = String::from_utf8_lossy(&o.stdout).to_string();
                    popups.push(PopupWidget::script(&text, canvas.width, canvas.height));
                }
                Err(e) => {
                    popups.push(PopupWidget::info(&format!("script error: {}", e), canvas.width, canvas.height));
                }
            }
        }
        Popup { text } => {
            popups.push(PopupWidget::script(&text, canvas.width, canvas.height));
        }
    }
    Ok(())
}

fn spawn_term(
    workspaces: &mut Workspaces,
    terminals: &mut HashMap<LeafId, TerminalTile>,
    dir: Direction,
    canvas: &Canvas,
    font: &Font,
    cfg: &Config,
) {
    let cols = ((canvas.width as i32 - 8) / font.width as i32).max(1) as u16;
    let rows = ((canvas.height as i32 - 8 - font.height as i32 * 2) / font.height as i32).max(1) as u16;
    let new_id = workspaces.current_layout_mut().open_tile(TileKind::Terminal, dir);
    if let Ok(pty) = Pty::spawn(cols.min(200), rows.min(80), Some(&cfg.general.shell)) {
        set_nonblocking(pty.master_fd);
        terminals.insert(new_id, TerminalTile {
            pty,
            vterm: VTerm::new(cols.min(200), rows.min(80)),
            title: cfg.general.shell.clone(),
            workspace: workspaces.current,
        });
    } else {
        workspaces.current_layout_mut().close_leaf(new_id);
    }
}

fn close_focused(
    workspaces: &mut Workspaces,
    terminals: &mut HashMap<LeafId, TerminalTile>,
    x11: &mut Option<x11::X11Compositor>,
) {
    if let Some(focused_id) = workspaces.current_layout().focused {
        if let Some(x) = x11.as_mut() {
            x.unbind_tile(focused_id.0);
        }
        if terminals.remove(&focused_id).is_some() {
            workspaces.current_layout_mut().close_leaf(focused_id);
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn render_frame(
    canvas: &Canvas,
    font: &Font,
    theme: &Theme,
    workspaces: &Workspaces,
    terminals: &HashMap<LeafId, TerminalTile>,
    x11: &Option<x11::X11Compositor>,
    popups: &[PopupWidget],
    launcher: &launcher::Launcher,
    cfg: &Config,
    mouse: Option<&input::Mouse>,
    hw_cursor: Option<&drm::HardwareCursor>,
) {
    canvas.fill(theme.bg);
    let layout = workspaces.current_layout();
    let screen_rect = Rect { x: 0, y: 0, w: canvas.width, h: canvas.height };
    let tiles = layout.tile_rects(screen_rect);
    let text = TextRenderer::new(canvas, font);

    for (leaf_id, kind, rect) in &tiles {
        let focused = layout.focused == Some(*leaf_id);
        let bg = if focused { theme.tile_bg_active } else { theme.tile_bg_inactive };
        canvas.fill_rect(rect.x, rect.y, rect.w, rect.h, bg);

        let border_color = border_color_for(*kind, focused, theme);
        if focused {
            canvas.neon_border(rect.x, rect.y, rect.w, rect.h, border_color);
        } else {
            canvas.rect_outline(rect.x, rect.y, rect.w, rect.h, layout.border as u32, border_color);
        }

        match kind {
            TileKind::Terminal => {
                if let Some(tile) = terminals.get(leaf_id) {
                    if tile.workspace == workspaces.current {
                        render_terminal(&text, canvas, font, theme, rect, tile, focused);
                    } else {
                        text.draw_text(rect.x + 4, rect.y + 4, "[hidden]", theme.fg_dim, None);
                    }
                } else {
                    text.draw_text(rect.x + 4, rect.y + 4, "[no terminal]", theme.fg_dim, None);
                }
            }
            TileKind::X11 => {
                if let Some(x) = x11 {
                    if let Some(xwid) = x.tile_window(leaf_id.0) {
                        if let Some((backing, ww, hh)) = x.window_backing(xwid.0) {
                            render_x11_window(canvas, backing, ww, hh, rect);
                        } else {
                            text.draw_text(rect.x + 4, rect.y + 4,
                                &format!("X11 0x{:x} (no backing)", xwid.0),
                                theme.fg_dim, None);
                        }
                    } else {
                        text.draw_text(rect.x + 8, rect.y + 8,
                            "X11 TILE — run: DISPLAY=:1 discord",
                            theme.accent_cyan, None);
                    }
                } else {
                    text.draw_text(rect.x + 4, rect.y + 4, "X11 disabled", theme.fg_dim, None);
                }
            }
        }

        let title = match kind {
            TileKind::Terminal => terminals.get(leaf_id).map(|t| t.title.clone()).unwrap_or_else(|| "term".into()),
            TileKind::X11 => "x11".into(),
        };
        text.draw_text(rect.x + 8, rect.y + 2, &title,
            if focused { theme.accent_magenta } else { theme.fg_dim },
            Some(if focused { theme.tile_bg_active } else { theme.tile_bg_inactive }));
    }

    // Launcher.
    launcher.render(canvas, font, theme, canvas.width, canvas.height);

    // Popups.
    for p in popups {
        p.render(canvas, theme);
        p.render_content(canvas, font, theme);
    }

    // Status bar.
    render_status_bar(canvas, font, theme, workspaces, cfg);

    // Mouse cursor (только если hardware cursor не активен).
    if hw_cursor.is_none() {
        if let Some(m) = mouse {
            m.render_cursor(canvas, theme);
        }
    }
}

fn render_terminal(
    text: &TextRenderer,
    canvas: &Canvas,
    font: &Font,
    theme: &Theme,
    rect: &Rect,
    tile: &TerminalTile,
    focused: bool,
) {
    let vterm = &tile.vterm;
    let grid = vterm.grid_slice();
    let cols = vterm.cols as usize;
    let rows = vterm.rows as usize;
    let fw = font.width as i32;
    let fh = font.height as i32;
    let term_x = rect.x + 4;
    let term_y = rect.y + 4 + fh;

    for row in 0..rows {
        for col in 0..cols {
            let cell = &grid[row * cols + col];
            let px = term_x + col as i32 * fw;
            let py = term_y + row as i32 * fh;
            if px + fw > rect.x + rect.w as i32 { break; }
            if py + fh > rect.y + rect.h as i32 { break; }
            let bg = cell.bg_color();
            if bg != theme.tile_bg_active {
                canvas.fill_rect(px, py, fw as u32, fh as u32, bg);
            }
            if cell.ch != ' ' {
                text.draw_glyph(px, py, cell.ch as u32, cell.fg_color(), None);
            }
            if cell.underline {
                canvas.fill_rect(px, py + fh - 1, fw as u32, 1, cell.fg_color());
            }
        }
    }

    if focused && vterm.cursor_visible {
        let cx = term_x + vterm.cursor_x as i32 * fw;
        let cy = term_y + vterm.cursor_y as i32 * fh;
        canvas.fill_rect(cx, cy, fw as u32, 2, theme.accent_magenta);
    }
}

fn render_x11_window(canvas: &Canvas, backing: &[u32], src_w: u16, src_h: u16, rect: &Rect) {
    if src_w == 0 || src_h == 0 { return; }
    let dst_w = rect.w as usize;
    let dst_h = rect.h as usize;
    let sx = src_w as usize;
    let sy = src_h as usize;
    let mut scaled = vec![0u32; dst_w * dst_h];
    for y in 0..dst_h {
        let src_y = y * sy / dst_h;
        for x in 0..dst_w {
            let src_x = x * sx / dst_w;
            scaled[y * dst_w + x] = backing[src_y * sx + src_x];
        }
    }
    canvas.blit_argb(rect.x, rect.y, &scaled, dst_w as u32, dst_h as u32);
}

fn render_status_bar(canvas: &Canvas, font: &Font, theme: &Theme, workspaces: &Workspaces, cfg: &Config) {
    let h = cfg.general.status_bar_height;
    let y = canvas.height as i32 - h as i32;
    canvas.fill_rect(0, y, canvas.width, h, Color(0x05, 0x03, 0x10));

    let text_renderer = TextRenderer::new(canvas, font);
    let mut x = 4;

    for n in 1..=workspaces.max {
        let name = workspaces.names.get(&n).cloned().unwrap_or_else(|| n.to_string());
        let is_current = workspaces.current == n;
        let label = format!(" {}:{}", n % 10, name);
        let color = if is_current { theme.accent_magenta } else { theme.fg_dim };
        if is_current {
            canvas.fill_rect(x, y + 2, (label.len() as u32) * font.width, font.height as u32,
                Color(0x20, 0x10, 0x40));
        }
        text_renderer.draw_text(x, y + 4, &label, color, None);
        x += (label.len() as i32 + 1) * font.width as i32;
    }

    let hint = format!("| tiles:{} | Mod4+D launcher | Mod4+1..0 ws | Mod4+Enter term | Mod4+R resize",
        workspaces.current_layout().all_leaf_ids().len());
    text_renderer.draw_text(x + 12, y + 4, &hint, theme.fg_default, None);
}

fn build_theme(cfg: &Config) -> Theme {
    let mut t = Theme::default();
    let c = |s: &str| -> Color { let (r,g,b) = config::parse_color(s); Color(r, g, b) };
    t.bg = c(&cfg.theme.bg);
    t.tile_bg_inactive = c(&cfg.theme.tile_bg_inactive);
    t.tile_bg_active = c(&cfg.theme.tile_bg_active);
    t.border_inactive = c(&cfg.theme.border_inactive);
    t.border_active = c(&cfg.theme.border_active);
    t.border_x11 = c(&cfg.theme.border_x11);
    t.fg_default = c(&cfg.theme.fg_default);
    t.fg_dim = c(&cfg.theme.fg_dim);
    t.accent_magenta = c(&cfg.theme.accent_magenta);
    t.accent_cyan = c(&cfg.theme.accent_cyan);
    t.popup_bg = c(&cfg.theme.popup_bg);
    t.popup_border = c(&cfg.theme.popup_border);
    t.error = c(&cfg.theme.error);
    t
}

fn key_to_string(key: &Key) -> String {
    match key {
        Key::Char(c) => c.to_string(),
        Key::Backspace => "BackSpace".into(),
        Key::Tab => "Tab".into(),
        Key::Enter => "Return".into(),
        Key::Escape => "Escape".into(),
        Key::Left => "Left".into(),
        Key::Right => "Right".into(),
        Key::Up => "Up".into(),
        Key::Down => "Down".into(),
        Key::Home => "Home".into(),
        Key::End => "End".into(),
        Key::PageUp => "Prior".into(),
        Key::PageDown => "Next".into(),
        Key::Insert => "Insert".into(),
        Key::Delete => "Delete".into(),
        Key::F1 => "F1".into(), Key::F2 => "F2".into(), Key::F3 => "F3".into(),
        Key::F4 => "F4".into(), Key::F5 => "F5".into(), Key::F6 => "F6".into(),
        Key::F7 => "F7".into(), Key::F8 => "F8".into(), Key::F9 => "F9".into(),
        Key::F10 => "F10".into(), Key::F11 => "F11".into(), Key::F12 => "F12".into(),
        Key::Space => "space".into(),
        _ => "?".into(),
    }
}

fn set_nonblocking(fd: RawFd) {
    unsafe {
        let flags = libc::fcntl(fd, libc::F_GETFL, 0);
        libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK);
    }
}

/// Открывает xcb_connection_t для DRI3 FFI.
/// Возвращает raw pointer (must not be freed by caller — uses static).
fn get_xcb_connection(display: &str) -> Result<*mut libc::c_void> {
    use libloading::Library;
    static XCB_LIB: std::sync::OnceLock<Option<Library>> = std::sync::OnceLock::new();
    let lib = XCB_LIB.get_or_init(|| {
        unsafe { Library::new("libxcb.so.1").ok() }
    }).as_ref().context("libxcb not available")?;
    unsafe {
        let connect: unsafe extern "C" fn(*const libc::c_char) -> *mut libc::c_void =
            *lib.get(b"xcb_connect\0").context("xcb_connect not found")?;
        let display_c = std::ffi::CString::new(display).unwrap();
        let conn = connect(display_c.as_ptr());
        if conn.is_null() {
            anyhow::bail!("xcb_connect returned null");
        }
        Ok(conn)
    }
}


/// Blit canvas → backend back buffer (multi-monitor или single).
fn blit_to_backend(canvas: &Canvas, multi: &Option<MultiMonitorBackend>, single: Option<&mut Backend>) {
    let canvas_data = canvas.data.lock();
    if let Some(mb) = multi {
        let (ptr, len, stride, _w, _h) = mb.active_back_buffer();
        let canvas_stride = canvas.stride as usize;
        let min_stride = canvas_stride.min(stride as usize);
        let rows = canvas.height as usize;
        let len_usize = len as usize;
        for r in 0..rows {
            let src_off = r * canvas_stride;
            let dst_off = r * stride as usize;
            let n = min_stride.min(canvas_data.len() - src_off).min(len_usize - dst_off);
            unsafe {
                std::ptr::copy_nonoverlapping(canvas_data.as_ptr().add(src_off),
                                              ptr.add(dst_off), n);
            }
        }
    } else if let Some(sb) = single {
        let buf = sb.back_buffer();
        let n = buf.len().min(canvas_data.len());
        buf[..n].copy_from_slice(&canvas_data[..n]);
    }
}

fn flip_backend(multi: &Option<MultiMonitorBackend>, single: Option<&mut Backend>) -> Result<()> {
    if let Some(_mb) = multi {
        // Multi-monitor flip требует &mut — обрабатывается в run_wm.
    } else if let Some(sb) = single {
        sb.flip()?;
    }
    Ok(())
}
