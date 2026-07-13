//! Полноценный конфиг тайлового WM в TOML формате (v0.3).
//!
//! Пути поиска конфига (в порядке приоритета):
//!   1. $XDG_CONFIG_HOME/SH-tty/config.toml  (обычно ~/.config/SH-tty/config.toml)
//!   2. ~/.config/SH-tty/config.toml
//!   3. /etc/SH-tty/config.toml              (system-wide default)
//!
//! Никаких захардкоженных биндингов — все в `[[keybindings]]`.
//! Никаких захардкоженных настроек — все имеют defaults.

use serde::Deserialize;
use std::collections::HashMap;
use std::path::PathBuf;

pub mod window_rules;
pub mod watcher;

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub general: General,
    #[serde(default)]
    pub theme: ThemeCfg,
    #[serde(default)]
    pub login: LoginCfg,
    #[serde(default)]
    pub keybindings: Vec<Binding>,
    #[serde(default)]
    pub workspaces: Vec<WorkspaceCfg>,
    #[serde(default)]
    pub monitors: Vec<MonitorCfg>,
    #[serde(default)]
    pub window_rules: Vec<WindowRule>,
    #[serde(default)]
    pub autostart: Vec<AutostartEntry>,
    #[serde(default)]
    pub launcher: LauncherCfg,
    #[serde(default)]
    pub popups: PopupsCfg,
    #[serde(default)]
    pub audio: AudioCfg,
    #[serde(default)]
    pub portal: PortalCfg,
    #[serde(default)]
    pub gamepad: GamepadCfg,
    #[serde(default)]
    pub x11: X11Cfg,
    /// Live-reload конфигурации (inotify watcher на config.toml).
    #[serde(default)]
    pub live_reload: LiveReloadCfg,
    /// Анимации перехода между workspaces и появления новых окон.
    #[serde(default)]
    pub animations: AnimationsCfg,
    /// IPC сокет (как i3-msg).
    #[serde(default)]
    pub ipc: IpcCfg,
    /// Внутренний флаг: используется для live reload — сохраняет пути поиска конфига.
    #[serde(skip)]
    pub _config_path: Option<std::path::PathBuf>,
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
    /// Случайные глитч-эффекты для MCD-стиля (0.0..1.0).
    pub glitch_intensity: f32,
    /// Количество workspaces (по умолчанию 10 — 1..9 + 0=10).
    pub workspace_count: u8,
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
            workspace_count: 10,
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

/// Конфигурация login screen.
#[derive(Debug, Clone, Deserialize)]
pub struct LoginCfg {
    /// Текст по центру экрана (как в SHMCD). Например "MORE", "БОЛЬШЕ" или свой текст.
    pub title: String,
    /// Подзаголовок под главным текстом.
    pub subtitle: String,
    /// Язык — определяет дефолтные строки если title/subtitle не заданы.
    /// "ru" → "БОЛЬШЕ" / "СУПЕРХОТ", "en" → "MORE" / "SUPERHOT".
    pub language: String,
    /// Шрифт для большого заголовка (если отличается от general.font).
    pub title_font: Option<String>,
    /// Показывать ли clock.
    pub show_clock: bool,
    /// Цвет текста login (по умолчанию = theme.accent_magenta).
    pub title_color: Option<String>,
    /// Показывать ли подсказку "Press Enter to login".
    pub show_hint: bool,
    /// PAM service (обычно "login").
    pub pam_service: String,
    /// Запускать ли WM сразу после login без выбора сессии.
    pub auto_start_session: bool,
}

impl Default for LoginCfg {
    fn default() -> Self {
        LoginCfg {
            title: String::new(),     // empty → use language default
            subtitle: String::new(),
            language: "en".into(),
            title_font: None,
            show_clock: true,
            title_color: None,
            show_hint: true,
            pam_service: "login".into(),
            auto_start_session: true,
        }
    }
}

impl LoginCfg {
    /// Возвращает эффективный заголовок (из конфига или по языку).
    pub fn effective_title(&self) -> String {
        if !self.title.is_empty() {
            self.title.clone()
        } else if self.language == "ru" {
            "БОЛЬШЕ".into()
        } else {
            "MORE".into()
        }
    }
    pub fn effective_subtitle(&self) -> String {
        if !self.subtitle.is_empty() {
            self.subtitle.clone()
        } else if self.language == "ru" {
            "СУПЕРХОТ TTY".into()
        } else {
            "SUPERHOT TTY".into()
        }
    }
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct Binding {
    pub key: String,
    pub mods: Vec<String>,
    pub action: Action,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Action {
    Spawn { cmd: String, args: Vec<String> },
    SpawnX11 { cmd: String, args: Vec<String> },
    SpawnTerminal { cmd: Option<String>, args: Vec<String> },
    Launcher,
    SplitHorizontal,
    SplitVertical,
    Focus { dir: Direction },
    Move { dir: Direction },
    Workspace { n: u8 },
    MoveToWorkspace { n: u8 },
    Close,
    Fullscreen,
    ResizeMode,
    CycleFocus,
    Resize { dir: Direction, delta: f32 },
    Quit,
    Terminal,
    TabNext,
    TabPrev,
    Swap { dir: Direction },
    ToggleLayout,
    Reload,
    /// Запустить скрипт и показать его вывод в popup.
    PopupScript { cmd: String, args: Vec<String> },
    /// Показать статичный popup с текстом.
    Popup { text: String },
}

#[derive(Debug, Clone, Deserialize, PartialEq, Copy)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    Left,
    Right,
    Up,
    Down,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct WorkspaceCfg {
    pub n: u8,
    pub name: String,
    #[serde(default)]
    pub on_init: Option<String>,
}

/// Конфигурация монитора: имя коннектора → workspace bindings.
#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct MonitorCfg {
    /// Имя коннектора DRM, например "HDMI-A-1", "DP-1", "eDP-1".
    pub connector: String,
    /// Список workspace IDs (1..N), которые привязаны к этому монитору.
    /// Пример: [2, 4, 6, 8, 10] — чётные на этом мониторе.
    pub workspaces: Vec<u8>,
    /// Разрешение (пусто = preferred из EDID).
    #[serde(default)]
    pub resolution: Option<(u32, u32)>,
    /// Частота обновления (пусто = default).
    #[serde(default)]
    pub refresh_rate: Option<u32>,
    /// Позиция относительно других мониторов: "left-of X", "right-of X", "above X", "below X", "primary".
    #[serde(default)]
    pub position: Option<String>,
    /// Включить монитор (false = отключить).
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn default_true() -> bool { true }

/// Правило для автоматического размещения окон.
/// Применяется при создании нового X11 окна (через launcher или автоматически).
#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct WindowRule {
    /// Критерий匹配ения. Все непустые поля должны совпасть (AND).
    /// Любое из: window class (WM_CLASS), window title (WM_NAME), app_id (from .desktop).
    #[serde(default)]
    pub match_class: Option<String>,
    #[serde(default)]
    pub match_title: Option<String>,
    #[serde(default)]
    pub match_app_id: Option<String>,
    /// Regex-матчинг (если true — поле трактуется как regex).
    #[serde(default)]
    pub regex: bool,
    /// На какой workspace поместить окно (1..N). Если пусто — текущий активный.
    #[serde(default)]
    pub workspace: Option<u8>,
    /// На какой монитор (по имени коннектора). Если пусто — монитор текущего ws.
    #[serde(default)]
    pub monitor: Option<String>,
    /// Размер плитки в процентах от экрана (width%, height%). Если пусто — auto.
    #[serde(default)]
    pub size: Option<(u32, u32)>,
    /// Позиция плитки в процентах (x%, y%). Если пусто — auto place.
    #[serde(default)]
    pub position: Option<(u32, u32)>,
    /// Сделать окно сфокусированным при появлении.
    #[serde(default = "default_true")]
    pub focus: bool,
    /// Fullscreen при появлении.
    #[serde(default)]
    pub fullscreen: bool,
    /// Не размещать автоматически (для .desktop-only правил, например).
    #[serde(default)]
    pub skip_auto_place: bool,
}

/// Запись автозапуска. Запускается при старте WM.
#[derive(Debug, Clone, Deserialize)]
pub struct AutostartEntry {
    /// Тип команды: "x11" (графическая), "terminal" (в нашем терминале),
    /// "command" (фоновый процесс, без UI).
    #[serde(rename = "type")]
    pub kind: String,
    pub cmd: String,
    #[serde(default)]
    pub args: Vec<String>,
    /// Задержка перед запуском (мс).
    #[serde(default)]
    pub delay_ms: u64,
    /// Workspace на котором запустить (если применимо).
    #[serde(default)]
    pub workspace: Option<u8>,
    /// Монитор для запуска (если применимо).
    #[serde(default)]
    pub monitor: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LauncherCfg {
    #[serde(default)]
    pub desktop_paths: Vec<String>,
    #[serde(default = "default_launcher_rows")]
    pub max_rows: u32,
    #[serde(default)]
    pub custom_entries: HashMap<String, String>,
    #[serde(default = "default_x11_display")]
    pub x11_display: String,
    /// Shell для запуска терминальных приложений (Terminal=true в .desktop).
    #[serde(default = "default_shell")]
    pub terminal_shell: String,
}

fn default_launcher_rows() -> u32 { 12 }
fn default_x11_display() -> String { ":1".into() }
fn default_shell() -> String { "zsh".into() }

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
            terminal_shell: "zsh".into(),
        }
    }
}

/// Конфигурация popups (центральный MCD-styled popup).
#[derive(Debug, Clone, Deserialize)]
pub struct PopupsCfg {
    /// Длительность показа popup (в кадрах, при framerate=60 → 240 = 4 сек).
    #[serde(default = "default_popup_duration")]
    pub duration_frames: u32,
    /// Максимальная ширина popup в процентах от экрана.
    #[serde(default = "default_popup_max_w")]
    pub max_width_pct: u32,
    /// Показывать glitch border (RGB-сдвиг).
    #[serde(default = "default_true")]
    pub glitch_border: bool,
    /// Шрифт для popup (если отличается).
    #[serde(default)]
    pub font: Option<String>,
}

fn default_popup_duration() -> u32 { 240 }
fn default_popup_max_w() -> u32 { 67 }

impl Default for PopupsCfg {
    fn default() -> Self {
        PopupsCfg {
            duration_frames: 240,
            max_width_pct: 67,
            glitch_border: true,
            font: None,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct AudioCfg {
    #[serde(default = "default_true")]
    pub start_pipewire_pulse: bool,
    #[serde(default = "default_true")]
    pub start_wireplumber: bool,
    #[serde(default = "default_volume")]
    pub default_volume: u32,
}

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
    #[serde(default = "default_true")]
    pub start_portal: bool,
    #[serde(default = "default_portal_name")]
    pub service_name: String,
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
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_true")]
    pub steam_passthrough: bool,
    #[serde(default)]
    pub keymap: HashMap<String, String>,
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
    #[serde(default = "default_true")]
    pub dri3: bool,
    #[serde(default = "default_x11_display")]
    pub display: String,
    #[serde(default = "default_x11_size")]
    pub screen_size: (u16, u16),
    #[serde(default = "default_true")]
    pub xtest_input: bool,
    #[serde(default = "default_true")]
    pub hardware_cursor: bool,
    /// Автоматически размещать новые X11 окна на активном workspace.
    /// Если false — окно ждёт пока пользователь не привяжет его вручную.
    #[serde(default = "default_true")]
    pub auto_place_windows: bool,
    /// Использовать hardware overlay planes для X11 окон (0% CPU).
    /// Если false — X11 окна blit'ятся в canvas (CPU).
    /// Требует dri3 = true.
    #[serde(default = "default_true")]
    pub overlay_planes: bool,
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
            auto_place_windows: true,
            overlay_planes: true,
        }
    }
}

// ===== Live reload =====

/// Конфигурация live-reload'а config.toml.
///
/// При изменении файла конфига (через inotify IN_MODIFY) WM перечитывает его
/// и применяет изменения: тема, keybindings, window rules, animation params.
/// Some fields (general.font, x11.display) требуют перезапуска — их изменения
/// логируются как warning и применяются при следующем старте.
#[derive(Debug, Clone, Deserialize)]
pub struct LiveReloadCfg {
    /// Включить watcher.
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Интервал опроса inotify в мс (debounce).
    #[serde(default = "default_live_reload_debounce_ms")]
    pub debounce_ms: u64,
}

fn default_live_reload_debounce_ms() -> u64 { 250 }

impl Default for LiveReloadCfg {
    fn default() -> Self {
        LiveReloadCfg { enabled: true, debounce_ms: 250 }
    }
}

// ===== Animations =====

/// Конфигурация glitch-анимаций MCD-стиля.
///
/// Анимации трёх типов:
///   1. Workspace transition — при переключении ws все символы экрана (терминалы,
///      разделители, X11-окна как квадраты) перебираются случайными символами
///      английского алфавита (заглавными) и квадратами с разной заливкой.
///      Параллельно новый ws "проявляется" — добавляются недостающие символы и
///      убираются лишние. Затем с левого верхнего угла в правый нижний символы
///      фиксируются в финальном состоянии.
///   2. New window — квадрат нового окна заливается перебором, через несколько
///      секунд с левого верхнего угла в правый нижний перебор снимается.
///   3. Random glitch — спонтанный глитч-эффект по тому же принципу (угол → угол),
///      но более быстрый. Срабатывает с вероятностью glitch_intensity на кадр.
#[derive(Debug, Clone, Deserialize)]
pub struct AnimationsCfg {
    /// Включить анимации перехода между workspaces.
    #[serde(default = "default_true")]
    pub workspace_transition: bool,
    /// Включить анимацию появления нового окна.
    #[serde(default = "default_true")]
    pub new_window: bool,
    /// Включить случайные глитч-эффекты (MCD-style).
    #[serde(default = "default_true")]
    pub random_glitch: bool,

    /// Длительность перехода между ws в мс (фаза перебора).
    #[serde(default = "default_ws_transition_ms")]
    pub ws_transition_ms: u32,
    /// Длительность фазы "manifest" нового ws в мс (проявление целевого ws поверх
    /// перебора — добавление недостающих и удаление лишних символов).
    #[serde(default = "default_ws_manifest_ms")]
    pub ws_manifest_ms: u32,
    /// Длительность фазы "reveal" (corner-to-corner) в мс — от левого верхнего
    /// до правого нижнего угла символы фиксируются.
    #[serde(default = "default_ws_reveal_ms")]
    pub ws_reveal_ms: u32,

    /// Сколько мс окно "перебирается" перед началом reveal (для new window).
    #[serde(default = "default_new_window_fill_ms")]
    pub new_window_fill_ms: u32,
    /// Длительность corner-to-corner reveal для нового окна в мс.
    #[serde(default = "default_new_window_reveal_ms")]
    pub new_window_reveal_ms: u32,

    /// Длительность случайного глитча в мс (короче чем ws transition).
    #[serde(default = "default_random_glitch_ms")]
    pub random_glitch_ms: u32,
    /// Частота случайного глитча — раз в N кадров в среднем. 0 = никогда.
    /// Если glitch_intensity в [general] тоже учтится (произведение).
    #[serde(default = "default_random_glitch_every_frames")]
    pub random_glitch_every_frames: u32,

    /// Скорость перебора символов: chars/sec для каждой анимации.
    #[serde(default = "default_glitch_chars_per_sec")]
    pub chars_per_sec: u32,
    /// Скорость перебора для random glitch (обычно выше).
    #[serde(default = "default_random_chars_per_sec")]
    pub random_chars_per_sec: u32,

    /// Использовать заглавные английские буквы (A-Z) в переборе.
    #[serde(default = "default_true")]
    pub glitch_use_alpha: bool,
    /// Использовать квадраты с разной заливкой (FULL BLOCK, DARK SHADE, MEDIUM SHADE,
    /// LIGHT SHADE, BLACK SQUARE, WHITE SQUARE, etc.) в переборе.
    #[serde(default = "default_true")]
    pub glitch_use_blocks: bool,
    /// Использовать цифры в переборе (для большего MCD-стиля).
    #[serde(default)]
    pub glitch_use_digits: bool,
    /// Цвет символов глитча (hex). По умолчанию = accent_cyan.
    #[serde(default)]
    pub glitch_color: Option<String>,
}

fn default_ws_transition_ms() -> u32 { 250 }
fn default_ws_manifest_ms() -> u32 { 200 }
fn default_ws_reveal_ms() -> u32 { 250 }
fn default_new_window_fill_ms() -> u32 { 600 }
fn default_new_window_reveal_ms() -> u32 { 250 }
fn default_random_glitch_ms() -> u32 { 120 }
fn default_random_glitch_every_frames() -> u32 { 360 }
fn default_glitch_chars_per_sec() -> u32 { 60 }
fn default_random_chars_per_sec() -> u32 { 220 }

impl Default for AnimationsCfg {
    fn default() -> Self {
        AnimationsCfg {
            workspace_transition: true,
            new_window: true,
            random_glitch: true,
            ws_transition_ms: 250,
            ws_manifest_ms: 200,
            ws_reveal_ms: 250,
            new_window_fill_ms: 600,
            new_window_reveal_ms: 250,
            random_glitch_ms: 120,
            random_glitch_every_frames: 360,
            chars_per_sec: 60,
            random_chars_per_sec: 220,
            glitch_use_alpha: true,
            glitch_use_blocks: true,
            glitch_use_digits: false,
            glitch_color: None,
        }
    }
}

// ===== IPC =====

/// Конфигурация IPC сокета (i3-msg совместимый протокол).
///
/// WM создаёт UNIX-доменный сокет по пути `socket_path`. Внешние утилиты
/// (например `shtty-msg`) отправляют JSON-команды:
///   { "type": "command", "cmd": "workspace 2" }
///   { "type": "command", "cmd": "exec firefox" }
///   { "type": "command", "cmd": "reload" }
///   { "type": "get_workspaces" }
///   { "type": "get_config" }
///
/// Ответ — JSON.
#[derive(Debug, Clone, Deserialize)]
pub struct IpcCfg {
    /// Включить IPC сокет.
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Путь к сокету. Если пусто — $XDG_RUNTIME_DIR/superhot-tty.sock или
    /// /tmp/superhot-tty-$UID.sock.
    #[serde(default)]
    pub socket_path: Option<String>,
    /// Права на файл сокета (octal). 0600 = только владелец.
    #[serde(default = "default_ipc_socket_mode")]
    pub socket_mode: u32,
}

fn default_ipc_socket_mode() -> u32 { 0o600 }

impl Default for IpcCfg {
    fn default() -> Self {
        IpcCfg {
            enabled: true,
            socket_path: None,
            socket_mode: 0o600,
        }
    }
}

impl Config {
    /// Загружает конфиг из стандартных местоположений (XDG priority).
    pub fn load() -> Self {
        let candidates = config_paths();
        for path in &candidates {
            let p = expand_tilde(path);
            if let Ok(s) = std::fs::read_to_string(&p) {
                match toml::from_str::<Config>(&s) {
                    Ok(mut c) => {
                        log::info!("loaded config from {}", p);
                        c._config_path = Some(std::path::PathBuf::from(p));
                        return c;
                    }
                    Err(e) => log::warn!("config parse error in {}: {}", p, e),
                }
            }
        }
        log::warn!("no config.toml found, using defaults");
        let mut c = Config::default();
        c._config_path = None;
        c
    }

    /// Перезагружает конфиг из того же пути что и в _config_path.
    /// Если путь неизвестен — использует стандартный load().
    /// Возвращает (new_config, changed_path).
    pub fn reload(&self) -> Self {
        if let Some(p) = &self._config_path {
            if let Ok(s) = std::fs::read_to_string(p) {
                match toml::from_str::<Config>(&s) {
                    Ok(mut c) => {
                        log::info!("reloaded config from {}", p.display());
                        c._config_path = Some(p.clone());
                        return c;
                    }
                    Err(e) => {
                        log::warn!("config parse error on reload in {}: {}", p.display(), e);
                        return self.clone();
                    }
                }
            }
        }
        Self::load()
    }

    pub fn default_config_toml() -> &'static str {
        include_str!("../../config/default.toml")
    }
}

/// Пути поиска конфига в порядке приоритета.
pub fn config_paths() -> Vec<String> {
    let mut v = Vec::new();
    // XDG_CONFIG_HOME (обычно ~/.config).
    if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
        v.push(format!("{}/SH-tty/config.toml", xdg));
    }
    // ~/.config/SH-tty/config.toml
    if let Ok(home) = std::env::var("HOME") {
        v.push(format!("{}/.config/SH-tty/config.toml", home));
    }
    // /etc/SH-tty/config.toml (system-wide).
    v.push("/etc/SH-tty/config.toml".into());
    v
}

impl Default for Config {
    fn default() -> Self {
        toml::from_str(Self::default_config_toml())
            .unwrap_or_else(|e| {
                log::error!("default config parse: {}", e);
                Config {
                    general: General::default(),
                    theme: ThemeCfg::default(),
                    login: LoginCfg::default(),
                    keybindings: Vec::new(),
                    workspaces: Vec::new(),
                    monitors: Vec::new(),
                    window_rules: Vec::new(),
                    autostart: Vec::new(),
                    launcher: LauncherCfg::default(),
                    popups: PopupsCfg::default(),
                    audio: AudioCfg::default(),
                    portal: PortalCfg::default(),
                    gamepad: GamepadCfg::default(),
                    x11: X11Cfg::default(),
                    live_reload: LiveReloadCfg::default(),
                    animations: AnimationsCfg::default(),
                    ipc: IpcCfg::default(),
                    _config_path: None,
                }
            })
    }
}

pub fn parse_keycombo(s: &str) -> (Vec<String>, String) {
    let parts: Vec<&str> = s.split('+').collect();
    if parts.is_empty() { return (vec![], s.to_string()); }
    let key = parts.last().unwrap().to_string();
    let mods = parts[..parts.len()-1].iter().map(|s| s.to_string()).collect();
    (mods, key)
}

pub fn parse_color(s: &str) -> (u8, u8, u8) {
    let s = s.trim_start_matches('#');
    if s.len() != 6 { return (0, 0, 0); }
    let r = u8::from_str_radix(&s[0..2], 16).unwrap_or(0);
    let g = u8::from_str_radix(&s[2..4], 16).unwrap_or(0);
    let b = u8::from_str_radix(&s[4..6], 16).unwrap_or(0);
    (r, g, b)
}

pub fn expand_tilde(s: &str) -> String {
    if let Some(rest) = s.strip_prefix("~/") {
        if let Ok(home) = std::env::var("HOME") {
            return format!("{}/{}", home, rest);
        }
    }
    s.to_string()
}

#[allow(dead_code)]
fn _unused(_p: PathBuf) {}
