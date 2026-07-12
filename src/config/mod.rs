//! Полноценный конфиг тайлового WM в TOML формате.
//!
//! Поддерживает:
//!   - модификаторы + клавиши → действия
//!   - workspaces 1-9
//!   - перемещение окон по сетке и между workspaces
//!   - настройки темы (MCD палитра)
//!   - настройки launcher
//!   - маппинг геймпада
//!   - настройки звука/портала

use serde::Deserialize;
use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub general: General,
    #[serde(default)]
    pub theme: ThemeCfg,
    #[serde(default)]
    pub keybindings: Vec<Binding>,
    #[serde(default)]
    pub workspaces: Vec<WorkspaceCfg>,
    #[serde(default)]
    pub launcher: LauncherCfg,
    #[serde(default)]
    pub audio: AudioCfg,
    #[serde(default)]
    pub portal: PortalCfg,
    #[serde(default)]
    pub gamepad: GamepadCfg,
    #[serde(default)]
    pub x11: X11Cfg,
}

#[derive(Debug, Clone, Deserialize)]
pub struct General {
    pub shell: String,
    pub font: String,
    pub font_size: u32,
    pub gap: i32,
    pub border: i32,
    pub outer_padding: i32,
    pub status_bar_height: u32,
    pub framerate: u32,
    /// Случайные глитч-эффекты для MCD-стиля (мигание рамок, RGB-сдвиг).
    pub glitch_intensity: f32, // 0.0..1.0
}

impl Default for General {
    fn default() -> Self {
        General {
            shell: "zsh".to_string(),
            font: "Lat2-Terminus16".to_string(),
            font_size: 16,
            gap: 4,
            border: 1,
            outer_padding: 4,
            status_bar_height: 24,
            framerate: 60,
            glitch_intensity: 0.15,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct ThemeCfg {
    pub bg: String,
    pub tile_bg_inactive: String,
    pub tile_bg_active: String,
    pub border_inactive: String,
    pub border_active: String,
    pub border_x11: String,
    pub fg_default: String,
    pub fg_dim: String,
    pub accent_magenta: String,
    pub accent_cyan: String,
    pub popup_bg: String,
    pub popup_border: String,
    pub error: String,
}

impl Default for ThemeCfg {
    fn default() -> Self {
        ThemeCfg {
            bg: "#0A0716".into(),
            tile_bg_inactive: "#120E24".into(),
            tile_bg_active: "#0F0A1E".into(),
            border_inactive: "#3A2D5C".into(),
            border_active: "#FF2E97".into(),
            border_x11: "#00F0FF".into(),
            fg_default: "#E6E1F0".into(),
            fg_dim: "#7A6F96".into(),
            accent_magenta: "#FF2E97".into(),
            accent_cyan: "#00F0FF".into(),
            popup_bg: "#140B2E".into(),
            popup_border: "#FF2E97".into(),
            error: "#FF4D4D".into(),
        }
    }
}

/// Описание клавишного биндинга.
/// `key` — клавиша (буква/цифра/F1/Return/Space/Tab/Left/Right/...).
/// `mods` — список модификаторов: Super, Ctrl, Alt, Shift.
/// `action` — команда для WM (см. Action).
#[derive(Debug, Clone, Deserialize)]
pub struct Binding {
    pub key: String,
    pub mods: Vec<String>,
    pub action: Action,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Action {
    /// Запустить команду в новом терминальном тайле.
    Spawn { cmd: String, args: Vec<String> },
    /// Открыть X11 приложение в новой плитке (через launcher).
    SpawnX11 { cmd: String, args: Vec<String> },
    /// Открыть launcher (rofi-подобный).
    Launcher,
    /// Split активного тайла горизонтально.
    SplitHorizontal,
    /// Split активного тайла вертикально.
    SplitVertical,
    /// Фокус в направлении.
    Focus { dir: Direction },
    /// Переместить активное окно в направлении.
    Move { dir: Direction },
    /// Переключиться на workspace N (1..9).
    Workspace { n: u8 },
    /// Переместить активный тайл на workspace N.
    MoveToWorkspace { n: u8 },
    /// Закрыть активный тайл.
    Close,
    /// Fullscreen toggle.
    Fullscreen,
    /// Resize mode toggle.
    ResizeMode,
    /// Cycle focus.
    CycleFocus,
    /// Resize активного split'а в направлении.
    Resize { dir: Direction, delta: f32 },
    /// Выход.
    Quit,
    /// Запустить терминал с shell.
    Terminal,
    /// Вперёд/назад в tab-стеке (если tile часть стека).
    TabNext,
    TabPrev,
    /// Свап активного окна с соседом в направлении.
    Swap { dir: Direction },
    /// Переключить layout активного контейнера (tile/stack/tabbed).
    ToggleLayout,
    /// Перезагрузить конфиг.
    Reload,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Copy)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    Left,
    Right,
    Up,
    Down,
}

#[derive(Debug, Clone, Deserialize)]
pub struct WorkspaceCfg {
    pub n: u8,
    pub name: String,
    /// Команда запускаемая при первом входе на workspace.
    #[serde(default)]
    pub on_init: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LauncherCfg {
    /// Список директорий с .desktop файлами.
    #[serde(default)]
    pub desktop_paths: Vec<String>,
    /// Шрифт launcher popup.
    #[serde(default = "default_launcher_rows")]
    pub max_rows: u32,
    /// Дополнительные команды (имя → cmd).
    #[serde(default)]
    pub custom_entries: HashMap<String, String>,
    /// Запускать X11 приложения на этом дисплее.
    #[serde(default = "default_x11_display")]
    pub x11_display: String,
}

fn default_launcher_rows() -> u32 { 12 }
fn default_x11_display() -> String { ":1".into() }

impl Default for LauncherCfg {
    fn default() -> Self {
        LauncherCfg {
            desktop_paths: vec![
                "/usr/share/applications".into(),
                "/usr/local/share/applications".into(),
                "~/.local/share/applications".into(),
            ],
            max_rows: 12,
            custom_entries: HashMap::new(),
            x11_display: ":1".into(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct AudioCfg {
    /// Запустить pipewire-pulse для совместимости с PulseAudio приложениями.
    #[serde(default = "default_true")]
    pub start_pipewire_pulse: bool,
    /// Запустить wireplumber (session manager).
    #[serde(default = "default_true")]
    pub start_wireplumber: bool,
    /// Громкость по умолчанию (0..100).
    #[serde(default = "default_volume")]
    pub default_volume: u32,
}

fn default_true() -> bool { true }
fn default_volume() -> u32 { 70 }

impl Default for AudioCfg {
    fn default() -> Self {
        AudioCfg {
            start_pipewire_pulse: true,
            start_wireplumber: true,
            default_volume: 70,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct PortalCfg {
    /// Запустить xdg-desktop-portal backend.
    #[serde(default = "default_true")]
    pub start_portal: bool,
    /// DBus service name нашего портала.
    #[serde(default = "default_portal_name")]
    pub service_name: String,
    /// Путь для записи object path.
    #[serde(default = "default_portal_path")]
    pub object_path: String,
}

fn default_portal_name() -> String { "org.freedesktop.impl.portal.desktop.SuperHot".into() }
fn default_portal_path() -> String { "/org/freedesktop/portal/desktop".into() }

impl Default for PortalCfg {
    fn default() -> Self {
        PortalCfg {
            start_portal: true,
            service_name: "org.freedesktop.impl.portal.desktop.SuperHot".into(),
            object_path: "/org/freedesktop/portal/desktop".into(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct GamepadCfg {
    /// Включить поддержку геймпадов через SDL2.
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Передавать события геймпада напрямую в Steam (через evdev).
    #[serde(default = "default_true")]
    pub steam_passthrough: bool,
    /// Маппинг кнопок → клавиши (для использования вне Steam).
    #[serde(default)]
    pub keymap: HashMap<String, String>,
    /// Чувствительность стиков (1..100).
    #[serde(default = "default_stick_sens")]
    pub stick_sensitivity: u32,
}

fn default_stick_sens() -> u32 { 50 }

impl Default for GamepadCfg {
    fn default() -> Self {
        let mut keymap = HashMap::new();
        keymap.insert("a".into(), "Return".into());
        keymap.insert("b".into(), "Escape".into());
        keymap.insert("x".into(), "space".into());
        keymap.insert("y".into(), "Tab".into());
        keymap.insert("dpad_up".into(), "k".into());
        keymap.insert("dpad_down".into(), "j".into());
        keymap.insert("dpad_left".into(), "h".into());
        keymap.insert("dpad_right".into(), "l".into());
        keymap.insert("start".into(), "Super".into());
        keymap.insert("back".into(), "Super".into());
        GamepadCfg {
            enabled: true,
            steam_passthrough: true,
            keymap,
            stick_sensitivity: 50,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct X11Cfg {
    /// Использовать DRI3 + DMA-BUF для GPU-ускорения.
    #[serde(default = "default_true")]
    pub dri3: bool,
    /// Xephyr display.
    #[serde(default = "default_x11_display")]
    pub display: String,
    /// Xephyr screen size.
    #[serde(default = "default_x11_size")]
    pub screen_size: (u16, u16),
    /// XTest extension для ввода в X11 окна.
    #[serde(default = "default_true")]
    pub xtest_input: bool,
    /// Hardware cursor через DRM cursor plane.
    #[serde(default = "default_true")]
    pub hardware_cursor: bool,
}

fn default_x11_size() -> (u16, u16) { (1920, 1080) }

impl Default for X11Cfg {
    fn default() -> Self {
        X11Cfg {
            dri3: true,
            display: ":1".into(),
            screen_size: (1920, 1080),
            xtest_input: true,
            hardware_cursor: true,
        }
    }
}

impl Config {
    /// Загружает конфиг из стандартных местоположений.
    pub fn load() -> Self {
        let candidates = [
            "/etc/superhot-tty/config.toml",
            "~/.config/superhot-tty/config.toml",
        ];
        for path in candidates {
            let p = shellexpand::tilde(path).to_string();
            if let Ok(s) = std::fs::read_to_string(&p) {
                match toml::from_str(&s) {
                    Ok(c) => {
                        log::info!("loaded config from {}", p);
                        return c;
                    }
                    Err(e) => log::warn!("config parse error in {}: {}", p, e),
                }
            }
        }
        log::warn!("no config.toml found, using defaults");
        Config::default()
    }

    pub fn default_config_toml() -> &'static str {
        include_str!("../../config/default.toml")
    }
}

impl Default for Config {
    fn default() -> Self {
        toml::from_str(Self::default_config_toml())
            .unwrap_or_else(|e| { log::error!("default config parse: {}", e); Config {
                general: General::default(),
                theme: ThemeCfg::default(),
                keybindings: default_bindings(),
                workspaces: default_workspaces(),
                launcher: LauncherCfg::default(),
                audio: AudioCfg::default(),
                portal: PortalCfg::default(),
                gamepad: GamepadCfg::default(),
                x11: X11Cfg::default(),
            }})
    }
}

fn default_bindings() -> Vec<Binding> {
    vec![]
}

fn default_workspaces() -> Vec<WorkspaceCfg> {
    vec![]
}

// minimal shellexpand replacement
mod shellexpand {
    pub fn tilde(s: &str) -> std::borrow::Cow<'_, str> {
        if let Some(rest) = s.strip_prefix("~/") {
            if let Ok(home) = std::env::var("HOME") {
                return std::borrow::Cow::Owned(format!("{}/{}", home, rest));
            }
        }
        std::borrow::Cow::Borrowed(s)
    }
}

/// Парсит строку вида "Super+D" → (mods, key).
pub fn parse_keycombo(s: &str) -> (Vec<String>, String) {
    let parts: Vec<&str> = s.split('+').collect();
    if parts.is_empty() { return (vec![], s.to_string()); }
    let key = parts.last().unwrap().to_string();
    let mods = parts[..parts.len()-1].iter().map(|s| s.to_string()).collect();
    (mods, key)
}

/// Конвертирует hex "#RRGGBB" в (r,g,b).
pub fn parse_color(s: &str) -> (u8, u8, u8) {
    let s = s.trim_start_matches('#');
    if s.len() != 6 { return (0, 0, 0); }
    let r = u8::from_str_radix(&s[0..2], 16).unwrap_or(0);
    let g = u8::from_str_radix(&s[2..4], 16).unwrap_or(0);
    let b = u8::from_str_radix(&s[4..6], 16).unwrap_or(0);
    (r, g, b)
}

#[allow(dead_code)]
fn _unused(_p: PathBuf) {}
