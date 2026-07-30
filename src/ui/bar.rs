//! Status bar — полность настраиваемая панель в стиле polybar/waybar.
//!
//! Конфигурация через [bar] секцию в config.toml:
//!   - position: top/bottom
//!   - height: высота в пикселях
//!   - bg/fg/active_bg/active_fg: цвета
//!   - modules: список модулей с типом, позицией (left/center/right),
//!     форматом, цветом, интервалом обновления
//!
//! Типы модулей:
//!   workspaces — список рабочих пространств
//!   clock      — strftime-форматированное время
//!   cpu        — загрузка CPU в %
//!   memory     — использование RAM в %
//!   battery    — заряд батареи в %
//!   network    — IP-адрес активного интерфейса
//!   text       — статичный текст
//!   custom     — вывод shell-команды
//!
//! Модули с refresh_ms > 0 кэшируются — обновляются не чаще указанного
//! интервала. Это критично для custom-модулей (запуск процесса каждый
//! кадр = 60+ fork/exec в секунду = лагает).

use std::collections::HashMap;
use std::process::Command;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use std::fs;

use crate::config::{BarCfg, BarModuleCfg};
use crate::render::canvas::Canvas;
use crate::render::font::Font;
use crate::render::text::TextRenderer;
use crate::ui::theme::{Color, Theme};
use crate::layout::workspaces::Workspaces;

/// Rendered text segment of the bar.
/// Each module produces 1 or more segments (workspaces produces one per ws).
#[derive(Debug, Clone)]
struct BarSegment {
    text: String,
    fg: Color,
    bg: Option<Color>,
}

/// Cached state for a single module.
struct ModuleCache {
    /// Last time the module was refreshed.
    last_refresh: Option<Instant>,
    /// Cached output (for dynamic modules).
    cached_output: Option<String>,
    /// Per-module internal state (e.g. previous CPU jiffies for delta calc).
    state: HashMap<String, String>,
}

impl ModuleCache {
    fn new() -> Self {
        ModuleCache {
            last_refresh: None,
            cached_output: None,
            state: HashMap::new(),
        }
    }
}

pub struct Bar {
    cfg: BarCfg,
    bg: Color,
    fg: Color,
    active_bg: Color,
    active_fg: Color,
    /// Per-module cache, indexed by module name.
    caches: HashMap<String, ModuleCache>,
    /// Separately cached CPU prev jiffies (because we share across refreshes).
    cpu_prev_idle: u64,
    cpu_prev_total: u64,
}

impl Bar {
    pub fn new(cfg: BarCfg, theme: &Theme) -> Self {
        let bg = cfg.bg.as_ref()
            .map(|s| { let (r,g,b) = crate::config::parse_color(s); Color(r,g,b) })
            .unwrap_or(Color(0x05, 0x03, 0x10));
        let fg = cfg.fg.as_ref()
            .map(|s| { let (r,g,b) = crate::config::parse_color(s); Color(r,g,b) })
            .unwrap_or(theme.fg_default);
        let active_bg = cfg.active_bg.as_ref()
            .map(|s| { let (r,g,b) = crate::config::parse_color(s); Color(r,g,b) })
            .unwrap_or(Color(0x20, 0x10, 0x40));
        let active_fg = cfg.active_fg.as_ref()
            .map(|s| { let (r,g,b) = crate::config::parse_color(s); Color(r,g,b) })
            .unwrap_or(theme.accent_magenta);

        Bar {
            cfg,
            bg,
            fg,
            active_bg,
            active_fg,
            caches: HashMap::new(),
            cpu_prev_idle: 0,
            cpu_prev_total: 0,
        }
    }

    /// Replace config (used for live reload).
    pub fn update_cfg(&mut self, cfg: BarCfg, theme: &Theme) {
        let bg = cfg.bg.as_ref()
            .map(|s| { let (r,g,b) = crate::config::parse_color(s); Color(r,g,b) })
            .unwrap_or(Color(0x05, 0x03, 0x10));
        let fg = cfg.fg.as_ref()
            .map(|s| { let (r,g,b) = crate::config::parse_color(s); Color(r,g,b) })
            .unwrap_or(theme.fg_default);
        let active_bg = cfg.active_bg.as_ref()
            .map(|s| { let (r,g,b) = crate::config::parse_color(s); Color(r,g,b) })
            .unwrap_or(Color(0x20, 0x10, 0x40));
        let active_fg = cfg.active_fg.as_ref()
            .map(|s| { let (r,g,b) = crate::config::parse_color(s); Color(r,g,b) })
            .unwrap_or(theme.accent_magenta);
        self.cfg = cfg;
        self.bg = bg;
        self.fg = fg;
        self.active_bg = active_bg;
        self.active_fg = active_fg;
    }

    /// Returns the height of the bar (0 if disabled).
    pub fn height(&self) -> u32 {
        if self.cfg.enabled { self.cfg.height } else { 0 }
    }

    /// Returns the Y coordinate of the top of the bar.
    pub fn y_pos(&self, canvas_h: u32) -> i32 {
        if !self.cfg.enabled { return 0; }
        if self.cfg.position == "top" {
            0
        } else {
            canvas_h as i32 - self.cfg.height as i32
        }
    }

    pub fn enabled(&self) -> bool { self.cfg.enabled }

    /// Renders the bar on the canvas.
    pub fn render(&mut self, canvas: &Canvas, font: &Font, theme: &Theme, workspaces: &Workspaces) {
        if !self.cfg.enabled { return; }
        let h = self.cfg.height;
        let y = self.y_pos(canvas.height);

        // Background.
        canvas.fill_rect(0, y, canvas.width, h, self.bg);

        // Top/bottom border line in accent color (subtle separator from tiles).
        let border_y = if self.cfg.position == "top" { y + h as i32 - 1 } else { y };
        canvas.fill_rect(0, border_y, canvas.width, 1, theme.border_inactive);

        let text = TextRenderer::new(canvas, font);
        let fw = font.width as i32;
        let fh = font.height as i32;
        let pad = 4i32;
        let text_y = y + ((h as i32 - fh) / 2).max(0);

        // Generate segments for each module.
        // Clone the module list to avoid borrow conflict: render_module
        // mutates self.caches (per-module refresh state), but we also need
        // to iterate over self.cfg.modules. Cloning the Vec is cheap.
        let modules: Vec<BarModuleCfg> = self.cfg.modules.clone();
        let mut left_segs: Vec<BarSegment> = Vec::new();
        let mut center_segs: Vec<BarSegment> = Vec::new();
        let mut right_segs: Vec<BarSegment> = Vec::new();

        for module in &modules {
            let segs = self.render_module(module, workspaces);
            let target = match module.position.as_str() {
                "center" => &mut center_segs,
                "right" => &mut right_segs,
                _ => &mut left_segs,
            };
            for s in segs {
                target.push(s);
            }
        }

        // Render left modules.
        let mut x = pad;
        for (i, seg) in left_segs.iter().enumerate() {
            if i > 0 && self.cfg.separators {
                let _ = self.cfg.separator; // not rendered as separate segment
            }
            let w = (seg.text.chars().count() as i32) * fw;
            if let Some(bg) = seg.bg {
                canvas.fill_rect(x, text_y - 1, w as u32 + 4, fh as u32 + 2, bg);
                text.draw_text(x + 2, text_y, &seg.text, seg.fg, Some(bg));
            } else {
                text.draw_text(x, text_y, &seg.text, seg.fg, None);
            }
            x += w + 6;
        }

        // Render right modules (right-aligned).
        let mut rx = canvas.width as i32 - pad;
        for seg in right_segs.iter().rev() {
            let w = (seg.text.chars().count() as i32) * fw;
            rx -= w + 6;
            if let Some(bg) = seg.bg {
                canvas.fill_rect(rx, text_y - 1, w as u32 + 4, fh as u32 + 2, bg);
                text.draw_text(rx + 2, text_y, &seg.text, seg.fg, Some(bg));
            } else {
                text.draw_text(rx, text_y, &seg.text, seg.fg, None);
            }
        }

        // Render center modules (centered).
        let total_w: i32 = center_segs.iter()
            .map(|s| (s.text.chars().count() as i32) * fw + 6)
            .sum();
        let mut cx = ((canvas.width as i32) - total_w) / 2;
        for seg in center_segs.iter() {
            let w = (seg.text.chars().count() as i32) * fw;
            if let Some(bg) = seg.bg {
                canvas.fill_rect(cx, text_y - 1, w as u32 + 4, fh as u32 + 2, bg);
                text.draw_text(cx + 2, text_y, &seg.text, seg.fg, Some(bg));
            } else {
                text.draw_text(cx, text_y, &seg.text, seg.fg, None);
            }
            cx += w + 6;
        }
    }

    /// Generate segments for a single module.
    fn render_module(&mut self, module: &BarModuleCfg, workspaces: &Workspaces) -> Vec<BarSegment> {
        let module_color = module.color.as_ref()
            .map(|s| { let (r,g,b) = crate::config::parse_color(s); Color(r,g,b) })
            .unwrap_or(self.fg);

        match module.type_.as_str() {
            "workspaces" => self.render_workspaces(module, workspaces, module_color),
            "clock" => vec![self.render_clock(module, module_color)],
            "cpu" => vec![self.render_cpu(module, module_color)],
            "memory" => vec![self.render_memory(module, module_color)],
            "battery" => vec![self.render_battery(module, module_color)],
            "network" => vec![self.render_network(module, module_color)],
            "text" => {
                let content = module.cmd.clone().unwrap_or_default();
                vec![BarSegment {
                    text: self.truncate(content, module.max_len),
                    fg: module_color,
                    bg: None,
                }]
            }
            "custom" => vec![self.render_custom(module, module_color)],
            _ => {
                log::warn!("unknown bar module type: '{}' (name='{}')", module.type_, module.name);
                vec![]
            }
        }
    }

    fn render_workspaces(&self, _module: &BarModuleCfg, workspaces: &Workspaces, _default_color: Color) -> Vec<BarSegment> {
        let mut segs = Vec::new();
        for n in 1..=workspaces.max {
            let name = workspaces.names.get(&n).cloned().unwrap_or_else(|| n.to_string());
            let is_current = workspaces.current == n;
            // format substitution: {n} = workspace number, {name} = workspace name
            let label = self.cfg.modules.iter()
                .find(|m| m.type_ == "workspaces")
                .map(|m| m.format.replace("{n}", &(n % 10).to_string()).replace("{name}", &name))
                .unwrap_or_else(|| format!(" {}:{}", n % 10, name));
            let display_n = if n == 10 { 0 } else { n };
            let label = label.replace("{n}", &display_n.to_string()).replace("{name}", &name);
            segs.push(BarSegment {
                text: format!(" {} ", label.trim()),
                fg: if is_current { self.active_fg } else { self.fg },
                bg: if is_current { Some(self.active_bg) } else { None },
            });
        }
        segs
    }

    fn render_clock(&mut self, module: &BarModuleCfg, color: Color) -> BarSegment {
        let cached = self.caches.entry(module.name.clone()).or_insert_with(ModuleCache::new);
        let need_refresh = cached.last_refresh.is_none()
            || module.refresh_ms == 0
            || cached.last_refresh.unwrap().elapsed() > Duration::from_millis(module.refresh_ms);

        if need_refresh {
            let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default();
            let secs = now.as_secs() as i64;
            let text = format_strftime(&module.format, secs);
            cached.cached_output = Some(text);
            cached.last_refresh = Some(Instant::now());
        }

        let text = cached.cached_output.clone().unwrap_or_default();
        BarSegment {
            text: self.truncate(text, module.max_len),
            fg: color,
            bg: None,
        }
    }

    fn render_cpu(&mut self, module: &BarModuleCfg, color: Color) -> BarSegment {
        let need_refresh = {
            let cached = self.caches.entry(module.name.clone()).or_insert_with(ModuleCache::new);
            cached.last_refresh.is_none()
                || module.refresh_ms == 0
                || cached.last_refresh.unwrap().elapsed() > Duration::from_millis(module.refresh_ms.max(500))
        };

        if need_refresh {
            // Read CPU first (mutates self.cpu_prev_*), THEN take the cache entry.
            let percent = self.read_cpu_percent();
            let text = module.format.replace("{percent}", &percent.to_string());
            let cached = self.caches.entry(module.name.clone()).or_insert_with(ModuleCache::new);
            cached.cached_output = Some(text);
            cached.last_refresh = Some(Instant::now());
        }

        let text = self.caches.get(&module.name)
            .and_then(|c| c.cached_output.clone())
            .unwrap_or_default();
        BarSegment {
            text: self.truncate(text, module.max_len),
            fg: color,
            bg: None,
        }
    }

    fn render_memory(&mut self, module: &BarModuleCfg, color: Color) -> BarSegment {
        let cached = self.caches.entry(module.name.clone()).or_insert_with(ModuleCache::new);
        let need_refresh = cached.last_refresh.is_none()
            || module.refresh_ms == 0
            || cached.last_refresh.unwrap().elapsed() > Duration::from_millis(module.refresh_ms.max(500));

        if need_refresh {
            let percent = read_mem_percent();
            let text = module.format.replace("{percent}", &percent.to_string());
            cached.cached_output = Some(text);
            cached.last_refresh = Some(Instant::now());
        }

        let text = cached.cached_output.clone().unwrap_or_default();
        BarSegment {
            text: self.truncate(text, module.max_len),
            fg: color,
            bg: None,
        }
    }

    fn render_battery(&mut self, module: &BarModuleCfg, color: Color) -> BarSegment {
        let cached = self.caches.entry(module.name.clone()).or_insert_with(ModuleCache::new);
        let need_refresh = cached.last_refresh.is_none()
            || module.refresh_ms == 0
            || cached.last_refresh.unwrap().elapsed() > Duration::from_millis(module.refresh_ms.max(2000));

        if need_refresh {
            let percent = read_battery_percent();
            let text = module.format.replace("{percent}", &percent.to_string());
            cached.cached_output = Some(text);
            cached.last_refresh = Some(Instant::now());
        }

        let text = cached.cached_output.clone().unwrap_or_default();
        BarSegment {
            text: self.truncate(text, module.max_len),
            fg: color,
            bg: None,
        }
    }

    fn render_network(&mut self, module: &BarModuleCfg, color: Color) -> BarSegment {
        let cached = self.caches.entry(module.name.clone()).or_insert_with(ModuleCache::new);
        let need_refresh = cached.last_refresh.is_none()
            || module.refresh_ms == 0
            || cached.last_refresh.unwrap().elapsed() > Duration::from_millis(module.refresh_ms.max(2000));

        if need_refresh {
            let (iface, ip) = read_network_info();
            let text = module.format
                .replace("{iface}", &iface)
                .replace("{ip}", &ip);
            cached.cached_output = Some(text);
            cached.last_refresh = Some(Instant::now());
        }

        let text = cached.cached_output.clone().unwrap_or_default();
        BarSegment {
            text: self.truncate(text, module.max_len),
            fg: color,
            bg: None,
        }
    }

    fn render_custom(&mut self, module: &BarModuleCfg, color: Color) -> BarSegment {
        let cached = self.caches.entry(module.name.clone()).or_insert_with(ModuleCache::new);
        let need_refresh = cached.last_refresh.is_none()
            || module.refresh_ms == 0
            || cached.last_refresh.unwrap().elapsed() > Duration::from_millis(module.refresh_ms.max(500));

        if need_refresh {
            let cmd = match module.cmd.as_ref() {
                Some(c) => c.clone(),
                None => {
                    cached.cached_output = Some(String::new());
                    cached.last_refresh = Some(Instant::now());
                    return BarSegment { text: String::new(), fg: color, bg: None };
                }
            };
            let args = module.args.clone().unwrap_or_default();
            let output = Command::new(&cmd)
                .args(&args)
                .output()
                .ok()
                .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
                .unwrap_or_default();
            let text = if module.format == "{}" || module.format.is_empty() {
                output
            } else {
                module.format.replace("{}", &output)
            };
            cached.cached_output = Some(text);
            cached.last_refresh = Some(Instant::now());
        }

        let text = cached.cached_output.clone().unwrap_or_default();
        BarSegment {
            text: self.truncate(text, module.max_len),
            fg: color,
            bg: None,
        }
    }

    fn truncate(&self, s: String, max_len: Option<usize>) -> String {
        match max_len {
            Some(max) if s.chars().count() > max => {
                let mut t: String = s.chars().take(max.saturating_sub(3)).collect();
                t.push_str("...");
                t
            }
            _ => s,
        }
    }

    /// Reads CPU usage percentage from /proc/stat.
    /// Returns delta-based percentage (0-100) since last call.
    fn read_cpu_percent(&mut self) -> u32 {
        let content = match fs::read_to_string("/proc/stat") {
            Ok(c) => c,
            Err(_) => return 0,
        };
        let first_line = match content.lines().next() {
            Some(l) => l,
            None => return 0,
        };
        // Format: "cpu  user nice system idle iowait irq softirq steal ..."
        let fields: Vec<u64> = first_line
            .split_whitespace()
            .skip(1)
            .filter_map(|s| s.parse().ok())
            .collect();
        if fields.len() < 4 { return 0; }
        let idle: u64 = fields.get(3).copied().unwrap_or(0)
            + fields.get(4).copied().unwrap_or(0); // idle + iowait
        let total: u64 = fields.iter().sum();

        let prev_idle = self.cpu_prev_idle;
        let prev_total = self.cpu_prev_total;

        let delta_total = total.saturating_sub(prev_total);
        let delta_idle = idle.saturating_sub(prev_idle);

        self.cpu_prev_idle = idle;
        self.cpu_prev_total = total;

        if delta_total == 0 { return 0; }
        let used = delta_total.saturating_sub(delta_idle);
        ((used * 100) / delta_total) as u32
    }
}

/// Reads memory usage percentage from /proc/meminfo.
/// Returns percentage of used memory (0-100).
fn read_mem_percent() -> u32 {
    let content = match fs::read_to_string("/proc/meminfo") {
        Ok(c) => c,
        Err(_) => return 0,
    };
    let mut total: u64 = 0;
    let mut available: u64 = 0;
    for line in content.lines() {
        if line.starts_with("MemTotal:") {
            total = line.split_whitespace().nth(1).and_then(|s| s.parse().ok()).unwrap_or(0);
        } else if line.starts_with("MemAvailable:") {
            available = line.split_whitespace().nth(1).and_then(|s| s.parse().ok()).unwrap_or(0);
        }
    }
    if total == 0 { return 0; }
    let used = total.saturating_sub(available);
    ((used * 100) / total) as u32
}

/// Reads battery percentage from /sys/class/power_supply/BAT*/capacity.
/// Returns 100 if no battery found (desktop).
fn read_battery_percent() -> u32 {
    let mut max_pct = 100u32;
    if let Ok(entries) = fs::read_dir("/sys/class/power_supply") {
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            if !name_str.starts_with("BAT") { continue; }
            let cap_path = entry.path().join("capacity");
            if let Ok(s) = fs::read_to_string(&cap_path) {
                if let Ok(p) = s.trim().parse::<u32>() {
                    max_pct = p;
                    break;
                }
            }
        }
    }
    max_pct
}

/// Reads network info: name of first non-lo interface with carrier, and its IPv4.
fn read_network_info() -> (String, String) {
    // Try /sys/class/net/ for interface names.
    let mut iface_name = String::new();
    if let Ok(entries) = fs::read_dir("/sys/class/net") {
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            if name_str == "lo" { continue; }
            // Check carrier — skip interfaces that are down.
            let carrier_path = entry.path().join("carrier");
            if let Ok(s) = fs::read_to_string(&carrier_path) {
                if s.trim() == "1" {
                    iface_name = name_str.into_owned();
                    break;
                }
            }
        }
    }
    if iface_name.is_empty() {
        return ("offline".into(), "-".into());
    }

    // Parse IPv4 from /proc/net/fib_trie (simpler than ioctl).
    // Alternative: run `ip -4 addr show <iface>` but that's a fork.
    // For now return iface name only — IP parsing is complex without libc.
    // We'll do a basic check via /proc/net/route for default route iface.
    let ip = get_ipv4_for_iface(&iface_name).unwrap_or_else(|| "-".into());
    (iface_name, ip)
}

/// Reads IPv4 address for a given interface using getifaddrs (libc).
fn get_ipv4_for_iface(iface: &str) -> Option<String> {
    use std::ffi::CStr;
    use std::os::raw::c_int;

    #[repr(C)]
    struct IfAddrsC {
        ifa_next: *mut IfAddrsC,
        ifa_name: *mut libc::c_char,
        ifa_flags: u32,
        ifa_addr: *mut libc::sockaddr,
        _ifa_netmask: *mut libc::sockaddr,
        _ifa_ifu: *mut libc::sockaddr,
        _ifa_data: *mut libc::c_void,
    }

    extern "C" {
        fn getifaddrs(ifap: *mut *mut IfAddrsC) -> c_int;
        fn freeifaddrs(ifa: *mut IfAddrsC);
    }

    let mut ifap: *mut IfAddrsC = std::ptr::null_mut();
    unsafe {
        if getifaddrs(&mut ifap) != 0 {
            return None;
        }
        let mut cur = ifap;
        let mut result: Option<String> = None;
        while !cur.is_null() {
            let ifa = &*cur;
            if !ifa.ifa_name.is_null() && !ifa.ifa_addr.is_null() {
                let name_cstr = CStr::from_ptr(ifa.ifa_name);
                if let Ok(name) = name_cstr.to_str() {
                    if name == iface {
                        let sa = &*ifa.ifa_addr;
                        if sa.sa_family as i32 == libc::AF_INET {
                            let sin: *const libc::sockaddr_in = ifa.ifa_addr as *const libc::sockaddr_in;
                            let addr = (*sin).sin_addr.s_addr;
                            let bytes = addr.to_ne_bytes();
                            result = Some(format!("{}.{}.{}.{}",
                                bytes[0], bytes[1], bytes[2], bytes[3]));
                            break;
                        }
                    }
                }
            }
            cur = ifa.ifa_next;
        }
        freeifaddrs(ifap);
        result
    }
}

/// Minimal strftime-like formatter. Supports the most common format specifiers:
///   %Y (year 4-digit), %m (month 01-12), %d (day 01-31),
///   %H (hour 00-23), %M (minute 00-59), %S (second 00-59),
///   %A (weekday name), %a (abbrev weekday), %B (month name), %b (abbrev month).
///   %% (literal %).
fn format_strftime(format: &str, epoch_secs: i64) -> String {
    // Convert epoch to UTC components (no timezone handling — UTC only,
    // because libc strftime would require a tm struct and would be C-locale).
    // For local time we'd need to read TZ — leave it as UTC for now,
    // most users configure their WM clock in UTC anyway. For local time
    // they can use a custom module with `date +%H:%M`.
    let secs_per_day = 86400i64;
    let days_since_epoch = epoch_secs.div_euclid(secs_per_day);
    let secs_in_day = epoch_secs.rem_euclid(secs_per_day);

    let hour = (secs_in_day / 3600) as u32;
    let minute = ((secs_in_day % 3600) / 60) as u32;
    let second = (secs_in_day % 60) as u32;

    // Days to year/month/day (algorithm from Howard Hinnant's date library).
    let z = days_since_epoch + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = (z - era * 146097) as u64; // [0, 146096]
    let yoe = (doe - doe/1460 + doe/36524 - doe/146096) / 365; // [0, 399]
    let y = yoe as i64 + era * 400;
    let doy = doe - (365*yoe + yoe/4 - yoe/100); // [0, 365]
    let mp = (5*doy + 2)/153; // [0, 11]
    let d = doy - (153*mp+2)/5 + 1; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 }; // [1, 12]
    let year = if m <= 2 { y + 1 } else { y };

    let weekday_idx = ((days_since_epoch % 7) + 4) % 7; // 0=Sunday, 1970-01-01 = Thursday (idx 4)
    let weekday_idx = if weekday_idx < 0 { weekday_idx + 7 } else { weekday_idx };

    const WEEKDAYS: [&str; 7] = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];
    const WEEKDAYS_ABBR: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
    const MONTHS: [&str; 12] = ["January", "February", "March", "April", "May", "June",
                                "July", "August", "September", "October", "November", "December"];
    const MONTHS_ABBR: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun",
                                     "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

    let weekday = WEEKDAYS[weekday_idx as usize];
    let weekday_abbr = WEEKDAYS_ABBR[weekday_idx as usize];
    let month_name = MONTHS[(m as usize).saturating_sub(1).min(11)];
    let month_abbr = MONTHS_ABBR[(m as usize).saturating_sub(1).min(11)];

    let mut out = String::with_capacity(format.len() * 2);
    let mut chars = format.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '%' {
            match chars.next() {
                Some('Y') => out.push_str(&year.to_string()),
                Some('m') => out.push_str(&format!("{:02}", m)),
                Some('d') => out.push_str(&format!("{:02}", d)),
                Some('H') => out.push_str(&format!("{:02}", hour)),
                Some('M') => out.push_str(&format!("{:02}", minute)),
                Some('S') => out.push_str(&format!("{:02}", second)),
                Some('A') => out.push_str(weekday),
                Some('a') => out.push_str(weekday_abbr),
                Some('B') => out.push_str(month_name),
                Some('b') => out.push_str(month_abbr),
                Some('%') => out.push('%'),
                Some(other) => { out.push('%'); out.push(other); }
                None => out.push('%'),
            }
        } else {
            out.push(c);
        }
    }
    out
}
