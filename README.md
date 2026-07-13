# superhot-tty v0.3

**Тайловый оконный менеджер для Linux-консоли в эстетике SuperHot: Mind Control Delete**

> Замена `agetty` с DRM/KMS direct access, login screen (PAM), multi-monitor, window rules, autostart, launcher (.desktop), X11 встраиванием с GPU-ускорением, PipeWire звуком и xdg-desktop-portal для screen share.

---

## Содержание

- [Что нового в v0.3](#что-нового-в-v03)
- [Ключевые возможности](#ключевые-возможности)
- [Архитектура](#архитектура)
- [Установка](#установка)
- [Конфигурация](#конфигурация)
- [Login screen](#login-screen)
- [Multi-monitor](#multi-monitor)
- [Window rules](#window-rules)
- [Autostart](#autostart)
- [Launcher](#launcher)
- [Popups из скриптов](#popups-из-скриптов)
- [Горячие клавиши](#горячие-клавиши)
- [X11 встраивание](#x11-встраивание)
- [Звук и screen share](#звук-и-screen-share)
- [Геймпады](#геймпады)
- [Сборка пакетов](#сборка-пакетов)
- [Структура проекта](#структура-проекта)
- [Roadmap](#roadmap)
- [FAQ](#faq)
- [Лицензия](#лицензия)

---

## Что нового в v0.3

### Login screen (PAM)
- Themed MCD login screen с большим заголовком по центру (как "MORE" / "БОЛЬШЕ")
- PAM аутентификация (service "login")
- После Enter: ввод логина → ввод пароля → переключение на пользователя → запуск WM
- Полностью настраивается в конфиге: текст, цвет, шрифт, язык

### Multi-monitor
- Перечисление всех подключённых коннекторов (HDMI-A-1, DP-1, eDP-1, и т.д.)
- Привязка workspaces к мониторам в конфиге (как в Hyprland)
- Пример: eDP-1 → нечётные ws (1,3,5,7,9), HDMI-A-1 → чётные ws (2,4,6,8,10)
- Переключение ws автоматически переводит фокус на нужный монитор

### 10 workspaces
- Workspaces 1..9 + 0=10 (всего 10, как в i3/Hyprland)
- Mod4+1..9 → workspace 1..9
- Mod4+0 → workspace 10

### Window rules
- Авто-placement X11 окон по правилам в конфиге
- Критерии: WM_CLASS, WM_NAME, app_id (с wildcard/regex)
- Действия: workspace, monitor, size, position, focus, fullscreen
- Применяется при CreateNotify (например Steam запустил игру)

### Autostart
- Список команд в конфиге, запускаемых при старте WM
- Типы: `x11` (графическая), `terminal` (в нативном терминале), `command` (фоновый)
- Поддержка задержки (delay_ms) и указания workspace/monitor

### Launcher improvements
- `.desktop` файлы с `Terminal=true` открываются в нативном терминале (через shell -c)
- Графические приложения открываются как X11 плитки
- Никаких захардкоженных биндов

### Popups из скриптов
- `PopupScript` action — запускает скрипт, показывает stdout в popup
- Полезно для уведомлений, статуса системы, ASCII art

### Пакетирование
- `build-packages.sh` генерирует 3 пакета:
  - pacman `.pkg.tar.zst` (Arch Linux)
  - `.deb` (Debian/Ubuntu)
  - `.rpm` (Fedora/RHEL/openSUSE)
- Использует `cargo-deb`, `cargo-rpm`, `makepkg` (или FPM fallback)

### Конфиг в XDG-пути
- `~/.config/SH-tty/config.toml` (XDG_CONFIG_HOME)
- `/etc/SH-tty/config.toml` (system-wide)
- Никаких захардкоженных биндов или настроек

---

## Ключевые возможности

### Менеджер окон
- **Тайловый layout** в стиле BSP/i3: бинарное дерево тайлов, split h/v, ratio resize
- **10 workspaces** с независимыми layout-деревьями
- **Multi-monitor**: per-monitor workspace binding (как Hyprland)
- **Перемещение окон** по тайловой сетке и между workspaces
- **Fullscreen** toggle, **resize mode**, **cycle focus**

### Login screen
- **PAM аутентификация** (service "login")
- **MCD-themed**: большой заголовок по центру, glitch border, corner brackets
- **Настраиваемый текст**: "MORE" / "БОЛЬШЕ" / свой
- **Clock, hint, error display**

### Запуск программ
- **Rofi-подобный launcher** `Mod4+D` — читает `.desktop` файлы
- **Terminal=true** приложения открываются в нативном терминале
- **Window rules** для авто-placement по WM_CLASS/WM_NAME
- **Autostart** для запуска при старте WM

### X11 встраивание
- **Xephyr** на `:1` + **Composite redirect** + **XDamage**
- **DRI3 + DMA-BUF infrastructure** для GPU-ускорения
- **Auto-place**: новые X11 окна автоматически размещаются по правилам или на active ws

### Ввод
- **Клавиатура** через evdev (эксклюзивный grab)
- **Мышь** через evdev + софтверный курсор MCD-стиля
- **Геймпады**: evdev passthrough (Steam Input) + опционально SDL2

### Звук и screen share
- **PipeWire** + pipewire-pulse + wireplumber
- **xdg-desktop-portal backend** для screen sharing в OBS/Discord

---

## Архитектура

```
+--------------------------------------------------------------------+
|                       superhot-tty v0.3                            |
|                                                                    |
|  1. LOGIN SCREEN (PAM)                                             |
|     ┌─────────────────────────┐                                    |
|     │       БОЛЬШЕ / MORE     │                                    |
|     │   (themed MCD login)    │                                    |
|     │   login → password      │                                    |
|     └───────────┬─────────────┘                                    |
|                 │ PAM auth success                                 |
|                 ▼                                                  |
|  2. WM (after switch_to_user)                                      |
|     +---------+ +-----------+ +----------+ +-------------------+   |
|     | Keyboard| |  Mouse    | | Gamepad  | |   Config (TOML)   |   |
|     | (evdev) | |  (evdev)  | | (SDL2/   | |   ~/.config/      |   |
|     |         | | +cursor   | |  passthru)| |   SH-tty/         |   |
|     +----+----+ +-----+-----+ +----+-----+ +---------+---------+   |
|          \           \            /                  |             |
|     +-----v-----------v----------v-+      +----------v----------+  |
|     |     Window Rules Engine      |      |     Autostart       |  |
|     | (match WM_CLASS → placement) |      | (run on WM start)   |  |
|     +-------------+----------------+      +----------+----------+  |
|                   |                                 |              |
|     +-------------v---------------------------------v----------+   |
|     |              Launcher + Layout + Workspaces             |   |
|     |       (.desktop scanner, BSP tree, 10 ws)               |   |
|     +-------------------------+-------------------------------+   |
|                               |                                     |
|                     +---------v---------+                           |
|                     |  Canvas + Font    |                           |
|                     +---------+---------+                           |
|                               |                                     |
|                     +---------v---------+                           |
|                     |  Multi-Monitor    |  ← per-monitor CRTC       |
|                     |  DRM/KMS          |  ← per-monitor ws binding |
|                     +-------------------+                           |
|                                                                    |
|  Sidecar: Xephyr :1, PipeWire, xdg-desktop-portal                  |
+--------------------------------------------------------------------+
```

---

## Установка

### Вариант 1: Из пакета (рекомендуется)

Соберите пакеты (см. [Сборка пакетов](#сборка-пакетов)) или скачайте pre-built:

**Arch Linux:**
```bash
sudo pacman -U superhot-tty-0.3.0-1-x86_64.pkg.tar.zst
```

**Debian/Ubuntu:**
```bash
sudo dpkg -i superhot-tty_0.3.0_amd64.deb
sudo apt-get install -f  # разрешить зависимости
```

**Fedora/RHEL:**
```bash
sudo dnf install superhot-tty-0.3.0-1.x86_64.rpm
```

После установки:
```bash
sudo systemctl disable getty@tty1
sudo systemctl enable superhot-tty@tty1
sudo reboot
```

### Вариант 2: Из исходников

```bash
git clone https://github.com/TumRedSun/SH-tty.git
cd SH-tty
cargo build --release
sudo install -Dm755 target/release/superhot-tty /usr/local/bin/
sudo install -Dm644 systemd/superhot-tty@.service /etc/systemd/system/
sudo install -Dm644 config/default.toml /etc/SH-tty/config.toml
sudo systemctl disable getty@tty1
sudo systemctl enable superhot-tty@tty1
sudo reboot
```

### Зависимости

**Обязательные:**
```
rust cargo gcc pkgconf systemd kbd zsh pam
xorg-server-xephyr
pipewire pipewire-pulse wireplumber
xdg-desktop-portal
```

**Опциональные:**
```
sdl2                    # для --features gamepad-sdl2
xdg-desktop-portal-gtk  # GTK file chooser
```

---

## Конфигурация

### Пути

1. `$XDG_CONFIG_HOME/SH-tty/config.toml` (обычно `~/.config/SH-tty/config.toml`)
2. `/etc/SH-tty/config.toml` (system-wide)

User config имеет приоритет над system-wide.

### Полный пример

См. `config/default.toml` — полный конфиг со всеми секциями и комментариями.

### Основные секции

```toml
[general]
shell = "zsh"
workspace_count = 10              # 1..9 + 0=10
framerate = 60

[login]
title = ""                        # empty → language default
language = "en"                   # "en" → "MORE", "ru" → "БОЛЬШЕ"
show_clock = true
pam_service = "login"

[[monitors]]
connector = "eDP-1"
workspaces = [1, 3, 5, 7, 9]     # нечётные

[[monitors]]
connector = "HDMI-A-1"
workspaces = [2, 4, 6, 8, 10]    # чётные
position = "right-of eDP-1"

[[window_rules]]
match_class = "firefox"
workspace = 2

[[window_rules]]
match_class = "Steam"
workspace = 5
focus = true

[[autostart]]
type = "x11"
cmd = "firefox"
delay_ms = 2000
workspace = 2

[[keybindings]]
key = "d"
mods = ["Super"]
action = { type = "launcher" }
```

---

## Login screen

При запуске на tty1, superhot-tty показывает themed login screen:

```
        ╔════════════════════╗
        ║                    ║
        ║       MORE         ║   ← configurable title
        ║   SUPERHOT TTY     ║   ← configurable subtitle
        ║                    ║
        ║     12:34:56       ║   ← clock (optional)
        ║                    ║
        ║ Press Enter to login║   ← hint (optional)
        ║                    ║
        ╚════════════════════╝
```

После Enter:
1. **Login:** — ввод логина
2. **Password:** — ввод пароля (скрытые символы •)
3. PAM аутентификация через service `login`
4. При успехе: переключение на UID/GID пользователя, загрузка user config, запуск WM
5. При ошибке: показ сообщения, возврат к Welcome

### Настройка

```toml
[login]
title = "MORE"                   # свой текст (пусто = language default)
subtitle = "SUPERHOT TTY"
language = "en"                  # или "ru" для "БОЛЬШЕ" / "СУПЕРХОТ TTY"
show_clock = true
show_hint = true
pam_service = "login"
title_color = "#FF2E97"          # optional override
# title_font = "Lat2-Terminus16" # optional override
```

---

## Multi-monitor

### Конфигурация

В конфиге `[[monitors]]` — список мониторов с привязкой workspaces:

```toml
[[monitors]]
connector = "eDP-1"              # встроенный экран ноутбука
workspaces = [1, 3, 5, 7, 9]     # нечётные ws на этом мониторе
enabled = true
position = "primary"

[[monitors]]
connector = "HDMI-A-1"
workspaces = [2, 4, 6, 8, 10]    # чётные ws на этом мониторе
enabled = true
position = "right-of eDP-1"
resolution = [2560, 1440]        # optional
refresh_rate = 144               # optional

[[monitors]]
connector = "DP-1"
workspaces = [1, 2, 3]
resolution = [1920, 1080]
position = "left-of eDP-1"
```

### Поведение

- При `Mod4+1` (ws 1) — фокус переходит на eDP-1 (т.к. ws 1 привязан к нему)
- При `Mod4+2` (ws 2) — фокус переходит на HDMI-A-1
- Layout каждого workspace сохраняется независимо
- Каждый монитор имеет свой CRTC + dumb buffer + page-flip

### Имена коннекторов

Имена определяются DRM. Распространённые:
- `eDP-1` — встроенный экран ноутбука
- `HDMI-A-1`, `HDMI-B-1` — HDMI выходы
- `DP-1`, `DP-2` — DisplayPort
- `DVI-D-1` — DVI
- `VGA-1` — VGA

Посмотреть доступные: `cat /sys/class/drm/*/status` или в логе superhot-tty при старте.

---

## Window rules

### Синтаксис

```toml
[[window_rules]]
match_class = "firefox"          # WM_CLASS (instance)
# match_title = "Mozilla Firefox"  # WM_NAME (optional, AND)
# match_app_id = "org.mozilla.firefox"  # app_id (optional, AND)
regex = false                    # true → wildcard match (* and ?)
workspace = 2                    # поместить на workspace 2
# monitor = "HDMI-A-1"           # или на конкретный монитор
size = [80, 80]                  # 80% ширины, 80% высоты (optional)
position = [10, 10]              # 10% x, 10% y (optional)
focus = true                     # сделать сфокусированным
fullscreen = false               # открыть в fullscreen
skip_auto_place = false          # не размещать автоматически
```

### Примеры

**Steam games → workspace 5 (MEDIA):**
```toml
[[window_rules]]
match_class = "Steam"
workspace = 5
focus = true
```

**Discord → workspace 4 (CHAT):**
```toml
[[window_rules]]
match_class = "discord"
workspace = 4
```

**Firefox → workspace 2 (WEB):**
```toml
[[window_rules]]
match_class = "firefox"
workspace = 2
```

**GIMP — большой размер:**
```toml
[[window_rules]]
match_class = "Gimp"
workspace = 5
size = [80, 80]
position = [10, 10]
```

### Как это работает

1. При `CreateNotify` в X11 сервере (например, Steam запустил игру)
2. Мы получаем `WM_CLASS` через `XGetWindowProperty`
3. Проходим по всем `[[window_rules]]` в конфиге
4. Первое совпавшее правило применяется:
   - Переключаемся на указанный workspace (если есть)
   - Создаём X11 tile на активном workspace
   - Привязываем окно к tile
5. Если ни одно правило не совпало — открываем на текущем active ws

---

## Autostart

Список команд, запускаемых при старте WM (после login):

```toml
[[autostart]]
type = "command"                 # фоновый процесс без UI
cmd = "pipewire"
delay_ms = 0

[[autostart]]
type = "command"
cmd = "pipewire-pulse"
delay_ms = 500

[[autostart]]
type = "x11"                     # графическое приложение
cmd = "firefox"
args = []
delay_ms = 2000
workspace = 2                    # открыть на workspace 2

[[autostart]]
type = "terminal"                # в нативном терминале
cmd = "htop"
delay_ms = 0
workspace = 1

[[autostart]]
type = "x11"
cmd = "discord"
delay_ms = 3000
workspace = 4
```

### Типы

- `command` — фоновый процесс (pipewire, wireplumber, и т.д.)
- `x11` — графическое приложение (firefox, discord, steam)
- `terminal` — запустить команду в нашем нативном терминале (htop, vim, btop)

Каждая команда запускается в отдельном потоке с указанной задержкой.

---

## Launcher

`Mod4+D` открывает launcher в центре экрана:

```
    ╔═══════════════════════════════════════╗
    ║ RUN // superhot launcher               ║
    ╠═══════════════════════════════════════╣
    ║ > firef                               ║
    ╠═══════════════════════════════════════╣
    ║ ▸ Firefox                       Web   ║
    ║   Firefox Developer Edition    Web    ║
    ║   Firejail                      Sys   ║
    ╠═══════════════════════════════════════╣
    ║ ↑↓ navigate  Enter run  Esc close [3/124] ║
    ╚═══════════════════════════════════════╝
```

### Поведение

- Читает `.desktop` файлы из:
  - `/usr/share/applications`
  - `/usr/local/share/applications`
  - `~/.local/share/applications`
- Если `.desktop` имеет `Terminal=true` → запускаем `Exec` в нашем нативном терминале
- Если `Terminal=false` → запускаем как X11 приложение на нашем display `:1`
- Авто-создаётся X11 tile для графического приложения

### Кастомные записи

```toml
[launcher.custom_entries]
"terminal: bash" = "bash"
"reload config" = "superhot-tty-reload"
"system status" = "neofetch"
```

---

## Popups из скриптов

`PopupScript` action — запускает скрипт и показывает его stdout в MCD-styled popup:

```toml
[[keybindings]]
key = "p"
mods = ["Super"]
action = { type = "popup_script", cmd = "echo", args = ["Hello from SuperHot TTY!"] }

[[keybindings]]
key = "s"
mods = ["Super"]
action = { type = "popup_script", cmd = "systemctl", args = ["status", "pipewire"] }

[[keybindings]]
key = "b"
mods = ["Super"]
action = { type = "popup_script", cmd = "neofetch", args = ["--stdout"] }
```

Также можно показывать статичный текст:
```toml
[[keybindings]]
key = "h"
mods = ["Super"]
action = { type = "popup", text = "Mod4+D launcher\nMod4+1..0 workspaces\nMod4+Enter terminal" }
```

Popup умеет:
- Multiline ASCII текст
- Glitch border (RGB-сдвиг)
- Corner brackets (MCD style)
- Авто-размер по контенту
- Авто-закрытие через `duration_frames` (default 240 = 4 сек)

---

## Горячие клавиши

Все биндинги в конфиге (`[[keybindings]]`). По умолчанию (см. `config/default.toml`):

`Mod4` = Super/Windows key.

| Hotkey              | Действие                                  |
|---------------------|-------------------------------------------|
| `Mod4+D`            | Launcher                                  |
| `Mod4+Enter`        | Новый терминал (zsh)                      |
| `Mod4+V`            | Split vertical                            |
| `Mod4+H/J/K/L`      | Фокус left/down/up/right                  |
| `Mod4+Shift+H/J/K/L`| Переместить окно                          |
| `Mod4+Ctrl+H/J/K/L` | Swap с соседом                            |
| `Mod4+R`            | Resize mode (HJKL)                        |
| `Mod4+Alt+H/J/K/L`  | Resize split в направлении                |
| `Mod4+1..9`         | Workspace 1..9                            |
| `Mod4+0`            | Workspace 10                              |
| `Mod4+Shift+1..0`   | Переместить окно на workspace 1..10       |
| `Mod4+Q`            | Закрыть тайл                              |
| `Mod4+F`            | Fullscreen toggle                         |
| `Mod4+Space`        | Cycle focus                               |
| `Mod4+E`            | Открыть X11-плитку (xterm)                |
| `Mod4+P`            | Popup script (echo Hello)                 |
| `Mod4+Ctrl+R`       | Reload config                             |
| `Mod4+Shift+E`      | Quit                                      |

---

## X11 встраивание

### Запуск через launcher

1. `Mod4+D` → начните вводить имя программы
2. `↑↓` навигация, `Enter` запуск
3. Графическое приложение откроется в новой X11 плитке на текущем ws
4. Терминальное приложение откроется в нативном терминале

### Запуск вручную

```bash
# Из любого терминала внутри superhot-tty:
DISPLAY=:1 firefox
DISPLAY=:1 steam
DISPLAY=:1 discord
```

### Авто-placement

Когда X11 приложение создаёт окно (CreateNotify):
1. Получаем `WM_CLASS` и `WM_NAME`
2. Ищем matching `[[window_rules]]` в конфиге
3. Применяем правило (workspace, size, position, focus, fullscreen)
4. Если правило не найдено — открываем на текущем active ws

---

## Звук и screen share

### PipeWire

superhot-tty запускает:
- `pipewire` — основной daemon
- `pipewire-pulse` — PulseAudio совместимость
- `wireplumber` — session manager

```bash
pactl set-sink-volume @DEFAULT_SINK@ 80%
pactl set-sink-mute @DEFAULT_SINK@ toggle
pactl list sinks short
```

### xdg-desktop-portal

Регистрируется DBus service `org.freedesktop.impl.portal.desktop.SuperHot` с интерфейсом `ScreenCast`.

**Discord/Slack:** Share Screen → выберите "SuperHot" monitor.

**OBS:** Add Source → ScreenCast (Portal) → выберите "SuperHot".

---

## Геймпады

### Steam (нативно)

Steam Input работает через evdev — superhot-tty не вмешивается:

```bash
DISPLAY=:1 steam
```

### Не-Steam (опционально)

Соберите с `--features gamepad-sdl2`:

```toml
[gamepad.keymap]
"a" = "Return"
"b" = "Escape"
"dpad_up" = "k"
"dpad_down" = "j"
"left_shoulder" = "bracketleft"
```

---

## Сборка пакетов

`build-packages.sh` генерирует 3 пакета:

```bash
./build-packages.sh           # все 3 пакета
./build-packages.sh pacman    # только Arch
./build-packages.sh deb       # только Debian
./build-packages.sh rpm       # только Fedora
```

Результат в `target/packages/`:
- `superhot-tty-0.3.0-1-x86_64.pkg.tar.zst` (Arch)
- `superhot-tty_0.3.0_amd64.deb` (Debian)
- `superhot-tty-0.3.0-1.x86_64.rpm` (Fedora)
- `superhot-tty-packages-0.3.0.tar.gz` (все 3 в одном архиве)

### Требования для сборки

- `cargo`, `rust` — для бинарника
- `makepkg` (Arch) или `cargo-deb` (Debian) или `cargo-rpm` (Fedora)
- Или `fpm` как универсальный fallback

---

## Структура проекта

```
superhot-tty/
├── Cargo.toml
├── build-packages.sh           # генерация 3 пакетов
├── README.md
├── LICENSE
├── config/
│   └── default.toml            # пример конфига
├── skel/
│   └── zshrc.example           # пример .zshrc с MCD-стилем
├── systemd/
│   └── superhot-tty@.service
├── packaging/
│   ├── arch/
│   │   ├── PKGBUILD
│   │   └── superhot-tty.install
│   ├── debian/                  # (используется cargo-deb)
│   └── rpm/
│       └── superhot-tty.spec
├── debian/
│   ├── control
│   ├── postinst
│   └── prerm
└── src/
    ├── main.rs                  # entry: login → WM flow
    ├── login/mod.rs             # PAM login screen (NEW v0.3)
    ├── config/
    │   ├── mod.rs               # TOML config
    │   └── window_rules.rs      # window rules engine (NEW v0.3)
    ├── launcher/mod.rs          # rofi-like launcher
    ├── drm/
    │   ├── kms.rs               # DRM/KMS ioctls
    │   ├── multi_monitor.rs     # multi-monitor backend (NEW v0.3)
    │   └── fbdev.rs             # legacy fallback
    ├── render/                  # canvas, font, text
    ├── term/                    # PTY + VTerm
    ├── layout/                  # BSP/i3 + workspaces
    ├── input/                   # keyboard, mouse, gamepad
    ├── x11/                     # Xephyr + Composite + DRI3
    ├── audio/                   # PipeWire
    ├── portal/                  # xdg-desktop-portal
    └── ui/                      # theme + popups
```

---

## Roadmap

### v0.3 (текущая)
- ✅ Login screen (PAM)
- ✅ Multi-monitor с per-monitor workspace binding
- ✅ 10 workspaces (Mod4+1..0)
- ✅ Window rules engine
- ✅ Autostart
- ✅ Launcher: Terminal=true → нативный терминал
- ✅ Popups из скриптов
- ✅ 3 пакета (pacman/.deb/.rpm)
- ✅ Config в XDG-пути

### v0.4 (план)
- Полная DRI3/DMA-BUF реализация (FFI к xcb-dri3)
- Hardware DRM cursor plane
- Hardware DRM overlay planes (0% CPU для X11)
- Live reload конфигурации
- Анимации перехода между workspaces
- IPC сокет (как i3-msg)
- Полная xterm совместимость (libvterm)

---

## FAQ

### Q: Как переключиться на обычный TTY?
**A:** `Ctrl+Alt+F2` — стандартный getty. `Ctrl+Alt+F1` — обратно в superhot-tty.

### Q: Не работает login — что делать?
**A:** Проверьте:
1. `/etc/pam.d/login` существует
2. В логе: `journalctl -u superhot-tty@tty1 | grep -i pam`
3. Для отладки можно временно отключить PAM в конфиге (не рекомендуется)

### Q: Мультимонитор не работает — только один экран
**A:** Проверьте:
1. Конфиг `[[monitors]]` — правильные имена коннекторов
2. `cat /sys/class/drm/*/status` — какие коннекторы connected
3. В логе superhot-tty при старте: "found N connectors, M connected"

### Q: Окна X11 не размещаются по правилам
**A:**
1. Проверьте `WM_CLASS` через `xprop WM_CLASS` (в Xephyr)
2. `match_class` — case-insensitive contains
3. Если нужно точное совпадение — `regex = true` и используйте wildcard

### Q: Steam не видит геймпад
**A:** Steam Input требует evdev. Проверьте что пользователь в группе `input`:
```bash
groups $USER
sudo usermod -aG input $USER
```

### Q: Как откатиться к стандартному getty?
```bash
sudo systemctl disable superhot-tty@tty1
sudo systemctl enable getty@tty1
sudo systemctl daemon-reload
sudo reboot
```

---

## Лицензия

MIT License. См. [LICENSE](LICENSE).
