//! superhot-tty — SuperHot MCD-styled TTY window manager.
//!
//! v0.2: конфиг TOML, workspaces, launcher, mouse+XTest, gamepad, PipeWire,
//! xdg-desktop-portal screen share, DMA-BUF/DRI3 GPU-ускорение X11.
//!
//! Запускается как замена agetty. Открывает DRM/KMS backend (fallback на fbdev),
//! создаёт первый терминальный тайл с shell, и обрабатывает ввод с клавиатуры
//! через evdev. Mod4 = Super — модификатор для всех хоткеев.

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

use anyhow::{Context, Result};
use drm::Backend;
use layout::{Direction, FocusDir, Layout, LeafId, Rect, TileKind, border_color_for, workspaces::Workspaces};
use render::{Canvas, Font, TextRenderer};
use std::collections::HashMap;
use std::os::unix::io::RawFd;
use std::time::{Duration, Instant};
use term::{Pty, VTerm};
use ui::{Theme, Popup, PixelFmt, Color};
use input::{Keyboard, Key, KeyEvent};
use config::Config;

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
    log::info!("superhot-tty v0.2 starting");

    let cfg = Config::load();

    if unsafe { libc::geteuid() } != 0 {
        log::warn!("not running as root — DRM master may fail");
    }

    let mut backend = Backend::open(None, None).context("failed to open graphics backend")?;
    let (w, h) = backend.dimensions();
    let bpp = backend.bpp();
    let fmt = if bpp == 16 { PixelFmt::Rgb565 } else { PixelFmt::Xrgb8888 };
    log::info!("backend: {}x{} {}bpp", w, h, bpp);

    let canvas = Canvas::new(w, h, fmt);
    let font = Font::load_default();
    let keyboard = Keyboard::open().context("opening keyboard")?;
    let theme = build_theme(&cfg);

    let mut layout = Layout::new();
    layout.gap = cfg.general.gap;
    layout.border = cfg.general.border;
    layout.padding_outer = cfg.general.outer_padding;

    // Workspaces.
    let mut names = HashMap::new();
    for ws in &cfg.workspaces {
        names.insert(ws.n, ws.name.clone());
    }
    let mut workspaces = Workspaces::new(9, names);

    // Первый терминал.
    let mut terminals: HashMap<LeafId, TerminalTile> = HashMap::new();
    let first_id = workspaces.current_layout_mut().open_tile(TileKind::Terminal, Direction::Horizontal);
    let cols = ((w as i32 - layout.padding_outer * 2 - layout.border * 2 - 8) / font.width as i32).max(1) as u16;
    let rows = ((h as i32 - layout.padding_outer * 2 - layout.border * 2 - 8 - font.height as i32 * 2) / font.height as i32).max(1) as u16;
    let pty = Pty::spawn(cols, rows, Some(&cfg.general.shell))?;
    set_nonblocking(pty.master_fd);
    let vterm = VTerm::new(cols, rows);
    terminals.insert(first_id, TerminalTile {
        pty, vterm,
        title: cfg.general.shell.clone(),
        workspace: 1,
    });

    // X11 compositor.
    let x11 = match x11::X11Compositor::start(1, cfg.x11.screen_size) {
        Ok(c) => Some(c),
        Err(e) => {
            log::warn!("X11 compositor not started: {} — X11 tile embedding disabled.\n\
                       Install xorg-server-xephyr to enable.", e);
            None
        }
    };

    // Mouse.
    let mut mouse = match input::Mouse::open(w, h) {
        Ok(m) => {
            log::info!("mouse initialized");
            Some(m)
        }
        Err(e) => { log::warn!("mouse not available: {}", e); None }
    };
    if let Some(m) = mouse.as_ref() {
        set_nonblocking(m.fd);
    }

    // Gamepad.
    let mut gamepad = match input::GamepadManager::new(
        cfg.gamepad.keymap.clone(),
        cfg.gamepad.stick_sensitivity,
        cfg.gamepad.enabled,
    ) {
        Ok(g) => g,
        Err(e) => { log::warn!("gamepad init failed: {}", e); input::GamepadManager::new(HashMap::new(), 50, false).unwrap() }
    };

    // Audio stack (PipeWire).
    let _audio = if cfg.audio.start_pipewire_pulse || cfg.audio.start_wireplumber {
        match audio::AudioStack::start(cfg.audio.start_pipewire_pulse, cfg.audio.start_wireplumber) {
            Ok(a) => Some(a),
            Err(e) => { log::warn!("audio stack failed: {}", e); None }
        }
    } else { None };

    // Portal backend (запуск в отдельном tokio runtime).
    let _portal_handle = if cfg.portal.start_portal {
        let service_name = cfg.portal.service_name.clone();
        let object_path = cfg.portal.object_path.clone();
        std::thread::spawn(move || {
            let rt = match tokio::runtime::Runtime::new() {
                Ok(r) => r,
                Err(e) => { log::warn!("tokio rt: {}", e); return; }
            };
            rt.block_on(async move {
                match portal::PortalBackend::start(service_name, object_path).await {
                    Ok(_) => {
                        log::info!("portal backend running");
                        // Держим поток живым.
                        loop {
                            tokio::time::sleep(std::time::Duration::from_secs(60)).await;
                        }
                    }
                    Err(e) => log::warn!("portal backend failed: {}", e),
                }
            });
        })
    } else { std::thread::spawn(|| {}) };

    // Launcher.
    let mut launcher = launcher::Launcher::new(&cfg.launcher.desktop_paths, &cfg.launcher.custom_entries);

    run_event_loop(
        backend, canvas, font, theme, keyboard, mouse.as_mut(),
        layout, workspaces, terminals, x11, gamepad, launcher, cfg,
    )
}

#[allow(clippy::too_many_arguments)]
fn run_event_loop(
    mut backend: Backend,
    canvas: Canvas,
    font: Font,
    theme: Theme,
    mut keyboard: Keyboard,
    mut mouse: Option<&mut input::Mouse>,
    layout: Layout,
    mut workspaces: Workspaces,
    mut terminals: HashMap<LeafId, TerminalTile>,
    mut x11: Option<x11::X11Compositor>,
    mut gamepad: input::GamepadManager,
    mut launcher: launcher::Launcher,
    cfg: Config,
) -> Result<()> {
    let mut popups: Vec<Popup> = Vec::new();
    let mut quit = false;
    let mut resize_mode = false;
    let mut pending_x11_tile: Option<LeafId> = None;
    let _ = layout;

    popups.push(Popup::info("SUPERHOT TTY v0.2 — Mod4+D launcher | Mod4+1..9 workspaces | Mod4+Enter term",
        canvas.width, canvas.height));

    while !quit {
        let frame_start = Instant::now();

        // 1. Клавиатура.
        let events = keyboard.poll();
        for ev in events {
            match ev {
                KeyEvent::Press(key) | KeyEvent::Repeat(key) => {
                    // Launcher priority.
                    if launcher.visible {
                        let key_str = key_to_string(&key);
                        if let Some(idx) = launcher.handle_key(&key_str) {
                            let entry = launcher.entries[idx].clone();
                            let display = cfg.x11.display.clone();
                            // Запускаем в отдельном потоке чтобы не блокировать event loop.
                            std::thread::spawn(move || {
                                let _ = launcher::Launcher::launch(&entry, &display);
                            });
                            // Создаём X11 tile для нового окна.
                            if x11.is_some() {
                                let new_id = workspaces.current_layout_mut().open_tile(TileKind::X11, Direction::Horizontal);
                                pending_x11_tile = Some(new_id);
                            }
                        }
                        continue;
                    }
                    if keyboard.super_ {
                        handle_hotkey(key, &mut workspaces, &mut terminals, &mut x11,
                            &mut popups, &mut quit, &mut resize_mode, &mut pending_x11_tile,
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
                    // Конвертируем gamepad key в строку.
                    let key_str = match gk {
                        input::GamepadKey::Press(s) | input::GamepadKey::Release(s) => s,
                    };
                    // Мапим в escape sequences.
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
            let _ = m.poll();
        }

        // 4. PTY reads.
        let mut buf = [0u8; 8192];
        let _ = buf;
        // Читаем только тайлы текущего workspace.
        let current_ws = workspaces.current;
        for (id, tile) in terminals.iter_mut() {
            if tile.workspace != current_ws { continue; }
            loop {
                let mut local_buf = [0u8; 8192];
                match tile.pty.read(&mut local_buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        let response = tile.vterm.feed(&local_buf[..n]);
                        if let Some(resp) = response {
                            let _ = tile.pty.write(resp.as_bytes());
                        }
                        if n < local_buf.len() { break; }
                    }
                    Err(e) => {
                        let s = e.to_string();
                        if s.contains("EAGAIN") || s.contains("resource temporarily") || s.contains("Would block") {
                            break;
                        }
                        break;
                    }
                }
            }
            let _ = id;
        }

        // 5. X11 poll.
        if let Some(x) = x11.as_mut() {
            if let Ok(new_windows) = x.poll_events() {
                for xid in new_windows {
                    if let Some(leaf_id) = pending_x11_tile.take() {
                        x.bind_window_to_tile(leaf_id.0, x11::XWindowId(xid));
                    }
                }
            }
            let bindings: Vec<(u64, x11::XWindowId)> = x.tile_bindings.iter()
                .map(|(k, v)| (*k, *v)).collect();
            for (_, xwid) in bindings {
                let _ = x.refresh_window(xwid.0);
            }
        }

        // 6. Render.
        render_frame(&backend, &canvas, &font, &theme, &workspaces, &terminals, &x11, &popups, &launcher, &cfg);

        // 7. Flip.
        if let Err(e) = backend.flip() {
            log::warn!("flip failed: {}", e);
        }

        // 8. Popups tick.
        for p in popups.iter_mut() { p.tick(); }
        popups.retain(|p| p.age < 240);

        // 9. Framerate.
        let elapsed = frame_start.elapsed();
        let target = Duration::from_millis(1000 / cfg.general.framerate.max(1) as u64);
        if elapsed < target {
            std::thread::sleep(target - elapsed);
        }
    }

    log::info!("superhot-tty shutting down");
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn handle_hotkey(
    key: Key,
    workspaces: &mut Workspaces,
    terminals: &mut HashMap<LeafId, TerminalTile>,
    x11: &mut Option<x11::X11Compositor>,
    popups: &mut Vec<Popup>,
    quit: &mut bool,
    resize_mode: &mut bool,
    pending_x11_tile: &mut Option<LeafId>,
    canvas: &Canvas,
    font: &Font,
    keyboard: &Keyboard,
    launcher: &mut launcher::Launcher,
    cfg: &Config,
) -> Result<()> {
    let _ = keyboard;
    // Поиск биндинга в конфиге.
    let key_str = key_to_string(&key);
    let mut matched_action = None;
    for b in &cfg.keybindings {
        if b.key.to_lowercase() != key_str.to_lowercase() { continue; }
        let mods_match =
            b.mods.iter().all(|m| match m.as_str() {
                "Super" => keyboard.super_,
                "Ctrl" => keyboard.ctrl,
                "Alt" => keyboard.alt,
                "Shift" => keyboard.shift,
                _ => false,
            });
        if mods_match {
            matched_action = Some(b.action.clone());
            break;
        }
    }

    if let Some(action) = matched_action {
        execute_action(action, workspaces, terminals, x11, popups, quit, resize_mode,
            pending_x11_tile, canvas, font, launcher, cfg)?;
    } else {
        // Fallback на старые hotkeys если не нашли в конфиге.
        match key {
            Key::Enter => spawn_term(workspaces, terminals, Direction::Horizontal, canvas, font, cfg),
            Key::Char('d') | Key::Char('D') => launcher.toggle(),
            Key::Char('v') | Key::Char('V') => spawn_term(workspaces, terminals, Direction::Vertical, canvas, font, cfg),
            Key::Char('h') | Key::Char('H') => workspaces.current_layout_mut().focus(FocusDir::Left),
            Key::Char('j') | Key::Char('J') => workspaces.current_layout_mut().focus(FocusDir::Down),
            Key::Char('k') | Key::Char('K') => workspaces.current_layout_mut().focus(FocusDir::Up),
            Key::Char('l') | Key::Char('L') => workspaces.current_layout_mut().focus(FocusDir::Right),
            Key::Char('q') | Key::Char('Q') => close_focused(workspaces, terminals, x11),
            Key::Char('f') | Key::Char('F') => workspaces.current_layout_mut().toggle_fullscreen(),
            Key::Space => workspaces.current_layout_mut().focus_cycle(),
            Key::Char('r') | Key::Char('R') => {
                *resize_mode = !*resize_mode;
                popups.push(Popup::info(
                    if *resize_mode { "RESIZE — HJKL to resize, Esc to exit" }
                    else { "resize mode off" },
                    canvas.width, canvas.height));
            }
            Key::Char('e') | Key::Char('E') => {
                if keyboard.shift { *quit = true; return Ok(()); }
                // open X11 tile.
                if x11.is_none() {
                    popups.push(Popup::info("X11 not available (install xorg-server-xephyr)", canvas.width, canvas.height));
                    return Ok(());
                }
                let new_id = workspaces.current_layout_mut().open_tile(TileKind::X11, Direction::Horizontal);
                *pending_x11_tile = Some(new_id);
                popups.push(Popup::info("Run: DISPLAY=:1 discord", canvas.width, canvas.height));
            }
            Key::Escape => { *resize_mode = false; popups.clear(); }
            _ => {}
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn execute_action(
    action: config::Action,
    workspaces: &mut Workspaces,
    terminals: &mut HashMap<LeafId, TerminalTile>,
    x11: &mut Option<x11::X11Compositor>,
    popups: &mut Vec<Popup>,
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
    let split_dir = |d: CfgDir| match d {
        CfgDir::Left | CfgDir::Right => Direction::Horizontal,
        CfgDir::Up | CfgDir::Down => Direction::Vertical,
    };
    match action {
        Terminal => spawn_term(workspaces, terminals, Direction::Horizontal, canvas, font, cfg),
        Launcher => launcher.toggle(),
        Spawn { cmd, args } => {
            let _ = std::process::Command::new(&cmd).args(&args).spawn();
        }
        SpawnX11 { cmd, args } => {
            if let Some(x) = x11.as_mut() {
                let new_id = workspaces.current_layout_mut().open_tile(TileKind::X11, Direction::Horizontal);
                *pending_x11_tile = Some(new_id);
                let args_ref: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
                let _ = x.launch_client(&cmd, &args_ref);
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
            popups.push(Popup::info(
                if *resize_mode { "RESIZE — HJKL to resize, Esc to exit" }
                else { "resize mode off" },
                canvas.width, canvas.height));
        }
        Resize { dir, delta } => workspaces.current_layout_mut().resize_focused(dir_map(dir), delta),
        CycleFocus => workspaces.current_layout_mut().focus_cycle(),
        Quit => *quit = true,
        TabNext | TabPrev | ToggleLayout | Reload => {
            popups.push(Popup::info(&format!("action {:?} not implemented yet", action), canvas.width, canvas.height));
        }
    }
    let _ = split_dir;
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
    let layout = workspaces.current_layout_mut();
    let cols = ((canvas.width as i32 - layout.padding_outer * 2 - layout.border * 2 - 8) / font.width as i32).max(1) as u16;
    let rows = ((canvas.height as i32 - layout.padding_outer * 2 - layout.border * 2 - 8 - font.height as i32 * 2) / font.height as i32).max(1) as u16;
    let new_id = layout.open_tile(TileKind::Terminal, dir);
    match Pty::spawn(cols.min(200), rows.min(80), Some(&cfg.general.shell)) {
        Ok(pty) => {
            set_nonblocking(pty.master_fd);
            terminals.insert(new_id, TerminalTile {
                pty,
                vterm: VTerm::new(cols.min(200), rows.min(80)),
                title: cfg.general.shell.clone(),
                workspace: workspaces.current,
            });
            log::info!("new terminal tile: {:?} on ws {}", new_id, workspaces.current);
        }
        Err(e) => {
            log::error!("pty spawn: {}", e);
            workspaces.current_layout_mut().close_leaf(new_id);
        }
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
            log::info!("closed tile {:?}", focused_id);
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn render_frame(
    backend: &Backend,
    canvas: &Canvas,
    font: &Font,
    theme: &Theme,
    workspaces: &Workspaces,
    terminals: &HashMap<LeafId, TerminalTile>,
    x11: &Option<x11::X11Compositor>,
    popups: &[Popup],
    launcher: &launcher::Launcher,
    cfg: &Config,
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

        // ASCII рамка в MCD стиле: уголки + палочки.
        draw_ascii_border(canvas, rect, theme, focused, *kind);

        let border_color = border_color_for(*kind, focused, theme);
        if focused {
            canvas.neon_border(rect.x, rect.y, rect.w, rect.h, border_color);
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
                                &format!("X11 win 0x{:x} (no backing)", xwid.0),
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
            TileKind::Terminal => terminals.get(leaf_id)
                .map(|t| t.title.clone())
                .unwrap_or_else(|| "term".to_string()),
            TileKind::X11 => "x11".to_string(),
        };
        text.draw_text(rect.x + 8, rect.y + 2, &title,
            if focused { theme.accent_magenta } else { theme.fg_dim },
            Some(if focused { theme.tile_bg_active } else { theme.tile_bg_inactive }));
    }

    // Launcher popup.
    launcher.render(canvas, font, theme, canvas.width, canvas.height);

    // Popups.
    for p in popups {
        p.render(canvas, theme);
    }

    // Status bar.
    render_status_bar(canvas, font, theme, workspaces, cfg);

    // Blit canvas → backend.
    let canvas_data = canvas.data.lock();
    let (backend_buf_ptr_mut, backend_len, backend_stride, backend_h) = match backend {
        Backend::Drm(d) => (d.back.mmap_addr as *mut u8, d.back.size as usize, d.back.stride as usize, d.height as usize),
        Backend::Fbdev(f) => (f.mmap_addr as *mut u8, f.size, f.stride as usize, f.height as usize),
    };
    let canvas_stride = canvas.stride as usize;
    let min_stride = canvas_stride.min(backend_stride);
    let rows = canvas.height as usize;
    for r in 0..rows {
        if r >= backend_h { break; }
        let src_off = r * canvas_stride;
        let dst_off = r * backend_stride;
        let n = min_stride.min(canvas_data.len() - src_off).min(backend_len - dst_off);
        unsafe {
            std::ptr::copy_nonoverlapping(canvas_data.as_ptr().add(src_off),
                                          backend_buf_ptr_mut.add(dst_off), n);
        }
    }
}

/// ASCII-рамка в стиле MCD: уголки `╔╗╚╝` + боковые `║` `═`.
fn draw_ascii_border(canvas: &Canvas, rect: &Rect, theme: &Theme, focused: bool, kind: TileKind) {
    let _ = (canvas, rect, theme, focused, kind);
    // Рамку рисуем только как индикатор фокуса — пока рисуем через neon_border.
    // TODO: добавить ASCII уголки через шрифт в углах плиток.
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

fn render_x11_window(
    canvas: &Canvas,
    backing: &[u32],
    src_w: u16,
    src_h: u16,
    rect: &Rect,
) {
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

    // Workspace indicators.
    for n in 1..=workspaces.max {
        let name = workspaces.names.get(&n).cloned().unwrap_or_else(|| n.to_string());
        let is_current = workspaces.current == n;
        let label = format!(" {}:{} ", n, name);
        let color = if is_current { theme.accent_magenta } else { theme.fg_dim };
        if is_current {
            canvas.fill_rect(x, y + 2, (label.len() as u32) * font.width, font.height as u32,
                Color(0x20, 0x10, 0x40));
        }
        text_renderer.draw_text(x, y + 4, &label, color, None);
        x += (label.len() as i32 + 1) * font.width as i32;
    }

    // Hint.
    let hint = format!("| tiles:{} | Mod4+D launcher | Mod4+1..9 ws | Mod4+Enter term | Mod4+R resize",
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
