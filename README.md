# superhot-tty v0.3

**Тайловый оконный менеджер для Linux-консоли в эстетике SuperHot: Mind Control Delete**

> ✅ **Полностью компилируется и работает.** Замена `agetty` с DRM/KMS direct access, login screen (PAM/shadow), multi-monitor, window rules, autostart, launcher (.desktop), X11 встраиванием с GPU-ускорением, PipeWire звуком и xdg-desktop-portal для screen share.

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
- PAM аутентификация (service "login") при сборке с `--features pam`
- Fallback на `/etc/shadow` + `crypt(3)` без PAM (по умолчанию)
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
libpam0g-dev            # для --features pam (иначе fallback на /etc/shadow)
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
    │   ├── window_rules.rs      # window rules engine (NEW v0.3)
    │   └── watcher.rs           # live-reload inotify watcher (NEW v0.5)
    ├── launcher/mod.rs          # rofi-like launcher
    ├── drm/
    │   ├── kms.rs               # DRM/KMS ioctls
    │   ├── multi_monitor.rs     # multi-monitor backend (NEW v0.3)
    │   └── fbdev.rs             # legacy fallback
    ├── render/                  # canvas, font, text, glitch animations (v0.5)
    ├── term/                    # PTY + VTerm (with libvterm FFI, v0.5)
    ├── layout/                  # BSP/i3 + workspaces
    ├── input/                   # keyboard, mouse, gamepad
    ├── x11/                     # Xephyr + Composite + DRI3
    ├── ipc/                     # i3-msg-compatible IPC socket (NEW v0.5)
    ├── bin/shtty_msg.rs         # standalone IPC CLI (NEW v0.5)
    ├── audio/                   # PipeWire
    ├── portal/                  # xdg-desktop-portal
    └── ui/                      # theme + popups
```

---

## Roadmap

### v0.5 (текущая — реализовано)
- ✅ **Полная DRI3/DMA-BUF реализация (FFI к xcb-dri3)** — `src/x11/dri3.rs`
- ✅ **Hardware DRM cursor plane** — `src/drm/cursor.rs`
- ✅ **Hardware DRM overlay planes (0% CPU для X11)** — `src/drm/planes.rs`
- ✅ **Live reload конфигурации** — inotify watcher на `config.toml`, debounce, hot-apply theme/keybindings/window_rules/animations/ipc, warn для перезапускаемых полей — `src/config/watcher.rs`
- ✅ **Анимации перехода между workspaces (glitch MCD-style)** — три фазы: transition (перебор) → manifest (проявление нового ws) → reveal (corner-to-corner фиксация). Плюс new-window анимация и random glitch — `src/render/glitch.rs`
- ✅ **IPC сокет (i3-msg-совместимый протокол)** — UNIX-domain сокет, JSON-команды, CLI `shtty-msg` — `src/ipc/mod.rs`, `src/bin/shtty_msg.rs`
- ✅ **Полная xterm совместимость (libvterm)** — runtime FFI к `libvterm.so.0` через `libloading`, fallback на расширенный built-in парсер (CSI/OSC/DCS, SGR truecolor, DEC modes, save/restore cursor, DECSC/DECRC) — `src/term/libvterm.rs`, `src/term/vterm.rs`

### v0.4
- ✅ DRI3/DMA-BUF FFI
- ✅ Hardware cursor
- ✅ Overlay planes

### v0.3
- ✅ Login screen (PAM)
- ✅ Multi-monitor с per-monitor workspace binding
- ✅ 10 workspaces (Mod4+1..0)
- ✅ Window rules engine
- ✅ Autostart
- ✅ Launcher: Terminal=true → нативный терминал
- ✅ Popups из скриптов
- ✅ 3 пакета (pacman/.deb/.rpm)
- ✅ Config в XDG-пути

---

## v0.4: GPU acceleration

### DRI3 + DMA-BUF (FFI к xcb-dri3)

Полная реализация через FFI к `libxcb-dri3` (динамическая загрузка через `libloading`):

- `DRI3QueryVersion` — проверка поддержки DRI3 на X-сервере
- `DRI3Open` — получение authenticated DRM fd от X-сервера
- `DRI3BuffersFromPixmap` (DRI3 1.2+) — получение dma-buf fd с modifiers
- `DRI3BufferFromPixmap` (DRI3 1.0) — fallback без modifiers
- Автоматический выбор API по версии DRI3

`src/x11/dri3.rs` — полностью реализован, использует `libloading` для runtime загрузки `libxcb-dri3.so.0`.

### Hardware DRM cursor plane

DRM dedicated cursor plane через `DRM_IOCTL_MODE_CURSOR2`:

- Узнёт размер курсора через `DRM_CAP_CURSOR_WIDTH/HEIGHT` (типично 64×64)
- Создаёт dumb buffer ARGB для курсора
- MCD-styled crosshair курсор (неоновый крестик с магента-центром и glow)
- `move_to(x, y)` — только обновляет позицию, **0% CPU**, без перерисовки framebuffer
- `show()` / `hide()` — управление видимостью
- `update_image()` — смена изображения (для анимации)

`src/drm/cursor.rs` — `HardwareCursor` struct.

### Hardware DRM overlay planes

Overlay planes для X11 окон — **0% CPU** rendering:

1. При `CreateNotify` X11 окна:
   - `CompositeNameWindowPixmap` → pixmap
   - `DRI3BuffersFromPixmap` → dma-buf fd
2. Импорт dma-buf в DRM:
   - `DRM_IOCTL_PRIME_FD_TO_HANDLE` → GEM handle
   - `DRM_IOCTL_MODE_ADDFB2` с modifiers → DRM framebuffer
3. `DRM_IOCTL_MODE_SETPLANE` — присвоение framebuffer к overlay plane
4. GPU composites overlay поверх primary scanout на лету

`src/drm/planes.rs` — `OverlayManager` с отслеживанием занятых planes, auto-finding free overlay plane per CRTC.

### Конфигурация

```toml
[x11]
dri3 = true              # DRI3 + DMA-BUF
hardware_cursor = true   # DRM cursor plane
overlay_planes = true    # 0% CPU X11 rendering
```

Если `overlay_planes = false` — X11 окна blit'ятся в canvas (CPU). Fallback на случай если GPU не поддерживает достаточно overlay planes.

---

## v0.5: Live reload, Glitch animations, IPC, libvterm

### Live reload конфигурации

Inotify- watcher на `config.toml` (через `libc::inotify_add_watch` в отдельном потоке).
При изменении файла — debounce (`live_reload.debounce_ms`, default 250 мс), затем перечитывание.

Применяется на лету (без перезапуска WM):
- `theme.*` — пересобирается `Theme` и сразу используется при рендере
- `keybindings` — новые биндинги активны сразу
- `window_rules` — применяются к новым окнам
- `animations.*` — параметры анимаций
- `ipc.*` (кроме `socket_path` — требует перезапуска)
- `live_reload.*`
- `general.glitch_intensity`, `general.framerate`, `general.gap/border/...`
- `popups.*`

Требуют перезапуска (логируется как warning):
- `general.font`, `general.font_size` — нужны новая загрузка PSF
- `general.workspace_count` — пересоздание структур Workspaces
- `monitors` — reinit DRM/KMS
- `x11.display`, `x11.screen_size` — reinit Xephyr
- `x11.dri3`, `x11.overlay_planes` — reinit DRM planes

`src/config/watcher.rs` — `ConfigWatcher` + `ConfigDiff` (детектор изменений).

### Glitch-анимации (MCD-style)

Три типа анимаций, все с corner-to-corner reveal (TL → BR диагональ):

**1. Workspace transition** (`animations.workspace_transition = true`)

При переключении workspaces:
1. **Transition** (`ws_transition_ms`, default 250 мс) — все символы экрана
   (терминальные ячейки, разделители, X11-окна как квадраты █▓) начинают
   перебираться случайными символами:
   - Заглавные A-Z (если `glitch_use_alpha = true`)
   - Квадраты с разной заливкой: ░ ▒ ▓ █ ■ □ ▢ ▣ ▤ ▥ ▦ ▧ ▨ ▩ ▀ ▄ ▌ ▐
     (если `glitch_use_blocks = true`)
   - Опционально цифры 0-9 (`glitch_use_digits`)
2. **Manifest** (`ws_manifest_ms`, default 200 мс) — поверх перебора проявляется
   целевой ws: добавляются недостающие символы, исчезают лишние. Перебор всё
   ещё идёт для остальных ячеек.
3. **Reveal** (`ws_reveal_ms`, default 250 мс) — diagonal corner-to-corner:
   от левого верхнего угла к правому нижнему символы фиксируются в финальном
   состоянии (terminal cells из целевого ws).

**2. New window** (`animations.new_window = true`)

При создании нового X11-окна или нативного терминального tile:
1. **Fill** (`new_window_fill_ms`, default 600 мс) — квадрат нового окна
   заливается перебором (фон глитча — тёмный, символы glitch_color).
2. **Reveal** (`new_window_reveal_ms`, default 250 мс) — corner-to-corner
   внутри rect окна: символы фиксируются, видно реальный терминал/окно.

**3. Random glitch** (`animations.random_glitch = true`)

Спонтанный быстрый глитч по случайному под-прямоугольнику экрана:
- Длительность: `random_glitch_ms` (default 120 мс — короче, чем ws transition)
- Частота: в среднем раз в `random_glitch_every_frames` кадров (default 360),
  умножается на `general.glitch_intensity` (0.0..1.0, default 0.15)
- Скорость перебора: `random_chars_per_sec` (default 220 chars/sec —
  значительно быстрее, чем у ws/new_window)
- Corner-to-corner reveal как и у остальных, но быстрый

Цвет глитча: `animations.glitch_color` (по умолчанию = `theme.accent_cyan`).
Скорость перебора для ws/new_window: `animations.chars_per_sec` (default 60).

`src/render/glitch.rs` — `AnimationManager`, `ActiveAnimation`, `CharSnapshot`,
`snapshot_workspace()`, `random_glitch_char()`.

### IPC сокет (i3-msg совместимый)

UNIX-доменный сокет (default `$XDG_RUNTIME_DIR/superhot-tty.sock` или
`/tmp/superhot-tty-$UID.sock`). Протокол: JSON-запросы и JSON-ответы.

Запросы:
```json
{ "type": "command", "cmd": "workspace 2" }
{ "type": "command", "cmd": "exec firefox" }
{ "type": "command", "cmd": "exec --no-startup-id alacritty" }
{ "type": "command", "cmd": "kill" }
{ "type": "command", "cmd": "reload" }
{ "type": "command", "cmd": "split vertical" }
{ "type": "command", "cmd": "focus left" }
{ "type": "command", "cmd": "fullscreen toggle" }
{ "type": "command", "cmd": "layout toggle" }
{ "type": "command", "cmd": "launcher" }
{ "type": "command", "cmd": "glitch" }
{ "type": "get_workspaces" }
{ "type": "get_config" }
{ "type": "get_focused" }
{ "type": "get_version" }
```

Ответы:
```json
{ "status": "ok", "result": "switched to workspace 2" }
{ "status": "error", "error": "unknown command: foo" }
```

CLI утилита `shtty-msg` (второй binary в crate):
```bash
shtty-msg "workspace 2"
shtty-msg "exec firefox"
shtty-msg --get-workspaces
shtty-msg --get-version
```

Права сокета: `ipc.socket_mode` (default `0o600`, только владелец).

`src/ipc/mod.rs` — `IpcServer`, `IpcRequest`, `IpcResponse`, `parse_i3_command()`.
`src/bin/shtty_msg.rs` — standalone CLI binary.

### Полная xterm совместимость (libvterm)

Runtime FFI к `libvterm.so.0` через `libloading`. Если библиотека доступна —
VTerm проксирует туда все байты от PTY и синхронизирует grid. Если нет —
fallback на расширенный built-in ANSI-парсер.

Built-in парсер покрывает:
- CSI: cursor position (H/f), up/down/fwd/back (A/B/C/D), CNL/CPL (E/F),
  column/row set (G/d), erase display/line (J/K), SGR (m), scroll region (r),
  device status report (n=6), insert/delete lines (L/M), delete chars (P),
  insert blanks (@), scroll (S/T), save/restore cursor (s/u)
- Private modes (CSI ?): alt screen (1049, 47, 1047), cursor visibility (25),
  autowrap (7), application cursor keys (1), cursor blink (12)
- ESC sequences: IND (D), NEL (E), RI (M), RIS (c), DECSC/DECRC (7/8),
  application keypad (= / >), DCS (P, игнорируется), ST (\\)
- OSC: 0;title / 2;title (window title), 4;N;COLOR (palette, игнорируется),
  8;params;uri (hyperlink, игнорируется), 52;clipboard (игнорируется)
- SGR: 0 (reset), 1/22 (bold), 3/23 (italic), 4/24 (underline), 7/27 (reverse),
  30-37/40-47 (16 colors), 38;5;N / 48;5;N (256 colors), 38;2;R;G;B / 48;2;R;G;B
  (truecolor → аппроксимация в 16-color палитру), 90-97/100-107 (bright)

`src/term/libvterm.rs` — `LibVTermHandle`, FFI bindings.
`src/term/vterm.rs` — `VTerm` с опциональным libvterm backend.

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
