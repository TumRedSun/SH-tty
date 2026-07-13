//! Live-reload watcher для config.toml.
//!
//! Архитектура:
//!   1. Запускаем фоновый поток с inotify на директории, содержащей конфиг.
//!   2. При IN_MODIFY / IN_CLOSE_WRITE / IN_MOVED_TO на config.toml —
//!      сигналим через mpsc канал (после debounce).
//!   3. Главный цикл WM каждое нажатие FPS-тика опрашивает канал (try_recv).
//!      Если пришло событие — вызывает Config::reload().
//!   4. После reload WM пересобирает theme, keybindings, window_rules и т.д.
//!
//! Используем libc::inotify_* напрямую (через nix::unistd не обязательно —
//! всё что нужно есть в libc).

use std::collections::HashMap;
use std::os::unix::io::RawFd;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::{Duration, Instant};

/// Событие от watcher'а: путь к изменённому файлу.
pub struct ConfigChanged;

pub struct ConfigWatcher {
    pub rx: mpsc::Receiver<ConfigChanged>,
    pub last_notify: Instant,
    pub debounce: Duration,
    pub stop_fd: RawFd,
}

impl ConfigWatcher {
    /// Запускает watcher на указанном пути к config.toml.
    /// Возвращает ConfigWatcher, у которого можно опрашивать `rx.try_recv()`.
    pub fn start(config_path: &Path, debounce_ms: u64) -> std::io::Result<Self> {
        let (tx, rx) = mpsc::channel::<ConfigChanged>();
        let debounce = Duration::from_millis(debounce_ms);
        let parent = config_path.parent().unwrap_or_else(|| Path::new("."));
        let file_name = config_path.file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "config.toml".to_string());
        let parent_str = parent.to_string_lossy().to_string();

        let fd = unsafe { libc::inotify_init1(libc::IN_NONBLOCK | libc::IN_CLOEXEC) };
        if fd < 0 {
            return Err(std::io::Error::last_os_error());
        }

        let mask = libc::IN_MODIFY | libc::IN_CLOSE_WRITE | libc::IN_MOVED_TO | libc::IN_CREATE;
        let parent_c = std::ffi::CString::new(parent_str.as_str())
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))?;
        let wd = unsafe { libc::inotify_add_watch(fd, parent_c.as_ptr(), mask) };
        if wd < 0 {
            let e = std::io::Error::last_os_error();
            unsafe { libc::close(fd); }
            return Err(e);
        }

        let file_name_owned = file_name.clone();
        std::thread::Builder::new()
            .name("config-watcher".into())
            .spawn(move || {
                log::info!("config watcher started on {}/{}", parent_str, file_name_owned);
                let mut buf = vec![0u8; 4096];
                let mut last_signal: Option<Instant> = None;
                loop {
                    // poll with 100ms timeout so we can detect fd close.
                    let mut fds = [libc::pollfd {
                        fd,
                        events: libc::POLLIN,
                        revents: 0,
                    }];
                    let r = unsafe { libc::poll(fds.as_mut_ptr(), 1, 100) };
                    if r < 0 {
                        let e = std::io::Error::last_os_error();
                        if e.kind() == std::io::ErrorKind::Interrupted { continue; }
                        log::warn!("config watcher poll error: {}", e);
                        std::thread::sleep(Duration::from_millis(100));
                        continue;
                    }
                    if r == 0 { continue; }
                    if fds[0].revents & libc::POLLIN == 0 {
                        if fds[0].revents & (libc::POLLERR | libc::POLLHUP) != 0 {
                            log::warn!("config watcher fd closed");
                            break;
                        }
                        continue;
                    }
                    let n = unsafe { libc::read(fd, buf.as_mut_ptr() as *mut _, buf.len()) };
                    if n <= 0 { continue; }
                    let n = n as usize;
                    let mut off = 0;
                    let mut matched = false;
                    while off + std::mem::size_of::<libc::inotify_event>() <= n {
                        let ev: &libc::inotify_event = unsafe {
                            &*(buf.as_ptr().add(off) as *const libc::inotify_event)
                        };
                        let name_len = ev.len as usize;
                        let name_str = if name_len > 0 {
                            let ptr = unsafe { buf.as_ptr().add(off + std::mem::size_of::<libc::inotify_event>()) };
                            let bytes = unsafe { std::slice::from_raw_parts(ptr, name_len) };
                            let term = bytes.iter().position(|&b| b == 0).unwrap_or(name_len);
                            String::from_utf8_lossy(&bytes[..term]).to_string()
                        } else {
                            String::new()
                        };
                        if name_str == file_name_owned {
                            matched = true;
                        }
                        off += std::mem::size_of::<libc::inotify_event>() + name_len;
                    }
                    if matched {
                        let now = Instant::now();
                        let should_fire = match last_signal {
                            Some(t) => now.duration_since(t) >= debounce,
                            None => true,
                        };
                        if should_fire {
                            last_signal = Some(now);
                            let _ = tx.send(ConfigChanged);
                        }
                    }
                }
                log::info!("config watcher thread exit");
            })?;

        Ok(ConfigWatcher {
            rx,
            last_notify: Instant::now() - debounce,
            debounce,
            stop_fd: fd,
        })
    }

    /// Неблокирующе проверяет, было ли событие изменения конфига.
    pub fn poll(&mut self) -> bool {
        let mut got = false;
        while let Ok(_) = self.rx.try_recv() {
            got = true;
        }
        if got {
            // Дополнительный программный debounce на стороне получателя.
            let now = Instant::now();
            if now.duration_since(self.last_notify) < self.debounce {
                return false;
            }
            self.last_notify = now;
            return true;
        }
        false
    }
}

impl Drop for ConfigWatcher {
    fn drop(&mut self) {
        if self.stop_fd >= 0 {
            unsafe { libc::close(self.stop_fd); }
        }
    }
}

/// Определяет, какие поля конфига изменились между двумя версиями.
/// Используется для логирования что именно нужно применить.
#[derive(Debug, Default, Clone)]
pub struct ConfigDiff {
    pub theme_changed: bool,
    pub keybindings_changed: bool,
    pub window_rules_changed: bool,
    pub animations_changed: bool,
    pub ipc_changed: bool,
    pub live_reload_changed: bool,
    pub general_changed: bool,
    pub x11_changed: bool,
    pub monitors_changed: bool,
}

impl ConfigDiff {
    pub fn from_configs(old: &crate::config::Config, new: &crate::config::Config) -> Self {
        let mut d = ConfigDiff::default();
        d.theme_changed = !theme_eq(&old.theme, &new.theme);
        d.keybindings_changed = old.keybindings != new.keybindings;
        d.window_rules_changed = old.window_rules.len() != new.window_rules.len()
            || old.window_rules.iter().zip(new.window_rules.iter())
                .any(|(a, b)| !window_rule_eq(a, b));
        d.animations_changed = !animations_eq(&old.animations, &new.animations);
        d.ipc_changed = !ipc_eq(&old.ipc, &new.ipc);
        d.live_reload_changed = !live_reload_eq(&old.live_reload, &new.live_reload);
        d.general_changed = !general_eq(&old.general, &new.general);
        d.x11_changed = !x11_eq(&old.x11, &new.x11);
        d.monitors_changed = old.monitors.len() != new.monitors.len()
            || old.monitors.iter().zip(new.monitors.iter())
                .any(|(a, b)| !monitor_eq(a, b));
        d
    }

    pub fn any(&self) -> bool {
        self.theme_changed || self.keybindings_changed || self.window_rules_changed
            || self.animations_changed || self.ipc_changed || self.live_reload_changed
            || self.general_changed || self.x11_changed || self.monitors_changed
    }
}

fn theme_eq(a: &crate::config::ThemeCfg, b: &crate::config::ThemeCfg) -> bool {
    a.bg == b.bg && a.tile_bg_inactive == b.tile_bg_inactive
        && a.tile_bg_active == b.tile_bg_active
        && a.border_inactive == b.border_inactive
        && a.border_active == b.border_active && a.border_x11 == b.border_x11
        && a.fg_default == b.fg_default && a.fg_dim == b.fg_dim
        && a.accent_magenta == b.accent_magenta && a.accent_cyan == b.accent_cyan
        && a.popup_bg == b.popup_bg && a.popup_border == b.popup_border
        && a.error == b.error
}

fn animations_eq(a: &crate::config::AnimationsCfg, b: &crate::config::AnimationsCfg) -> bool {
    a.workspace_transition == b.workspace_transition
        && a.new_window == b.new_window
        && a.random_glitch == b.random_glitch
        && a.ws_transition_ms == b.ws_transition_ms
        && a.ws_manifest_ms == b.ws_manifest_ms
        && a.ws_reveal_ms == b.ws_reveal_ms
        && a.new_window_fill_ms == b.new_window_fill_ms
        && a.new_window_reveal_ms == b.new_window_reveal_ms
        && a.random_glitch_ms == b.random_glitch_ms
        && a.random_glitch_every_frames == b.random_glitch_every_frames
        && a.chars_per_sec == b.chars_per_sec
        && a.random_chars_per_sec == b.random_chars_per_sec
        && a.glitch_use_alpha == b.glitch_use_alpha
        && a.glitch_use_blocks == b.glitch_use_blocks
        && a.glitch_use_digits == b.glitch_use_digits
        && a.glitch_color == b.glitch_color
}

fn ipc_eq(a: &crate::config::IpcCfg, b: &crate::config::IpcCfg) -> bool {
    a.enabled == b.enabled && a.socket_path == b.socket_path && a.socket_mode == b.socket_mode
}

fn live_reload_eq(a: &crate::config::LiveReloadCfg, b: &crate::config::LiveReloadCfg) -> bool {
    a.enabled == b.enabled && a.debounce_ms == b.debounce_ms
}

fn general_eq(a: &crate::config::General, b: &crate::config::General) -> bool {
    a.shell == b.shell && a.font == b.font && a.font_size == b.font_size
        && a.gap == b.gap && a.border == b.border
        && a.outer_padding == b.outer_padding
        && a.status_bar_height == b.status_bar_height
        && a.framerate == b.framerate
        && a.glitch_intensity == b.glitch_intensity
        && a.workspace_count == b.workspace_count
}

fn x11_eq(a: &crate::config::X11Cfg, b: &crate::config::X11Cfg) -> bool {
    a.dri3 == b.dri3 && a.display == b.display && a.screen_size == b.screen_size
        && a.xtest_input == b.xtest_input && a.hardware_cursor == b.hardware_cursor
        && a.auto_place_windows == b.auto_place_windows
        && a.overlay_planes == b.overlay_planes
}

fn monitor_eq(a: &crate::config::MonitorCfg, b: &crate::config::MonitorCfg) -> bool {
    a.connector == b.connector && a.workspaces == b.workspaces
        && a.resolution == b.resolution && a.refresh_rate == b.refresh_rate
        && a.position == b.position && a.enabled == b.enabled
}

fn window_rule_eq(a: &crate::config::WindowRule, b: &crate::config::WindowRule) -> bool {
    a.match_class == b.match_class && a.match_title == b.match_title
        && a.match_app_id == b.match_app_id && a.regex == b.regex
        && a.workspace == b.workspace && a.monitor == b.monitor
        && a.size == b.size && a.position == b.position
        && a.focus == b.focus && a.fullscreen == b.fullscreen
        && a.skip_auto_place == b.skip_auto_place
}

/// Хелпер: сохраняет в HashMap какие поля были изменены для отладки.
pub fn diff_summary(d: &ConfigDiff) -> HashMap<String, bool> {
    let mut m = HashMap::new();
    m.insert("theme".into(), d.theme_changed);
    m.insert("keybindings".into(), d.keybindings_changed);
    m.insert("window_rules".into(), d.window_rules_changed);
    m.insert("animations".into(), d.animations_changed);
    m.insert("ipc".into(), d.ipc_changed);
    m.insert("live_reload".into(), d.live_reload_changed);
    m.insert("general".into(), d.general_changed);
    m.insert("x11".into(), d.x11_changed);
    m.insert("monitors".into(), d.monitors_changed);
    m
}
