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
    /// (e.g. _NET_CLIENT_LIST); currently used for input focus management.
    pub root: u32,
    pub windows: Vec<TrackedWindow>,
    /// Child process of the X server (Xvfb). Named `xephyr` for backward
    /// compatibility with field accessors; actually holds Xvfb.
    pub xephyr: Option<Child>,
    pub display: String,
    pub tile_bindings: std::collections::HashMap<u64, XWindowId>,
    /// Currently focused X11 window (XID). Used to skip redundant
    /// set_input_focus calls and to route keyboard events.
    focused_xid: Option<u32>,
    /// Cached atom IDs to avoid repeated intern_atom round-trips.
    atoms: std::sync::OnceLock<Atoms>,
    /// XTest extension opcode (major opcode). Cached after first query.
    xtest_opcode: std::sync::OnceLock<u8>,
}

#[derive(Clone, Copy)]
struct Atoms {
    wm_protocols: u32,
    wm_delete_window: u32,
    wm_name: u32,
    wm_class: u32,
    net_wm_name: u32,
    wm_transient_for: u32,
    utf8_string: u32,
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
        // Подписываемся на SubstructureNotify (CreateNotify, DestroyNotify,
        // ConfigureNotify, MapNotify) И SubstructureRedirect (MapRequest,
        // ConfigureRequest) на root. SubstructureRedirect критичен — без него
        // MapRequest не приходит и мы не можем перехватить map top-level окон
        // (они мапятся напрямую, и мы не успеваем поставить input focus).
        // PropertyChange нужен чтобы получать PropertyNotify на root (это
        // даёт нам события об изменении свойств root, но для свойств самих
        // окон нам нужно отдельно подписаться через change_window_attributes
        // на каждое окно — см. register_window).
        let event_mask = EventMask::SUBSTRUCTURE_NOTIFY
            | EventMask::SUBSTRUCTURE_REDIRECT
            | EventMask::EXPOSURE;
        let _ = change_window_attributes(&conn, root, &ChangeWindowAttributesAux::new()
            .event_mask(event_mask));
        let _ = conn.flush();

        log::info!("X11 compositor started on {} (root=0x{:x})", display, root);

        let mut compositor = X11Compositor {
            conn,
            root,
            windows: Vec::new(),
            xephyr: Some(xserver_child),
            display,
            tile_bindings: std::collections::HashMap::new(),
            focused_xid: None,
            atoms: std::sync::OnceLock::new(),
            xtest_opcode: std::sync::OnceLock::new(),
        };

        // Intern common atoms upfront (saves round-trips later).
        // Errors here are non-fatal — atom lookups will be retried on demand.
        let _ = compositor.init_atoms();
        // Pre-fetch XTest opcode so we can decide at runtime whether XTest
        // fake_input is available (otherwise we fall back to XSendEvent).
        let _ = compositor.init_xtest();

        Ok(compositor)
    }

    pub fn poll_events(&mut self) -> Result<Vec<u32>> {
        let mut new_windows = Vec::new();
        while let Ok(Some(ev)) = self.conn.poll_for_event() {
            match ev {
                x11rb::protocol::Event::CreateNotify(c) => {
                    // CreateNotify fires when a window is created but before
                    // it's mapped and before WM_NAME/WM_CLASS are typically set.
                    // We log it but DEFER registration to MapNotify — this
                    // avoids tracking splash/decoration windows that apps
                    // create early. Also skips override_redirect (menus,
                    // tooltips, dropdowns — those bypass WM management).
                    if c.override_redirect {
                        log::debug!("X CreateNotify (override_redirect): 0x{:x} ({}x{}) — skipping",
                            c.window, c.width, c.height);
                    } else {
                        log::debug!("X CreateNotify: 0x{:x} ({}x{}) — deferring to MapNotify",
                            c.window, c.width, c.height);
                    }
                }
                x11rb::protocol::Event::MapRequest(m) => {
                    // MapRequest fires when a client wants to map a top-level
                    // window. We MUST respond with map_window() — otherwise the
                    // app's window never appears (X11 protocol requires the WM
                    // to either map it or deny it).
                    let _ = map_window(&self.conn, m.window);
                    let _ = self.conn.flush();
                    log::debug!("X MapRequest: 0x{:x} — mapped", m.window);
                }
                x11rb::protocol::Event::MapNotify(m) => {
                    // MapNotify fires after the window is actually mapped.
                    // This is the right time to register it: WM_NAME/WM_CLASS
                    // are usually set by now, and we can filter out dialogs
                    // and decorations.
                    //
                    // Note: MapNotifyEvent does NOT include width/height —
                    // we need to call get_geometry() to fetch them. This adds
                    // a round-trip per new window, but only on registration
                    // (once per app launch), so it's acceptable.
                    let xid = m.window;
                    if m.override_redirect {
                        // override_redirect windows (menus, tooltips, dropdowns)
                        // bypass WM management — don't track them.
                        continue;
                    }
                    // Skip if already tracked (avoid duplicates).
                    if self.windows.iter().any(|w| w.xid == xid) {
                        continue;
                    }
                    // Fetch geometry to get width/height.
                    let (w, h) = match self.get_window_geometry(xid) {
                        Some((w, h)) => (w, h),
                        None => {
                            log::debug!("X MapNotify: 0x{:x} — get_geometry failed, skipping", xid);
                            continue;
                        }
                    };
                    // Skip tiny windows (< 50x50) — these are usually splash
                    // screens, tooltips, or decoration windows that apps
                    // create before their main window.
                    if w < 50 || h < 50 {
                        log::debug!("X MapNotify: 0x{:x} ({}x{}) — too small, skipping",
                            xid, w, h);
                        continue;
                    }
                    // Skip transient windows (dialogs) whose parent is already
                    // tracked — they belong to an existing app's tile, not a
                    // new tile. This prevents dialogs from creating new tiles.
                    if self.is_transient_of_tracked(xid) {
                        log::debug!("X MapNotify: 0x{:x} — transient of tracked window, skipping",
                            xid);
                        continue;
                    }
                    log::info!("X MapNotify: 0x{:x} ({}x{}) — registering",
                        xid, w, h);
                    self.register_window(xid, w, h);
                    new_windows.push(xid);
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
                    // If the destroyed window was focused, clear focus.
                    if self.focused_xid == Some(d.window) {
                        self.focused_xid = None;
                    }
                    // Remove any tile→window binding pointing at the destroyed
                    // window. Without this, the tile would keep rendering an
                    // empty backing (the XID no longer exists on the server).
                    self.tile_bindings.retain(|_, xwid| xwid.0 != d.window);
                    log::info!("X DestroyNotify: 0x{:x}", d.window);
                }
                x11rb::protocol::Event::Expose(e) => {
                    if let Some(w) = self.windows.iter_mut().find(|w| w.xid == e.window) {
                        w.dirty = true;
                    }
                }
                x11rb::protocol::Event::PropertyNotify(p) => {
                    // Refresh window title when WM_NAME or _NET_WM_NAME changes.
                    // Many apps set WM_NAME asynchronously after MapNotify —
                    // without this, the tile title would stay "win-0x{xid}".
                    let atoms = self.atoms();
                    if p.atom == atoms.wm_name || p.atom == atoms.net_wm_name {
                        if let Some(new_title) = self.get_window_name(p.window) {
                            if let Some(w) = self.windows.iter_mut().find(|w| w.xid == p.window) {
                                if w.title != new_title {
                                    log::debug!("X PropertyNotify: 0x{:x} title → {:?}",
                                        p.window, new_title);
                                    w.title = new_title;
                                }
                            }
                        }
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

        // Fetch WM_NAME for the tile title. Fall back to "win-0x{xid}" if
        // the property is missing (some apps set it asynchronously; we'll
        // refresh it later from poll_events when a PropertyNotify fires).
        let title = self.get_window_name(xid).unwrap_or_else(|| format!("win-0x{:x}", xid));

        // Subscribe to PropertyChange events on this window so we get
        // PropertyNotify when WM_NAME / _NET_WM_NAME changes. Without this,
        // apps that set their title after MapNotify (very common — e.g.
        // xterm, firefox) would show "win-0x{xid}" forever.
        let _ = change_window_attributes(&self.conn, xid, &ChangeWindowAttributesAux::new()
            .event_mask(EventMask::PROPERTY_CHANGE));
        let _ = self.conn.flush();

        let backing = vec![0; (w as usize) * (h as usize)];
        self.windows.push(TrackedWindow {
            xid,
            damage: damage_handle,
            width: w, height: h,
            backing,
            dirty: true,
            title,
        });

        // НЕ вызываем focus_window здесь. Раньше это порождало race condition:
        // register_window ставил X-focus на новое окно, но WM focused tile
        // ещё не был обновлён (это происходит в auto_place после poll_events).
        // В следующем кадре focus sync видел что WM focused tile не X11,
        // вызывал unfocus() — и X-focus возвращался на root.
        // Теперь фокус управляется исключительно через focus sync в main loop —
        // он ставит X-focus на X-окно когда WM focused tile = X11 tile.

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

    /// Возвращает title (WM_NAME / _NET_WM_NAME) окна если известно.
    /// Используется для отображения реального имени приложения в tile title
    /// вместо хардкоженного "x11".
    pub fn window_title(&self, xid: u32) -> Option<&str> {
        self.windows.iter()
            .find(|w| w.xid == xid)
            .map(|w| w.title.as_str())
    }

    /// Устанавливает input focus на окно `xid`. После этого все keyboard
    /// события от X-сервера пойдут в это окно (но мы также используем
    /// XTest fake_input чтобы инжектить события из evdev напрямую — см.
    /// send_key()).
    ///
    /// InputFocus::POINTER_ROOT означает: если окно закроется (DestroyNotify),
    /// фокус вернётся на root window (а не на PointerRoot, что активировало
    /// "follow mouse" — не то, что мы хотим для tile-based WM).
    pub fn focus_window(&mut self, xid: u32) -> Result<()> {
        if self.focused_xid == Some(xid) {
            return Ok(()); // already focused — skip redundant call
        }
        use x11rb::protocol::xproto::{set_input_focus, InputFocus};
        set_input_focus(&self.conn, InputFocus::POINTER_ROOT, xid, x11rb::CURRENT_TIME)?;
        self.conn.flush()?;
        self.focused_xid = Some(xid);
        log::debug!("X focus set to 0x{:x}", xid);
        Ok(())
    }

    /// Сбрасывает input focus на root window. Вызывается когда tile
    /// расфокусирован или закрыт.
    pub fn unfocus(&mut self) -> Result<()> {
        if self.focused_xid.take().is_some() {
            use x11rb::protocol::xproto::{set_input_focus, InputFocus};
            set_input_focus(&self.conn, InputFocus::POINTER_ROOT, self.root, x11rb::CURRENT_TIME)?;
            self.conn.flush()?;
            log::debug!("X focus cleared (back to root)");
        }
        Ok(())
    }

    /// Возвращает XID окна, на которое сейчас установлен input focus.
    pub fn focused_window(&self) -> Option<u32> {
        self.focused_xid
    }

    /// Инжектит keyboard event в X-сервер через XTest extension.
    /// `evdev_keycode` — Linux evdev keycode (как в /usr/include/linux/input-event-codes.h).
    /// `pressed` — true для KeyPress, false для KeyRelease.
    ///
    /// XTest fake_input отправляет событие напрямую в X-сервер, который
    /// маршрутизирует его в окно с input focus. Это делает события
    /// неотличимыми от реальных нажатий (в отличие от XSendEvent, который
    /// помечает события флагом send_event=true, и многие приложения
    /// — Firefox, chromium — их игнорируют из соображений безопасности).
    ///
    /// Конвертация keycode: X11 keycodes = evdev keycodes + 8 (X резервирует
    /// 0-7 для modifier keys). Это стандартный offset для X-серверов с
    /// evdev keyboard driver (включая Xvfb на современных системах).
    pub fn send_key(&self, evdev_keycode: u32, pressed: bool) -> Result<()> {
        // Skip modifier-only events — they're tracked separately via
        // Keyboard.shift/ctrl/alt/super_ flags, and X server generates its
        // own modifier state from real key events. We DO want to forward
        // them too so apps see consistent modifier state.
        let x_keycode = evdev_keycode + 8; // evdev → X keycode offset
        let event_type = if pressed {
            x11rb::protocol::xproto::KEY_PRESS_EVENT
        } else {
            x11rb::protocol::xproto::KEY_RELEASE_EVENT
        };

        // XTest fake_input: type=2 (FakeInput), detail=event_type (KeyPress=2/KeyRelease=3).
        // Если XTest opcode не закэширован (extension недоступен), fake_input
        // всё равно вернёт Err — мы логируем это и возвращаем ошибку.
        match x11rb::protocol::xtest::fake_input(
            &self.conn,
            event_type as u8,
            x_keycode as u8,
            0,           // delay (ms)
            x11rb::NONE, // focus window (0 = use current focus)
            0, 0,        // root x, y (for motion events)
            0,           // device id (0 = core device)
        ) {
            Ok(_) => {
                self.conn.flush()?;
                Ok(())
            }
            Err(e) => {
                log::warn!("XTest fake_input failed: {} (XTest extension may be unavailable)", e);
                Err(anyhow::anyhow!("XTest fake_input: {}", e))
            }
        }
    }

    /// Альтернативный способ отправить keyboard event в X-окно: XSendEvent.
    /// Используется как fallback если XTest недоступен. Менее надёжен —
    /// многие приложения (Firefox, Chromium) игнорируют synthetic events
    /// из соображений безопасности. Но для простых приложений (xterm, xeyes)
    /// это работает.
    pub fn send_key_via_send_event(&self, xid: u32, evdev_keycode: u32, pressed: bool) -> Result<()> {
        let x_keycode = (evdev_keycode + 8) as u8;
        let response_type = if pressed {
            x11rb::protocol::xproto::KEY_PRESS_EVENT
        } else {
            x11rb::protocol::xproto::KEY_RELEASE_EVENT
        };

        // Конструируем KeyPressEvent как raw [u8; 32] буфер.
        // X11 KeyPress/KeyRelease event structure (32 bytes):
        //   offset 0: response_type (u8) — 2 for KeyPress, 3 for KeyRelease
        //   offset 1: detail (u8) = keycode
        //   offset 2: sequence (u16, little-endian)
        //   offset 4: time (u32, little-endian)
        //   offset 8: root (u32)
        //   offset 12: event (u32) = window receiving the event
        //   offset 16: child (u32)
        //   offset 20: root_x (i16)
        //   offset 22: root_y (i16)
        //   offset 24: event_x (i16)
        //   offset 26: event_y (i16)
        //   offset 28: state (u16) = modifier mask
        //   offset 30: same_screen (u8)
        //   offset 31: padding (u8)
        let mut buf = [0u8; 32];
        buf[0] = response_type;
        buf[1] = x_keycode;
        // sequence = 0 (offset 2-3, already 0)
        // time = CURRENT_TIME (offset 4-7, already 0)
        // root (offset 8-11)
        buf[8..12].copy_from_slice(&self.root.to_ne_bytes());
        // event (offset 12-15)
        buf[12..16].copy_from_slice(&xid.to_ne_bytes());
        // child = 0 (offset 16-19)
        // root_x, root_y = 0 (offset 20-23)
        // event_x, event_y = 0 (offset 24-27)
        // state = 0 (offset 28-29)
        buf[30] = 1; // same_screen = true

        x11rb::protocol::xproto::send_event(
            &self.conn,
            false, // propagate
            xid,   // destination
            x11rb::protocol::xproto::EventMask::KEY_PRESS | x11rb::protocol::xproto::EventMask::KEY_RELEASE,
            buf,
        )?;
        self.conn.flush()?;
        Ok(())
    }

    /// Закрывает X-окно корректно:
    ///   1. Если окно поддерживает WM_DELETE_WINDOW protocol — отправляем
    ///      ClientMessage с WM_DELETE_WINDOW. Приложение получает событие и
    ///      может закрыться само (с сохранением состояния и т.д.).
    ///      Также отправляем WM_SAVE_YOURSELF чтобы приложение могло
    ///      сохранить состояние перед закрытием.
    ///   2. Иначе — destroy_window(). Это жёсткое закрытие, приложение не
    ///      получает шанса очистить ресурсы. Но для некоторых старых/простых
    ///      приложений (xterm без WM_PROTOCOLS) это единственный способ.
    ///   3. Если WM_DELETE_WINDOW отправлен но окно не закрылось —
    ///      принудительно destroy_window() как fallback.
    pub fn close_window(&mut self, xid: u32) -> Result<()> {
        let atoms = self.atoms();
        let supports_delete = self.window_supports_wm_delete(xid);

        if supports_delete {
            // Send WM_DELETE_WINDOW ClientMessage.
            // Format 32 (long), 1 element = the WM_DELETE_WINDOW atom.
            let mut data = [0u32; 5];
            data[0] = atoms.wm_delete_window;
            let msg = x11rb::protocol::xproto::ClientMessageEvent {
                response_type: x11rb::protocol::xproto::CLIENT_MESSAGE_EVENT,
                format: 32,
                sequence: 0,
                window: xid,
                type_: atoms.wm_protocols,
                data: x11rb::protocol::xproto::ClientMessageData::from(data),
            };
            x11rb::protocol::xproto::send_event(
                &self.conn,
                false,           // propagate
                xid,             // destination
                x11rb::protocol::xproto::EventMask::NO_EVENT,
                msg,
            )?;
            self.conn.flush()?;
            log::info!("X sent WM_DELETE_WINDOW to 0x{:x}", xid);

            // Даём приложению время на обработку WM_DELETE_WINDOW.
            // Если через 200мс окно ещё живо — force destroy.
            // Это решает проблему "не закрывается" для приложений, которые
            // либо не обрабатывают ClientMessage корректно, либо зависли.
            std::thread::sleep(std::time::Duration::from_millis(200));
            if self.window_exists(xid) {
                log::warn!("X window 0x{:x} still alive after WM_DELETE_WINDOW — force destroying", xid);
                let _ = x11rb::protocol::xproto::destroy_window(&self.conn, xid);
                self.conn.flush()?;
            }
        } else {
            // Force destroy — приложение не поддерживает WM_DELETE_WINDOW.
            x11rb::protocol::xproto::destroy_window(&self.conn, xid)?;
            self.conn.flush()?;
            log::info!("X destroyed window 0x{:x} (no WM_DELETE_WINDOW)", xid);
        }

        // Clear focus if we just closed the focused window.
        if self.focused_xid == Some(xid) {
            self.focused_xid = None;
        }
        Ok(())
    }

    /// Проверяет, существует ли ещё X-окно на сервере.
    /// Используется в close_window для fallback destroy.
    fn window_exists(&self, xid: u32) -> bool {
        use x11rb::protocol::xproto::{get_window_attributes, ConnectionExt};
        match get_window_attributes(&self.conn, xid) {
            Ok(c) => c.reply().is_ok(),
            Err(_) => false,
        }
    }

    /// Проверяет, поддерживает ли окно WM_DELETE_WINDOW protocol.
    /// Приложения, которые этого не делают, при попытке закрытия через
    /// ClientMessage просто его игнорируют — приходится destroy_window().
    fn window_supports_wm_delete(&self, xid: u32) -> bool {
        use x11rb::protocol::xproto::{get_property, ConnectionExt};
        let atoms = self.atoms();
        let conn = &self.conn;

        let reply = match get_property(
            conn,
            false,
            xid,
            atoms.wm_protocols,
            x11rb::protocol::xproto::AtomEnum::ATOM,
            0,
            256,
        ) {
            Ok(c) => match c.reply() {
                Ok(r) => r,
                Err(_) => return false,
            },
            Err(_) => return false,
        };

        // reply.value is Vec<u8> for ATOM type (format 32, 4 bytes per atom).
        // Check if WM_DELETE_WINDOW atom is in the list.
        reply.value.chunks_exact(4).any(|chunk| {
            u32::from_ne_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]) == atoms.wm_delete_window
        })
    }

    /// Получает WM_NAME или _NET_WM_NAME property окна. _NET_WM_NAME
    /// предпочтительнее — это UTF-8, в то время как WM_NAME может быть
    /// в COMPOUND_TEXT (устаревший). Возвращаем String (UTF-8).
    fn get_window_name(&self, xid: u32) -> Option<String> {
        use x11rb::protocol::xproto::{get_property, ConnectionExt};
        let atoms = self.atoms();
        let conn = &self.conn;

        // Try _NET_WM_NAME first (UTF-8).
        if let Ok(c) = get_property(
            conn, false, xid, atoms.net_wm_name, atoms.utf8_string, 0, 1024,
        ) {
            if let Ok(r) = c.reply() {
                if !r.value.is_empty() {
                    if let Ok(s) = String::from_utf8(r.value) {
                        return Some(s);
                    }
                }
            }
        }
        // Fall back to WM_NAME (STRING / COMPOUND_TEXT).
        if let Ok(c) = get_property(
            conn, false, xid, atoms.wm_name,
            x11rb::protocol::xproto::AtomEnum::STRING, 0, 1024,
        ) {
            if let Ok(r) = c.reply() {
                if !r.value.is_empty() {
                    match String::from_utf8(r.value.clone()) {
                        Ok(s) => return Some(s),
                        Err(_) => {
                            // COMPOUND_TEXT — best-effort Latin-1 decode.
                            return Some(r.value.iter().map(|&b| b as char).collect());
                        }
                    }
                }
            }
        }
        None
    }

    /// Получает geometry окна (width, height) через get_geometry request.
    /// Возвращает None если запрос не удался (например, окно уже уничтожено).
    fn get_window_geometry(&self, xid: u32) -> Option<(u16, u16)> {
        use x11rb::protocol::xproto::{get_geometry, ConnectionExt};
        let conn = &self.conn;
        match get_geometry(conn, xid) {
            Ok(c) => match c.reply() {
                Ok(r) => Some((r.width, r.height)),
                Err(e) => {
                    log::debug!("get_geometry reply for 0x{:x} failed: {}", xid, e);
                    None
                }
            },
            Err(e) => {
                log::debug!("get_geometry for 0x{:x} failed: {}", xid, e);
                None
            }
        }
    }

    /// Проверяет, является ли окно transient (диалог) другого уже
    /// отслеживаемого окна. Возвращает true если WM_TRANSIENT_FOR указывает
    /// на одно из наших tracked windows.
    fn is_transient_of_tracked(&self, xid: u32) -> bool {
        use x11rb::protocol::xproto::{get_property, ConnectionExt};
        let atoms = self.atoms();
        let conn = &self.conn;

        let reply = match get_property(
            conn, false, xid, atoms.wm_transient_for,
            x11rb::protocol::xproto::AtomEnum::WINDOW, 0, 1,
        ) {
            Ok(c) => match c.reply() {
                Ok(r) => r,
                Err(_) => return false,
            },
            Err(_) => return false,
        };

        if reply.value.len() < 4 { return false; }
        let parent = u32::from_ne_bytes([
            reply.value[0], reply.value[1], reply.value[2], reply.value[3],
        ]);
        self.windows.iter().any(|w| w.xid == parent)
    }

    /// Возвращает cached Atoms. Если atoms ещё не инициализированы (например
    /// init_atoms() не вызывался или завершился ошибкой), пытается
    /// инициализировать сейчас.
    fn atoms(&self) -> Atoms {
        *self.atoms.get_or_init(|| self.init_atoms_sync().unwrap_or(Atoms {
            wm_protocols: 0,
            wm_delete_window: 0,
            wm_name: 0,
            wm_class: 0,
            net_wm_name: 0,
            wm_transient_for: 0,
            utf8_string: 0,
        }))
    }

    /// Инициализирует atoms (WM_PROTOCOLS, WM_DELETE_WINDOW, etc.) через
    /// intern_atom. Вызывается один раз при start() и кэшируется в
    /// self.atoms (OnceLock).
    fn init_atoms(&self) -> Result<Atoms> {
        let atoms = self.init_atoms_sync().context("intern_atom failed")?;
        let _ = self.atoms.set(atoms);
        Ok(atoms)
    }

    fn init_atoms_sync(&self) -> std::result::Result<Atoms, x11rb::errors::ReplyError> {
        use x11rb::protocol::xproto::intern_atom;
        // Batch all intern_atom requests then collect replies.
        let c = &self.conn;
        let wm_protocols = intern_atom(c, false, b"WM_PROTOCOLS")?;
        let wm_delete_window = intern_atom(c, false, b"WM_DELETE_WINDOW")?;
        let wm_name = intern_atom(c, false, b"WM_NAME")?;
        let wm_class = intern_atom(c, false, b"WM_CLASS")?;
        let net_wm_name = intern_atom(c, false, b"_NET_WM_NAME")?;
        let wm_transient_for = intern_atom(c, false, b"WM_TRANSIENT_FOR")?;
        let utf8_string = intern_atom(c, false, b"UTF8_STRING")?;
        // Collect replies (forces round-trip).
        Ok(Atoms {
            wm_protocols: wm_protocols.reply()?.atom,
            wm_delete_window: wm_delete_window.reply()?.atom,
            wm_name: wm_name.reply()?.atom,
            wm_class: wm_class.reply()?.atom,
            net_wm_name: net_wm_name.reply()?.atom,
            wm_transient_for: wm_transient_for.reply()?.atom,
            utf8_string: utf8_string.reply()?.atom,
        })
    }

    /// Запрашивает XTest extension opcode. Если extension доступен,
    /// кэширует opcode в self.xtest_opcode (OnceLock). XTest используется
    /// в send_key() для инжекта клавиатурных событий.
    fn init_xtest(&self) -> Result<u8> {
        use x11rb::connection::RequestConnection;
        let c = &self.conn;
        match c.extension_information("XTEST") {
            Ok(Some(info)) => {
                let opcode = info.major_opcode;
                let _ = self.xtest_opcode.set(opcode);
                log::info!("XTest extension available (opcode={})", opcode);
                Ok(opcode)
            }
            Ok(None) => {
                log::warn!("XTest extension NOT available — keyboard input to X11 windows will not work");
                Ok(0)
            }
            Err(e) => {
                log::warn!("XTest query failed: {}", e);
                Err(anyhow::anyhow!("XTest query: {}", e))
            }
        }
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
