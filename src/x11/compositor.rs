//! X11 compositor-клиент.
//!
//! Архитектура:
//!   1. Запускаем Xephyr в фоне, на отдельном дисплее :1.
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
use x11rb::protocol::xproto::{self, *};
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
    pub title: String,
}

pub struct X11Compositor {
    pub conn: RustConnection,
    pub root: u32,
    pub windows: Vec<TrackedWindow>,
    pub xephyr: Option<Child>,
    pub display: String,
    pub tile_bindings: std::collections::HashMap<u64, XWindowId>,
}

impl X11Compositor {
    pub fn start(display_num: u32, screen_size: (u16, u16)) -> Result<Self> {
        let display = format!(":{}", display_num);
        let xephyr = Command::new("Xephyr")
            .arg(&display)
            .arg("-screen")
            .arg(format!("{}x{}", screen_size.0, screen_size.1))
            .arg("-ac")
            .arg("-reset")
            .arg("-terminate")
            .arg("-nolisten")
            .arg("tcp")
            .spawn()
            .context("failed to launch Xephyr — install xorg-server-xephyr")?;

        // Ждём подключения.
        std::thread::sleep(std::time::Duration::from_millis(500));

        let (conn, _) = x11rb::connect(Some(&display))
            .context("connecting to Xephyr")?;

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
            xephyr: Some(xephyr),
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
        // Damage handle для упрощения не сохраняем (используем dirty flag + poll events).
        let _ = damage::create(&self.conn, xid, 0, damage::ReportLevel::NON_EMPTY);
        let backing = vec![0; (w as usize) * (h as usize)];
        self.windows.push(TrackedWindow {
            xid,
            damage: 0,
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
