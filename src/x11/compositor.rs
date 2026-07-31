//! X11 compositor-клиент.
//!
//! Архитектура:
//!   1. Запускаем Xvfb (X virtual framebuffer) в фоне, на отдельном дисплее :1.
//!      Раньше использовали Xephyr, но Xephyr — это nested X server, которому
//!      нужен host display (родительское X-окно). На чистом TTY (без X-сервера)
//!      Xephyr падает с "Xephyr cannot open host display. Is DISPLAY set?".
//!      Xvfb этого не требует — он сам рисует в memory framebuffer.
//!   2. Подключаемся к нему через x11rb, включаем Composite extension.
//!   3. Перенаправляем redirect_subwindows на root — теперь каждый top-level
//!      X-клиент становится отдельным окном, которое мы можем захватить.
//!   4. Подписываемся на CreateNotify/ConfigureNotify/DestroyNotify.
//!   5. При отрисовке читаем пиксели окна через XGetImage.
//!
//! Пользователь запускает X-клиентов:
//!   DISPLAY=:1 discord
//! Менеджер автоматически находит новое окно и привязывает его к текущей плитке.

use anyhow::{Context, Result};
use x11rb::connection::Connection as _;
use x11rb::protocol::composite::{self, Redirect};
use x11rb::protocol::damage::{self, Damage};
use x11rb::protocol::xproto::*;
use x11rb::rust_connection::RustConnection;
use std::process::{Command, Child};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct XWindowId(pub u32);

pub struct TrackedWindow {
    pub xid: u32,
    pub damage: Damage,
    pub width: u16,
    pub height: u16,
    /// ARGB backing store, layout: row-major, top-to-bottom, len = w*h.
    pub backing: Vec<u32>,
    pub dirty: bool,
    /// Window title (from WM_NAME). Stored for future display in tile border
    /// or title bar; currently not rendered.
    #[allow(dead_code)]
    pub title: String,
}

pub struct X11Compositor {
    pub conn: RustConnection,
    /// Root window XID. Stored for future root-window property queries
    /// (e.g. _NET_CLIENT_LIST); currently not used after init.
    #[allow(dead_code)]
    pub root: u32,
    pub windows: Vec<TrackedWindow>,
    /// Child process of the X server (Xvfb). Named `xephyr` for backward
    /// compatibility with field accessors; actually holds Xvfb.
    pub xephyr: Option<Child>,
    pub display: String,
    pub tile_bindings: std::collections::HashMap<u64, XWindowId>,
}

impl X11Compositor {
    pub fn start(display_num: u32, screen_size: (u16, u16)) -> Result<Self> {
        let display = format!(":{}", display_num);

        // Перед запуском Xvfb подготавливаем окружение:
        //   1. Создаём /tmp/.X11-unix если нет (Xvfb как non-root не может создать
        //      сам — пишет "_XSERVTransmkdir: ERROR: euid != 0" и fallback'ит на
        //      abstract socket, что работает, но некоторые X-клиенты ищут только
        //      /tmp/.X11-unix/X<N> и не находят его).
        //   2. Убиваем leftover Xvfb с предыдущего краша (если WM упал, Xvfb
        //      остаётся running на том же display — новый Xvfb не сможет bind).
        //   3. Устанавливаем TMPDIR — xkbcomp пишет /tmp/server-<n>.xkm, в
        //      PrivateTmp systemd namespace /tmp должен быть writable, но
        //      на всякий случай указываем явно.
        prepare_x11_env(display_num);
        kill_leftover_xvfb(display_num);

        // Пробуем сначала Xvfb — он не требует host display (в отличие от Xephyr).
        // Xvfb работает на чистом TTY, в headless-конфигурациях и в контейнерах.
        //
        // ВАЖНО: `-screen scrn WxHxD` — это ТРИ отдельных argv значения.
        // Раньше мы делали .arg(format!("0 {}x{}x24", w, h)) — это порождало
        // ОДИН аргумент "0 1024x768x24" (с пробелом внутри), Xorg парсил его
        // как scrn="0 1024x768x24", а следующий argv ("-nolisten") — как WxHxD,
        // и падал с "Invalid screen configuration -nolisten for -screen 0".
        let xvfb = Command::new("Xvfb")
            .arg(&display)
            .arg("-screen")
            .arg("0")
            .arg(format!("{}x{}x24", screen_size.0, screen_size.1))
            .arg("-nolisten")
            .arg("tcp")
            .arg("-ac")  // disable access control (Xvfb на отдельном дисплее, изолирован)
            // Явно не наследуем DISPLAY от родителя — иначе Xvfb может пытаться
            // использовать родительский дисплей (что бессмысленно для него).
            .env_remove("DISPLAY")
            .spawn();

        let xserver_child = match xvfb {
            Ok(child) => child,
            Err(e) => {
                // Fallback на Xephyr если Xvfb не установлен (для обратной совместимости).
                // На TTY это скорее всего тоже упадёт, но возможно у пользователя
                // настроен host X server (например, через xinit).
                log::warn!("Xvfb launch failed ({}), falling back to Xephyr", e);
                Command::new("Xephyr")
                    .arg(&display)
                    .arg("-screen")
                    .arg(format!("{}x{}", screen_size.0, screen_size.1))
                    .arg("-reset")
                    .arg("-terminate")
                    .arg("-nolisten")
                    .arg("tcp")
                    .spawn()
                    .context("failed to launch Xvfb or Xephyr — install xvfb package")?
            }
        };

        // Ждём подключения — Xvfb может стартовать 200-1500мс в зависимости
        // от нагрузки и от того, холодный ли cache. Делаем несколько попыток
        // с экспоненциальным backoff вместо одной sleep+connect — иначе на
        // медленных системах WM запускается без X11 и все X-клиенты падают
        // с "can't open display".
        let (conn, _) = {
            let mut last_err = None;
            let mut delay = std::time::Duration::from_millis(100);
            let mut connected = None;
            for attempt in 0..15u32 {
                if attempt > 0 {
                    std::thread::sleep(delay);
                    delay = delay.saturating_mul(2).min(std::time::Duration::from_millis(800));
                }
                match x11rb::connect(Some(&display)) {
                    Ok(c) => { connected = Some(c); break; }
                    Err(e) => {
                        log::debug!("X11 connect attempt {} failed: {}", attempt + 1, e);
                        last_err = Some(e);
                    }
                }
            }
            match connected {
                Some(c) => c,
                None => return Err(anyhow::anyhow!("connecting to Xvfb/Xephyr — server may not be ready: {}",
                    last_err.map(|e| e.to_string()).unwrap_or_default())),
            }
        };

        // Проверяем composite extension.
        match composite::query_version(&conn, 0, 4) {
            Ok(r) => { let _ = r.reply(); }
            Err(e) => log::warn!("composite query_version failed: {}", e),
        }
        match damage::query_version(&conn, 1, 1) {
            Ok(r) => { let _ = r.reply(); }
            Err(e) => log::warn!("damage query_version failed: {}", e),
        }

        let root = conn.setup().roots[0].root;

        // Redirect all subwindows of root.
        let _ = composite::redirect_subwindows(&conn, root, Redirect::MANUAL);
        // Подписываемся на SubstructureNotify на root.
        let event_mask = EventMask::SUBSTRUCTURE_NOTIFY | EventMask::EXPOSURE;
        let _ = change_window_attributes(&conn, root, &ChangeWindowAttributesAux::new()
            .event_mask(event_mask));
        let _ = conn.flush();

        log::info!("X11 compositor started on {} (root=0x{:x})", display, root);

        Ok(X11Compositor {
            conn,
            root,
            windows: Vec::new(),
            xephyr: Some(xserver_child),
            display,
            tile_bindings: std::collections::HashMap::new(),
        })
    }

    pub fn poll_events(&mut self) -> Result<Vec<u32>> {
        let mut new_windows = Vec::new();
        while let Ok(Some(ev)) = self.conn.poll_for_event() {
            match ev {
                x11rb::protocol::Event::CreateNotify(c) => {
                    let xid = c.window;
                    log::info!("X CreateNotify: 0x{:x} ({}x{})", xid, c.width, c.height);
                    if !c.override_redirect && c.width > 1 && c.height > 1 {
                        self.register_window(xid, c.width, c.height);
                        new_windows.push(xid);
                    }
                }
                x11rb::protocol::Event::MapRequest(m) => {
                    let _ = map_window(&self.conn, m.window);
                    let _ = self.conn.flush();
                }
                x11rb::protocol::Event::ConfigureNotify(c) => {
                    if let Some(w) = self.windows.iter_mut().find(|w| w.xid == c.window) {
                        if w.width != c.width || w.height != c.height {
                            w.width = c.width;
                            w.height = c.height;
                            w.backing = vec![0; (c.width as usize) * (c.height as usize)];
                            w.dirty = true;
                        }
                    }
                }
                x11rb::protocol::Event::DestroyNotify(d) => {
                    // Уничтожаем damage object для удаляемого окна, иначе
                    // будет leak X server resources (damage handles не GC'ятся).
                    if let Some(w) = self.windows.iter().find(|w| w.xid == d.window) {
                        if w.damage != 0 {
                            let _ = damage::destroy(&self.conn, w.damage);
                        }
                    }
                    self.windows.retain(|w| w.xid != d.window);
                    log::info!("X DestroyNotify: 0x{:x}", d.window);
                }
                x11rb::protocol::Event::Expose(e) => {
                    if let Some(w) = self.windows.iter_mut().find(|w| w.xid == e.window) {
                        w.dirty = true;
                    }
                }
                _ => {}
            }
        }
        Ok(new_windows)
    }

    fn register_window(&mut self, xid: u32, w: u16, h: u16) {
        // Создаём damage object для отслеживания изменений окна.
        // Правильный flow: generate_id() → damage::create(drawable, damage_id, level).
        // Сохраняем damage_id чтобы уничтожить объект при DestroyNotify —
        // иначе X server будет leak'ать damage handles.
        let damage_handle = match self.conn.generate_id() {
            Ok(damage_id) => {
                match damage::create(&self.conn, xid, damage_id, damage::ReportLevel::NON_EMPTY) {
                    Ok(_) => {
                        let _ = self.conn.flush();
                        damage_id
                    }
                    Err(e) => {
                        log::warn!("damage::create failed for 0x{:x}: {}", xid, e);
                        0
                    }
                }
            }
            Err(e) => {
                log::warn!("generate_id for damage failed: {}", e);
                0
            }
        };
        let backing = vec![0; (w as usize) * (h as usize)];
        self.windows.push(TrackedWindow {
            xid,
            damage: damage_handle,
            width: w, height: h,
            backing,
            dirty: true,
            title: format!("win-0x{:x}", xid),
        });
        let _ = self.conn.flush();
    }

    pub fn refresh_window(&mut self, xid: u32) -> Result<bool> {
        let idx = self.windows.iter().position(|w| w.xid == xid);
        let Some(idx) = idx else { return Ok(false); };
        self.windows[idx].dirty = false;
        let width = self.windows[idx].width;
        let height = self.windows[idx].height;
        if width == 0 || height == 0 { return Ok(false); }

        let img = get_image(&self.conn, ImageFormat::Z_PIXMAP, xid, 0, 0, width, height, 0xffffffff)?
            .reply()?;

        let depth = img.depth;
        let bytes_per_pixel = ((depth + 7) / 8) as usize;
        let expected_len = (width as usize) * (height as usize) * bytes_per_pixel;
        if img.data.len() < expected_len {
            log::warn!("XGetImage truncated: got {}, expected {}", img.data.len(), expected_len);
            return Ok(false);
        }

        let w = &mut self.windows[idx];
        w.backing.resize((width as usize) * (height as usize), 0);
        match bytes_per_pixel {
            4 => {
                for i in 0..(width as usize) * (height as usize) {
                    let off = i * 4;
                    let b = img.data[off] as u32;
                    let g = img.data[off + 1] as u32;
                    let r = img.data[off + 2] as u32;
                    let a = if depth == 32 { img.data[off + 3] as u32 } else { 255 };
                    w.backing[i] = (a << 24) | (r << 16) | (g << 8) | b;
                }
            }
            3 => {
                for i in 0..(width as usize) * (height as usize) {
                    let off = i * 3;
                    let b = img.data[off] as u32;
                    let g = img.data[off + 1] as u32;
                    let r = img.data[off + 2] as u32;
                    w.backing[i] = (255 << 24) | (r << 16) | (g << 8) | b;
                }
            }
            _ => return Ok(false),
        }
        Ok(true)
    }

    pub fn window_backing(&self, xid: u32) -> Option<(&[u32], u16, u16)> {
        self.windows.iter()
            .find(|w| w.xid == xid)
            .map(|w| (w.backing.as_slice(), w.width, w.height))
    }

    pub fn launch_client(&self, cmd: &str, args: &[&str]) -> Result<Child> {
        Command::new(cmd)
            .args(args)
            .env("DISPLAY", &self.display)
            .spawn()
            .with_context(|| format!("launching {}", cmd))
    }

    pub fn bind_window_to_tile(&mut self, leaf_id: u64, xid: XWindowId) {
        self.tile_bindings.insert(leaf_id, xid);
    }

    pub fn unbind_tile(&mut self, leaf_id: u64) {
        self.tile_bindings.remove(&leaf_id);
    }

    pub fn tile_window(&self, leaf_id: u64) -> Option<XWindowId> {
        self.tile_bindings.get(&leaf_id).copied()
    }

    pub fn shutdown(&mut self) {
        // Уничтожаем все damage objects чтобы не leak'ать X server resources.
        for w in &self.windows {
            if w.damage != 0 {
                let _ = damage::destroy(&self.conn, w.damage);
            }
        }
        self.windows.clear();
        let _ = self.conn.flush();
        if let Some(mut x) = self.xephyr.take() {
            let _ = x.kill();
        }
    }
}

impl Drop for X11Compositor {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// Подготавливает окружение для Xvfb:
///   - Создаёт /tmp/.X11-unix (mode 1777) если не существует.
///     Xvfb как non-root не может создать сам, и fallback на abstract socket
///     работает не для всех X-клиентов.
///   - Устанавливает TMPDIR в /tmp явно — xkbcomp пишет /tmp/server-<n>.xkm
///     и иногда не находит /tmp если systemd PrivateTmp меняет namespace.
fn prepare_x11_env(display_num: u32) {
    use std::os::unix::fs::DirBuilderExt;
    // /tmp/.X11-unix — стандартная директория для Unix-domain X сокетов.
    let x11_unix = "/tmp/.X11-unix";
    if !std::path::Path::new(x11_unix).exists() {
        match std::fs::DirBuilder::new().mode(0o1777).create(x11_unix) {
            Ok(_) => log::debug!("created {} (mode 1777)", x11_unix),
            Err(e) => {
                // Не критично — Xvfb fallback'ит на abstract socket.
                log::debug!("could not create {} ({}): Xvfb will use abstract socket", x11_unix, e);
            }
        }
    }
    // Удаляем stale socket file если есть (от предыдущего краша Xvfb).
    let sock = format!("/tmp/.X11-unix/X{}", display_num);
    let _ = std::fs::remove_file(&sock);
    // Также удаляем lock file.
    let lock = format!("/tmp/.X{}-lock", display_num);
    let _ = std::fs::remove_file(&lock);
    // TMPDIR — xkbcomp пишет сюда.
    if std::env::var("TMPDIR").is_err() {
        std::env::set_var("TMPDIR", "/tmp");
    }
}

/// Убивает leftover Xvfb с предыдущего запуска (если WM упал, Xvfb остаётся
/// running на том же display — новый Xvfb не сможет bind).
///
/// Ищем процесс с argv содержит "Xvfb" и ":<display_num>".
/// Используем /proc вместо pgrep (pgrep может быть не установлен).
fn kill_leftover_xvfb(display_num: u32) {
    let display_arg = format!(":{}", display_num);
    let mut killed = 0u32;
    if let Ok(entries) = std::fs::read_dir("/proc") {
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name_str = match name.to_str() { Some(s) => s, None => continue };
            let pid: u32 = match name_str.parse() { Ok(p) => p, Err(_) => continue };
            if pid <= 1 { continue; } // never kill init
            // Читаем cmdline.
            let cmdline_path = format!("/proc/{}/cmdline", pid);
            let cmdline = match std::fs::read(&cmdline_path) { Ok(c) => c, Err(_) => continue };
            // cmdline — NUL-separated argv.
            let argv: Vec<&str> = cmdline.split(|&b| b == 0)
                .filter(|s| !s.is_empty())
                .filter_map(|s| std::str::from_utf8(s).ok())
                .collect();
            // Ищем "Xvfb" в argv[0] и ":<n>" в любом argv.
            let is_xvfb = argv.first().map(|s| {
                let base = s.rsplit('/').next().unwrap_or(s);
                base == "Xvfb"
            }).unwrap_or(false);
            if is_xvfb && argv.iter().any(|a| *a == display_arg) {
                // SIGTERM first, then SIGKILL if still alive.
                unsafe {
                    if libc::kill(pid as i32, libc::SIGTERM) == 0 {
                        killed += 1;
                        log::info!("killing leftover Xvfb (pid={})", pid);
                        // Ждём до 500мс пока процесс умрёт.
                        for _ in 0..50 {
                            if libc::kill(pid as i32, 0) != 0 { break; }
                            std::thread::sleep(std::time::Duration::from_millis(10));
                        }
                        // Если ещё жив — SIGKILL.
                        if libc::kill(pid as i32, 0) == 0 {
                            libc::kill(pid as i32, libc::SIGKILL);
                            log::warn!("Xvfb pid={} did not exit on SIGTERM, sent SIGKILL", pid);
                        }
                    }
                }
            }
        }
    }
    if killed > 0 {
        log::info!("killed {} leftover Xvfb process(es) on display :{}", killed, display_num);
    }
}
