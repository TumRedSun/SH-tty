# superhot-tty v0.5

**Тайловый оконный менеджер для Linux-консоли в эстетике SuperHot: Mind Control Delete**

> Полная замена `agetty` с DRM/KMS direct access, login screen (PAM/shadow), multi-monitor, window rules, autostart, launcher (.desktop), X11 встраивание с GPU-ускорением (DRI3/DMA-BUF + hardware cursor + overlay planes), MCD-style glitch-анимации перехода между workspaces, live-reload конфигурации через inotify, IPC-сокет (i3-msg-совместимый протокол), полная xterm совместимость через libvterm, PipeWire звуком и xdg-desktop-portal для screen share.

---

## Содержание

- [Что нового в v0.5](#что-нового-в-v05)
- [Ключевые возможности](#ключевые-возможности)
- [Архитектура](#архитектура)
- [Быстрый старт](#быстрый-старт)
- [Установка](#установка)
- [Конфигурация (полный справочник)](#конфигурация-полный-справочник)
  - [Пути и приоритет](#пути-и-приоритет)
  - [\[general\] — общие настройки](#general--общие-настройки)
  - [\[theme\] — цвета интерфейса](#theme--цвета-интерфейса)
  - [\[login\] — экран входа](#login--экран-входа)
  - [\[\[workspaces\]\] — рабочие пространства](#workspaces--рабочие-пространства)
  - [\[\[monitors\]\] — мульти-монитор](#monitors--мульти-монитор)
  - [\[\[window_rules\]\] — правила размещения окон](#window_rules--правила-размещения-окон)
  - [\[\[autostart\]\] — автозапуск](#autostart--автозапуск)
  - [\[launcher\] — лаунчер приложений](#launcher--лаунчер-приложений)
  - [\[popups\] — попапы](#popups--попапы)
  - [\[\[keybindings\]\] — горячие клавиши](#keybindings--горячие-клавиши)
  - [\[live_reload\] — live-reload конфигурации](#live_reload--live-reload-конфигурации)
  - [\[animations\] — glitch-анимации MCD-style](#animations--glitch-анимации-mcd-style)
  - [\[ipc\] — IPC сокет (i3-msg совместимый)](#ipc--ipc-сокет-i3-msg-совместимый)
  - [\[x11\] — X11 встраивание и GPU-ускорение](#x11--x11-встраивание-и-gpu-ускорение)
  - [\[audio\] — звук (PipeWire)](#audio--звук-pipewire)
  - [\[portal\] — xdg-desktop-portal](#portal--xdg-desktop-portal)
  - [\[gamepad\] — геймпады](#gamepad--геймпады)
  - [Сводная таблица всех полей](#сводная-таблица-всех-полей)
- [Горячие клавиши (по умолчанию)](#горячие-клавиши-по-умолчанию)
- [Login screen](#login-screen)
- [Multi-monitor](#multi-monitor)
- [Window rules](#window-rules)
- [Autostart](#autostart)
- [Launcher](#launcher)
- [Popups из скриптов](#popups-из-скриптов)
- [Live reload конфигурации](#live-reload-конфигурации)
- [Glitch-анимации MCD-style](#glitch-анимации-mcd-style)
- [IPC сокет (i3-msg совместимый)](#ipc-сокет-i3-msg-совместимый-1)
- [Полная xterm совместимость (libvterm)](#полная-xterm-совместимость-libvterm)
- [X11 встраивание (Xephyr + DRI3 + overlay planes)](#x11-встраивание-xephyr--dri3--overlay-planes)
- [Звук и screen share](#звук-и-screen-share)
- [Геймпады](#геймпады)
- [Сборка пакетов](#сборка-пакетов)
- [Структура проекта](#структура-проекта)
- [История версий](#история-версий)
- [FAQ](#faq)
- [Устранение неполадок](#устранение-неполадок)
- [Лицензия](#лицензия)

---

## Что нового в v0.5

### Live reload конфигурации
- Inotify-наблюдатель на `config.toml` в отдельном потоке (через `libc::inotify_add_watch`)
- Debounce (`live_reload.debounce_ms`, по умолчанию 250 мс) для защиты от множественных событий
- Автоматическое применение изменений на лету (без перезапуска WM):
  - `theme.*` — пересборка `Theme` и немедленное использование при рендере
  - `keybindings` — новые биндинги активны сразу
  - `window_rules` — применяются к новым окнам
  - `animations.*` — параметры анимаций
  - `ipc.*` (кроме `socket_path`)
  - `live_reload.*`, `popups.*`, `general.glitch_intensity/framerate/gap/border`
- Поля, требующие перезапуска, логируются как warning:
  - `general.font`, `general.font_size`, `general.workspace_count`
  - `monitors`, `x11.display`, `x11.screen_size`, `x11.dri3`, `x11.overlay_planes`
- `ConfigDiff` — детектор изменений для отладки (логирует что именно поменялось)

### Glitch-анимации (MCD-style)
Три типа анимаций, все с corner-to-corner reveal (TL → BR диагональ):

**1. Workspace transition** — при переключении между workspaces:
- **Transition** (`ws_transition_ms`, по умолчанию 250 мс) — все символы экрана (терминальные ячейки, разделители, X11-окна как квадраты █▓) перебираются случайными символами
- **Manifest** (`ws_manifest_ms`, по умолчанию 200 мс) — поверх перебора проявляется целевой ws: добавляются недостающие символы, исчезают лишние. Перебор всё ещё идёт для остальных ячеек
- **Reveal** (`ws_reveal_ms`, по умолчанию 250 мс) — от левого верхнего угла к правому нижнему символы фиксируются в финальном состоянии

**2. New window** — при создании нового окна/терминала:
- **Fill** (`new_window_fill_ms`, по умолчанию 600 мс) — квадрат нового окна заливается перебором
- **Reveal** (`new_window_reveal_ms`, по умолчанию 250 мс) — corner-to-corner внутри rect окна

**3. Random glitch** — спонтанный быстрый глитч:
- Длительность `random_glitch_ms` (по умолчанию 120 мс — короче, чем ws transition)
- Частота = `general.glitch_intensity` / `random_glitch_every_frames`
- Скорость перебора `random_chars_per_sec` (по умолчанию 220 chars/sec — значительно быстрее)

Набор символов для перебора полностью настраивается:
- Заглавные A-Z (`glitch_use_alpha`)
- Квадраты с разной заливкой: ░ ▒ ▓ █ ■ □ ▢ ▣ ▤ ▥ ▦ ▧ ▨ ▩ ▀ ▄ ▌ ▐ (`glitch_use_blocks`)
- Цифры 0-9 (`glitch_use_digits`)

### IPC сокет (i3-msg совместимый)
- UNIX-доменный сокет (по умолчанию `$XDG_RUNTIME_DIR/superhot-tty.sock` или `/tmp/superhot-tty-$UID.sock`)
- JSON-протокол: команды и запросы
- Поддерживаемые команды (i3-msg синтаксис):
  - `workspace N|next|prev` — переключение ws
  - `move to workspace N` — переместить окно
  - `exec CMD [ARGS]`, `exec --no-startup-id CMD` — запуск программ
  - `kill`, `reload`, `restart`, `quit`, `split h|v`, `focus dir`, `fullscreen toggle`, `layout toggle`, `launcher`, `glitch`
- Запросы: `get_workspaces`, `get_config`, `get_focused`, `get_version`
- Standalone CLI утилита `shtty-msg` (второй binary в crate)

### Полная xterm совместимость (libvterm)
- Runtime FFI к `libvterm.so.0` через `libloading` (без жёсткой зависимости)
- Если libvterm доступна — используется полная xterm state machine (CSI/OSC/DCS, 256/truecolor, cursor styles, mouse tracking, DEC modes, OSC 4/8/52, ...)
- Если нет — fallback на расширенный built-in ANSI-парсер:
  - CSI: A-Z (cursor), J/K (erase), m (SGR), r (scroll region), L/M/P/@/S/T, s/u (save/restore), E/F/G/d, n (DSR)
  - Private modes: 1049/47/1047 (alt screen), 25 (cursor vis), 7 (autowrap), 1 (cursor keys), 12 (cursor blink)
  - ESC: D (IND), E (NEL), M (RI), c (RIS), 7/8 (DECSC/DECRC), =/> (keypad)
  - OSC: 0/2 (title), 4 (palette), 8 (hyperlink), 52 (clipboard)
  - SGR: bold/italic/underline/reverse, 16-color, 256-color, truecolor → 16-color палитра

### Прочее
- Версия bumped до `0.5.0`
- Добавлены секции конфига: `[live_reload]`, `[animations]`, `[ipc]`
- Добавлен второй binary `shtty-msg` (CLI для IPC)
- Обновлён README с полной документацией

---

## Ключевые возможности

### Менеджер окон
- **Тайловый layout** в стиле BSP/i3: бинарное дерево тайлов, split h/v, ratio resize
- **10 workspaces** (1..9 + 0=10) с независимыми layout-деревьями
- **Multi-monitor**: per-monitor workspace binding (как Hyprland)
- **Перемещение окон** по тайловой сетке и между workspaces
- **Fullscreen** toggle, **resize mode**, **cycle focus**
- **Live-reload** конфигурации (theme/keybindings/window_rules/animations/ipc) — без перезапуска

### Login screen
- **PAM аутентификация** (service "login") при сборке с `--features pam`
- **Fallback на `/etc/shadow` + `crypt(3)`** без PAM (по умолчанию)
- **MCD-themed**: большой заголовок по центру, glitch border, corner brackets
- **Настраиваемый текст**: "MORE" / "БОЛЬШЕ" / свой
- **Clock, hint, error display**

### Запуск программ
- **Rofi-подобный launcher** `Mod4+D` — читает `.desktop` файлы
- **Terminal=true** приложения открываются в нативном терминале
- **Window rules** для авто-placement по WM_CLASS/WM_NAME
- **Autostart** для запуска при старте WM

### X11 встраивание (GPU-ускорение)
- **Xephyr** на `:1` + **Composite redirect** + **XDamage**
- **DRI3 + DMA-BUF** для GPU-ускорения (FFI к `libxcb-dri3`)
- **Hardware DRM cursor plane** — 0% CPU, без перерисовки framebuffer
- **Hardware DRM overlay planes** — 0% CPU для X11 rendering
- **Auto-place**: новые X11 окна автоматически размещаются по правилам или на active ws

### Анимации MCD-style
- **Workspace transition** — glitch-перебор символов при переключении ws
- **New window** — заливка нового окна перебором + corner-to-corner reveal
- **Random glitch** — спонтанный быстрый глитч (настраиваемая вероятность)
- Все параметры (длительность фаз, набор символов, скорость перебора, цвет) настраиваются

### IPC сокет (i3-msg совместимый)
- UNIX-domain сокет, JSON-протокол
- CLI утилита `shtty-msg`
- Скриптование WM извне (workspace, exec, kill, reload, get_workspaces, ...)

### Терминал
- **PTY** через `libc::openpty` + `fork`/`execvp`
- **Полная xterm совместимость** через libvterm (если доступна)
- **Fallback ANSI-парсер** на ~95% случаев (если libvterm нет)
- **256-color + truecolor** поддержка (truecolor аппроксимируется в 16-color палитру)
- **OSC title** — синхронизация заголовка tile с `WM_NAME` приложения

### Ввод
- **Клавиатура** через evdev (эксклюзивный grab)
- **Мышь** через evdev + MCD-styled курсор (софтверный или hardware DRM cursor plane)
- **Геймпады**: evdev passthrough (Steam Input) + опционально SDL2

### Звук и screen share
- **PipeWire** + pipewire-pulse + wireplumber (автозапуск)
- **xdg-desktop-portal backend** для screen sharing в OBS/Discord

---

## Архитектура

```
+------------------------------------------------------------------------+
|                           superhot-tty v0.5                            |
|                                                                        |
|  1. LOGIN SCREEN (PAM)                                                 |
|     ┌─────────────────────────┐                                        |
|     │       БОЛЬШЕ / MORE     │                                        |
|     │   (themed MCD login)    │                                        |
|     │   login → password      │                                        |
|     └───────────┬─────────────┘                                        |
|                 │ PAM auth success                                     |
|                 ▼                                                      |
|  2. WM (after switch_to_user)                                          |
|     +----------+  +-----------+ +------------+ +--------------------+  |
|     | Keyboard |  |  Mouse    | | Gamepad    | |   Config (TOML)    |  |
|     | (evdev)  |  |  (evdev)  | | (SDL2/     | |   ~/.config/       |  |
|     |          |  |  +cursor  | |  passthru) | |   SH-tty/          |  |
|     +---+------+  +---+-------+ +--+---------+ +-----+--------------+  |
|          \            |           /                  |                 |
|     +-----v-----------v----------v-+      +----------v----------+      |
|     |     Window Rules Engine      |      |     Autostart       |      |
|     | (match WM_CLASS → placement) |      | (run on WM start)   |      |
|     +-------------+----------------+      +---------+-----------+      |
|                   |                                 |                  |
|     +-------------v---------------------------------v----------+       |
|     |     IPC Server ← shtty-msg     Animation Manager         |       |
|     |     (i3-msg JSON protocol)     (MCD glitch animations)   |       |
|     +-------------+-------------------------------------+------+       |
|                   |                                     |              |
|     +-------------v-------------------------------------v------+       |
|     |              Launcher + Layout + Workspaces              |       |
|     |       (.desktop scanner, BSP tree, 10 ws)                |       |
|     +-------------------------+--------------------------------+       |
|                               |                                        |
|                     +---------v---------+                              |
|                     | Canvas + Font     | ← live-reload                |
|                     | + VTerm (libvterm)|   theme/animations           |
|                     +---------+---------+                              |
|                               |                                        |
|                     +---------v---------+                              |
|                     |  Multi-Monitor    |  ← per-monitor CRTC          |
|                     |  DRM/KMS          |  ← per-monitor ws            |
|                     |  + DRI3 + Planes  |  ← GPU Aceleration for X11   |
|                     +-------------------+                              |
|                                                                        |
|  Watchers:                                                             |
|   • ConfigWatcher (inotify) → ConfigDiff → hot-apply                   |
|   • IPC listener thread → IpcRequest → execute                         |
|                                                                        |
|  Sidecar: Xephyr :1, PipeWire, xdg-desktop-portal                      |
+------------------------------------------------------------------------+
```

---

## Быстрый старт

```bash
# 1. Установить зависимости (Arch Linux)
sudo pacman -S rust cargo gcc pkgconf systemd kbd zsh pam \
              xorg-server-xephyr pipewire pipewire-pulse wireplumber \
              xdg-desktop-portal libvterm

# 2. Клонировать и собрать
git clone https://github.com/TumRedSun/SH-tty.git
cd SH-tty
cargo build --release

# 3. Установить
sudo install -Dm755 target/release/superhot-tty /usr/local/bin/
sudo install -Dm755 target/release/shtty-msg /usr/local/bin/
sudo install -Dm644 systemd/superhot-tty@.service /etc/systemd/system/
sudo install -Dm644 config/default.toml /etc/SH-tty/config.toml

# 4. Активировать (заменяет getty на tty1)
sudo systemctl disable getty@tty1
sudo systemctl enable superhot-tty@tty1
sudo reboot

# 5. После перезагрузки — themed login screen на tty1
#    Login → Password → shell
```

Для других дистрибутивов см. [Установка](#установка).

---

## Установка

### Вариант 1: Из пакета (рекомендуется)

Соберите пакеты (см. [Сборка пакетов](#сборка-пакетов)) или скачайте pre-built:

**Arch Linux:**
```bash
sudo pacman -U superhot-tty-0.5.0-1-x86_64.pkg.tar.zst
```

**Debian/Ubuntu:**
```bash
sudo dpkg -i superhot-tty_0.5.0_amd64.deb
sudo apt-get install -f  # разрешить зависимости
```

**Fedora/RHEL:**
```bash
sudo dnf install superhot-tty-0.5.0-1.x86_64.rpm
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
sudo install -Dm755 target/release/shtty-msg /usr/local/bin/
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

**Опциональные (но рекомендуются):**
```
libvterm                # полная xterm совместимость (fallback built-in парсер без неё)
libxcb-dri3             # DRI3 GPU-ускорение
sdl2                    # для --features gamepad-sdl2
libpam0g-dev            # для --features pam (иначе fallback на /etc/shadow)
xdg-desktop-portal-gtk  # GTK file chooser
```

### Cargo features

```bash
# По умолчанию — без PAM, без SDL2
cargo build --release

# С PAM (реальная PAM аутентификация вместо /etc/shadow)
cargo build --release --features pam

# С SDL2 геймпадом (для не-Steam сценария)
cargo build --release --features gamepad-sdl2

# Всё сразу
cargo build --release --features pam,gamepad-sdl2
```

---

## Конфигурация (полный справочник)

### Пути и приоритет

superhot-tty ищет конфиг в следующем порядке (первый найденный используется):

1. `$XDG_CONFIG_HOME/SH-tty/config.toml` (обычно `~/.config/SH-tty/config.toml`)
2. `~/.config/SH-tty/config.toml`
3. `/etc/SH-tty/config.toml` (system-wide)

Пример конфига со всеми секциями и комментариями — `config/default.toml` в репозитории.

Все секции имеют значения по умолчанию — пропущенные поля берутся из defaults. Можно иметь минимальный конфиг только с теми полями, которые нужно переопределить:

```toml
[general]
shell = "bash"

[login]
language = "ru"
```

### `[general]` — общие настройки

```toml
[general]
shell = "zsh"                    # shell для нативных терминалов
font = "Lat2-Terminus16"         # PSF-шрифт (PSF1/PSF2, ищется в /usr/share/kbd/consolefonts/)
font_size = 16                   # размер шрифта (для информации, реальный размер — из PSF)
gap = 4                          # зазор между тайлами в пикселях
border = 1                       # толщина бордюра неактивного тайла
outer_padding = 4                # внешний padding от краёв экрана
status_bar_height = 24           # высота статус-бара внизу
framerate = 60                   # целевой FPS (60 = 16.67 мс на кадр)
glitch_intensity = 0.15          # 0.0..1.0 — вероятность random glitch
workspace_count = 10             # 1..9 + 0=10 (как в i3)
```

**Поля, требующие перезапуска при изменении:**
- `font`, `font_size` — нужна новая загрузка PSF
- `workspace_count` — пересоздание структур Workspaces

**Hot-reload:** `gap`, `border`, `outer_padding`, `status_bar_height`, `framerate`, `glitch_intensity`, `shell`.

### `[theme]` — цвета интерфейса

Все цвета в hex-формате `#RRGGBB`.

```toml
[theme]
bg = "#0A0716"                # фон экрана (тёмно-фиолетовый)
tile_bg_inactive = "#120E24"  # фон неактивного тайла
tile_bg_active = "#0F0A1E"    # фон активного тайла
border_inactive = "#3A2D5C"   # бордюр неактивного тайла
border_active = "#FF2E97"     # бордюр активного тайла (магента)
border_x11 = "#00F0FF"        # бордюр X11 тайла (циан)
fg_default = "#E6E1F0"        # основной текст
fg_dim = "#7A6F96"            # приглушённый текст
accent_magenta = "#FF2E97"    # акцент магента
accent_cyan = "#00F0FF"       # акцент циан (используется для glitch_color)
popup_bg = "#140B2E"          # фон попапа
popup_border = "#FF2E97"      # бордюр попапа
error = "#FF4D4D"             # текст ошибок
```

**Hot-reload:** да, все поля — Theme пересобирается сразу.

### `[login]` — экран входа

```toml
[login]
title = ""                    # пусто → использовать language default
subtitle = ""                 # пусто → использовать language default
language = "en"               # "en" → "MORE" / "SUPERHOT TTY", "ru" → "БОЛЬШЕ" / "СУПЕРХОТ TTY"
show_clock = true             # показывать часы
show_hint = true              # показывать "Press Enter to login"
pam_service = "login"         # PAM service (если собрано с --features pam)
auto_start_session = true     # автоматически запускать WM после login
# title_color = "#FF2E97"     # optional override (default = theme.accent_magenta)
# title_font = ""             # optional override (default = general.font)
```

**Hot-reload:** да.

### `[[workspaces]]` — рабочие пространства

Список workspace'ов с именами (для отображения в статус-баре). Каждый — отдельный массив.

```toml
[[workspaces]]
n = 1
name = "MAIN"

[[workspaces]]
n = 2
name = "WEB"

[[workspaces]]
n = 3
name = "DEV"

[[workspaces]]
n = 4
name = "CHAT"

[[workspaces]]
n = 5
name = "MEDIA"

[[workspaces]]
n = 6
name = "6"

[[workspaces]]
n = 7
name = "7"

[[workspaces]]
n = 8
name = "8"

[[workspaces]]
n = 9
name = "9"

[[workspaces]]
n = 10
name = "MISC"
```

Число workspace'ов — `general.workspace_count` (по умолчанию 10). `[[workspaces]]` — метаданные (имена), не количество.

**Hot-reload:** да (имена обновляются в статус-баре).

### `[[monitors]]` — мульти-монитор

Привязка workspaces к конкретным мониторам (как в Hyprland).

```toml
[[monitors]]
connector = "eDP-1"             # имя коннектора DRM
workspaces = [1, 3, 5, 7, 9]   # нечётные ws на этом мониторе
enabled = true                  # false = отключить
position = "primary"            # "primary" / "left-of X" / "right-of X" / "above X" / "below X"
# resolution = [2560, 1440]    # опц.: пусто = preferred из EDID
# refresh_rate = 144           # опц.: пусто = default

[[monitors]]
connector = "HDMI-A-1"
workspaces = [2, 4, 6, 8, 10]  # чётные ws
enabled = true
position = "right-of eDP-1"
```

Если конфиг пустой — все workspaces на primary monitor.

Имена коннекторов (распространённые):
- `eDP-1` — встроенный экран ноутбука
- `HDMI-A-1`, `HDMI-B-1` — HDMI выходы
- `DP-1`, `DP-2` — DisplayPort
- `DVI-D-1` — DVI
- `VGA-1` — VGA

Посмотреть доступные: `cat /sys/class/drm/*/status` или в логе superhot-tty при старте.

**Hot-reload:** нет (требует перезапуска — reinit DRM/KMS).

### `[[window_rules]]` — правила размещения окон

Автоматическое размещение X11 окон по критериям (при CreateNotify).

Все непустые поля `match_*` должны совпасть (AND). Поле `regex = true` включает wildcard match (`*` и `?`).

```toml
[[window_rules]]
match_class = "firefox"          # WM_CLASS (instance)
# match_title = "Mozilla Firefox"  # WM_NAME (опц., AND)
# match_app_id = "org.mozilla.firefox"  # app_id (опц., AND)
regex = false                    # true → wildcard match
workspace = 2                    # поместить на workspace N
# monitor = "HDMI-A-1"           # или на конкретный монитор
size = [80, 80]                  # 80% ширины, 80% высоты (опц.)
position = [10, 10]              # 10% x, 10% y (опц.)
focus = true                     # сделать сфокусированным
fullscreen = false               # открыть в fullscreen
skip_auto_place = false          # не размещать автоматически
```

**Примеры:**
```toml
# Steam → MEDIA (ws 5)
[[window_rules]]
match_class = "Steam"
workspace = 5
focus = true

# Discord → CHAT (ws 4)
[[window_rules]]
match_class = "discord"
workspace = 4
focus = true

# Firefox → WEB (ws 2)
[[window_rules]]
match_class = "firefox"
workspace = 2

# GIMP — большой размер
[[window_rules]]
match_class = "Gimp"
workspace = 5
size = [80, 80]
position = [10, 10]
```

**Hot-reload:** да (применяется к новым окнам).

### `[[autostart]]` — автозапуск

Список команд, запускаемых при старте WM (после login), в отдельных потоках.

```toml
[[autostart]]
type = "command"           # фоновый процесс без UI
cmd = "pipewire"
delay_ms = 0               # задержка перед запуском

[[autostart]]
type = "command"
cmd = "pipewire-pulse"
delay_ms = 500

[[autostart]]
type = "command"
cmd = "wireplumber"
delay_ms = 1000

[[autostart]]
type = "x11"               # графическое приложение (DISPLAY=:1)
cmd = "firefox"
args = []
delay_ms = 2000
workspace = 2              # открыть на workspace 2

[[autostart]]
type = "x11"
cmd = "discord"
delay_ms = 3000
workspace = 4

[[autostart]]
type = "terminal"          # в нативном терминале (TERM=xterm-256color)
cmd = "htop"
delay_ms = 0
workspace = 1
```

Типы:
- `command` — фоновый процесс (pipewire, wireplumber, и т.д.)
- `x11` — графическое приложение (firefox, discord, steam)
- `terminal` — запустить команду в нашем нативном терминале (htop, vim, btop)

**Hot-reload:** нет (применяется только при старте WM).

### `[launcher]` — лаунчер приложений

```toml
[launcher]
max_rows = 12              # макс. число строк в списке
x11_display = ":1"         # DISPLAY для запуска X11 приложений
terminal_shell = "zsh"     # shell для терминальных приложений (Terminal=true)
desktop_paths = [
    "/usr/share/applications",
    "/usr/local/share/applications",
    "~/.local/share/applications",
]

[launcher.custom_entries]
"terminal: bash" = "bash"
"reload config" = "superhot-tty-reload"
"system status" = "neofetch"
```

Custom entries — дополнительные пункты в лаунчере (ключ — отображаемое имя, значение — команда).

**Hot-reload:** нет (применяется при старте WM).

### `[popups]` — попапы

```toml
[popups]
duration_frames = 240      # длительность показа (240 кадров при 60fps = 4 сек)
max_width_pct = 67         # макс. ширина в % от экрана
glitch_border = true       # RGB-сдвиг бордюра
# font = "Lat2-Terminus16" # опц. override (default = general.font)
```

**Hot-reload:** да.

### `[[keybindings]]` — горячие клавиши

Все биндинги в конфиге. Никаких захардкоженных.

```toml
[[keybindings]]
key = "d"                  # клавиша (a-z, 0-9, Return, Space, Tab, Left/Right/Up/Down, F1-F12, ...)
mods = ["Super"]           # модификаторы: Super, Ctrl, Alt, Shift
action = { type = "launcher" }
```

**Mods:** `Super`, `Ctrl`, `Alt`, `Shift` — комбинируются как массив. Все модификаторы должны быть нажаты (AND).

**Типы actions:**
- `launcher` — открыть лаунчер
- `terminal` — новый терминал
- `split_horizontal`, `split_vertical` — сплит текущего тайла
- `focus { dir = "left|right|up|down" }` — фокус в направлении
- `move { dir = "left|right|up|down" }` — переместить окно
- `swap { dir = "left|right|up|down" }` — swap с соседом
- `resize { dir = "left|right|up|down", delta = 0.05 }` — resize сплита
- `resize_mode` — войти/выйти из resize mode (HJKL)
- `workspace { n = 1 }` — переключиться на ws N
- `move_to_workspace { n = 1 }` — переместить окно на ws N
- `close` — закрыть тайл
- `fullscreen` — toggle fullscreen
- `cycle_focus` — цикл по всем тайлам
- `toggle_layout` — toggle layout (заглушка)
- `reload` — reload конфига (через ConfigWatcher)
- `quit` — выйти из WM
- `spawn { cmd = "...", args = [...] }` — запустить фоновый процесс
- `spawn_x11 { cmd = "...", args = [...] }` — запустить X11 приложение (DISPLAY=:1)
- `spawn_terminal { cmd = "...", args = [...] }` — запустить в нативном терминале
- `popup_script { cmd = "...", args = [...] }` — запустить скрипт, показать stdout в popup
- `popup { text = "..." }` — показать статичный popup
- `tab_next`, `tab_prev` — переключение табов (заглушки)

Примеры:
```toml
[[keybindings]]
key = "Return"
mods = ["Super"]
action = { type = "terminal" }

[[keybindings]]
key = "h"
mods = ["Super", "Shift"]
action = { type = "move", dir = "left" }

[[keybindings]]
key = "2"
mods = ["Super"]
action = { type = "workspace", n = 2 }

[[keybindings]]
key = "e"
mods = ["Super"]
action = { type = "spawn_x11", cmd = "xterm", args = [] }

[[keybindings]]
key = "p"
mods = ["Super"]
action = { type = "popup_script", cmd = "echo", args = ["Hello from SuperHot TTY!"] }
```

**Hot-reload:** да (новые биндинги активны сразу).

### `[live_reload]` — live-reload конфигурации

```toml
[live_reload]
enabled = true              # включить inotify watcher
debounce_ms = 250           # задержка перед перечитыванием (защита от множественных событий)
```

При изменении `config.toml` (через inotify IN_MODIFY/IN_CLOSE_WRITE/IN_MOVED_TO/IN_CREATE) — после debounce WM перечитывает файл и применяет изменения.

Логируется через `log::info!` что именно изменилось (theme / keybindings / window_rules / animations / ipc / live_reload / general / x11 / monitors).

Поля, требующие перезапуска, логируются как warning.

**Hot-reload:** да (можно включать/выключать watcher на лету).

### `[animations]` — glitch-анимации MCD-style

Полная настройка glitch-эффектов.

```toml
[animations]
# Включение/выключение анимаций
workspace_transition = true   # переход между ws
new_window = true             # появление нового окна
random_glitch = true          # случайные глитчи

# WS transition — три фазы:
ws_transition_ms = 250        # фаза 1: сплошной перебор всего экрана
ws_manifest_ms = 200          # фаза 2: целевой ws проявляется поверх перебора
ws_reveal_ms = 250            # фаза 3: corner-to-corner фиксация

# New window — две фазы:
new_window_fill_ms = 600      # фаза 1: квадрат окна заливается перебором
new_window_reveal_ms = 250    # фаза 2: corner-to-corner фиксация

# Random glitch:
random_glitch_ms = 120        # длительность (короче, чем ws transition)
random_glitch_every_frames = 360  # в среднем раз в N кадров (× glitch_intensity)

# Скорость перебора символов (chars/sec):
chars_per_sec = 60            # для ws transition и new window
random_chars_per_sec = 220    # для random glitch (значительно быстрее)

# Набор символов для перебора:
glitch_use_alpha = true       # A-Z (заглавные английские)
glitch_use_blocks = true      # ░ ▒ ▓ █ ■ □ ▢ ▣ ▤ ▥ ▦ ▧ ▨ ▩ ▀ ▄ ▌ ▐
glitch_use_digits = false     # 0-9

# Цвет символов глитча (hex):
# glitch_color = "#00F0FF"    # по умолчанию = theme.accent_cyan
```

**Подробнее о фазах workspace transition:**

1. **Transition** (`ws_transition_ms`) — все символы экрана (терминальные ячейки, разделители, X11-окна как квадраты █▓) перебираются случайными символами из набора (`glitch_use_alpha` + `glitch_use_blocks` + `glitch_use_digits`). Скорость перебора — `chars_per_sec`.

2. **Manifest** (`ws_manifest_ms`) — поверх перебора постепенно проявляется целевой ws. Ячейки целевого ws, в которых есть символ, заменяют glitch-символ. Ячейки, которых в целевом ws нет (но были в старом) — остаются в glitch (эффект "исчезновения"). Перебор для остальных ячеек продолжается.

3. **Reveal** (`ws_reveal_ms`) — diagonal corner-to-corner: от левого верхнего угла к правому нижнему символы фиксируются в финальном состоянии (terminal cells из целевого ws). Ячейка фиксируется если её нормализованное диагональное расстояние `(col/(cols-1) + row/(rows-1)) / 2` ≤ прогрессу reveal.

**Hot-reload:** да.

### `[ipc]` — IPC сокет (i3-msg совместимый)

```toml
[ipc]
enabled = true              # включить IPC сервер
# socket_path = "/run/user/1000/superhot-tty.sock"  # опц. (пусто = авто)
socket_mode = 0o600         # права на файл сокета (octal)
```

По умолчанию сокет:
1. `$XDG_RUNTIME_DIR/superhot-tty.sock` (обычно `/run/user/$UID/`)
2. `/tmp/superhot-tty-$UID.sock` (fallback)

**Hot-reload:** частично (`enabled` — да, `socket_path` — требует перезапуска).

См. подробности в [IPC сокет](#ipc-сокет-i3-msg-совместимый-1).

### `[x11]` — X11 встраивание и GPU-ускорение

```toml
[x11]
dri3 = true                 # DRI3 + DMA-BUF (FFI к libxcb-dri3)
display = ":1"              # display для Xephyr
screen_size = [1920, 1080]  # размер Xephyr screen
xtest_input = true          # XTest extension для ввода в X11 окна
hardware_cursor = true      # DRM cursor plane (0% CPU)
auto_place_windows = true   # авто-place X11 окон по правилам
overlay_planes = true       # DRM overlay planes (0% CPU для X11 rendering)
```

**Hot-reload:** нет (требует перезапуска — reinit Xephyr/DRM).

### `[audio]` — звук (PipeWire)

```toml
[audio]
start_pipewire_pulse = true  # запускать pipewire + pipewire-pulse при старте
start_wireplumber = true     # запускать wireplumber (session manager)
default_volume = 70          # начальная громкость (0-100)
```

**Hot-reload:** нет (применяется при старте WM).

### `[portal]` — xdg-desktop-portal

```toml
[portal]
start_portal = true                                            # запускать портал-бэкенд
service_name = "org.freedesktop.impl.portal.desktop.SuperHot"  # DBus имя
object_path = "/org/freedesktop/portal/desktop"                # DBus путь
```

**Hot-reload:** нет (применяется при старте WM).

### `[gamepad]` — геймпады

```toml
[gamepad]
enabled = true                # обрабатывать геймпад
steam_passthrough = true      # evdev passthrough для Steam Input
stick_sensitivity = 50        # чувствительность стика (1-100)

[gamepad.keymap]
"a" = "Return"
"b" = "Escape"
"x" = "space"
"y" = "Tab"
"dpad_up" = "k"
"dpad_down" = "j"
"dpad_left" = "h"
"dpad_right" = "l"
"start" = "Super"
"back" = "Super"
"left_shoulder" = "bracketleft"
"right_shoulder" = "bracketright"
```

**Hot-reload:** частично.

### Сводная таблица всех полей

| Секция | Поле | Тип | По умолчанию | Hot-reload | Описание |
|--------|------|-----|--------------|------------|----------|
| `general` | `shell` | string | `"zsh"` | ✅ | shell для терминалов |
| `general` | `font` | string | `"Lat2-Terminus16"` | ❌ | PSF-шрифт |
| `general` | `font_size` | u32 | `16` | ❌ | размер шрифта |
| `general` | `gap` | i32 | `4` | ✅ | зазор между тайлами |
| `general` | `border` | i32 | `1` | ✅ | толщина бордюра |
| `general` | `outer_padding` | i32 | `4` | ✅ | внешний padding |
| `general` | `status_bar_height` | u32 | `24` | ✅ | высота статус-бара |
| `general` | `framerate` | u32 | `60` | ✅ | целевой FPS |
| `general` | `glitch_intensity` | f32 | `0.15` | ✅ | вероятность random glitch (0..1) |
| `general` | `workspace_count` | u8 | `10` | ❌ | число workspaces |
| `theme` | (12 colors) | string hex | MCD-палитра | ✅ | цвета интерфейса |
| `login` | `title` | string | `""` (auto) | ✅ | заголовок login screen |
| `login` | `subtitle` | string | `""` (auto) | ✅ | подзаголовок |
| `login` | `language` | string | `"en"` | ✅ | "en" / "ru" |
| `login` | `show_clock` | bool | `true` | ✅ | показывать часы |
| `login` | `show_hint` | bool | `true` | ✅ | подсказка "Press Enter" |
| `login` | `pam_service` | string | `"login"` | ✅ | PAM service |
| `login` | `auto_start_session` | bool | `true` | ✅ | авто-запуск WM |
| `workspaces` | `n` | u8 | — | ✅ | номер ws |
| `workspaces` | `name` | string | — | ✅ | имя ws |
| `monitors` | `connector` | string | — | ❌ | имя коннектора DRM |
| `monitors` | `workspaces` | u8[] | — | ❌ | список ws на мониторе |
| `monitors` | `enabled` | bool | `true` | ❌ | вкл/выкл |
| `monitors` | `resolution` | (u32,u32) | `None` | ❌ | разрешение |
| `monitors` | `refresh_rate` | u32 | `None` | ❌ | частота |
| `monitors` | `position` | string | `None` | ❌ | позиционирование |
| `window_rules` | `match_class` | string | — | ✅ | WM_CLASS match |
| `window_rules` | `match_title` | string | — | ✅ | WM_NAME match |
| `window_rules` | `match_app_id` | string | — | ✅ | app_id match |
| `window_rules` | `regex` | bool | `false` | ✅ | wildcard/regex |
| `window_rules` | `workspace` | u8 | `None` | ✅ | target ws |
| `window_rules` | `monitor` | string | `None` | ✅ | target monitor |
| `window_rules` | `size` | (u32,u32) | `None` | ✅ | размер в % |
| `window_rules` | `position` | (u32,u32) | `None` | ✅ | позиция в % |
| `window_rules` | `focus` | bool | `true` | ✅ | сделать сфокусированным |
| `window_rules` | `fullscreen` | bool | `false` | ✅ | открыть в fullscreen |
| `window_rules` | `skip_auto_place` | bool | `false` | ✅ | не размещать авто |
| `autostart` | `type` | string | — | ❌ | command/x11/terminal |
| `autostart` | `cmd` | string | — | ❌ | команда |
| `autostart` | `args` | string[] | `[]` | ❌ | аргументы |
| `autostart` | `delay_ms` | u64 | `0` | ❌ | задержка |
| `autostart` | `workspace` | u8 | `None` | ❌ | target ws |
| `autostart` | `monitor` | string | `None` | ❌ | target monitor |
| `launcher` | `max_rows` | u32 | `12` | ❌ | макс. строк |
| `launcher` | `x11_display` | string | `":1"` | ❌ | DISPLAY |
| `launcher` | `terminal_shell` | string | `"zsh"` | ❌ | shell для Terminal=true |
| `launcher` | `desktop_paths` | string[] | стандартные | ❌ | пути .desktop |
| `launcher.custom_entries` | (key,value) | string→string | `{}` | ❌ | доп. пункты |
| `popups` | `duration_frames` | u32 | `240` | ✅ | длительность показа |
| `popups` | `max_width_pct` | u32 | `67` | ✅ | макс. ширина % |
| `popups` | `glitch_border` | bool | `true` | ✅ | RGB-сдвиг бордюра |
| `popups` | `font` | string? | `None` | ✅ | override шрифта |
| `live_reload` | `enabled` | bool | `true` | ✅ | вкл inotify watcher |
| `live_reload` | `debounce_ms` | u64 | `250` | ✅ | debounce |
| `animations` | `workspace_transition` | bool | `true` | ✅ | анимация перехода ws |
| `animations` | `new_window` | bool | `true` | ✅ | анимация нового окна |
| `animations` | `random_glitch` | bool | `true` | ✅ | случайные глитчи |
| `animations` | `ws_transition_ms` | u32 | `250` | ✅ | фаза 1 ws transition |
| `animations` | `ws_manifest_ms` | u32 | `200` | ✅ | фаза 2 ws transition |
| `animations` | `ws_reveal_ms` | u32 | `250` | ✅ | фаза 3 ws transition |
| `animations` | `new_window_fill_ms` | u32 | `600` | ✅ | фаза 1 new window |
| `animations` | `new_window_reveal_ms` | u32 | `250` | ✅ | фаза 2 new window |
| `animations` | `random_glitch_ms` | u32 | `120` | ✅ | длительность random |
| `animations` | `random_glitch_every_frames` | u32 | `360` | ✅ | частота random |
| `animations` | `chars_per_sec` | u32 | `60` | ✅ | скорость перебора |
| `animations` | `random_chars_per_sec` | u32 | `220` | ✅ | скорость random |
| `animations` | `glitch_use_alpha` | bool | `true` | ✅ | A-Z в переборе |
| `animations` | `glitch_use_blocks` | bool | `true` | ✅ | блоки в переборе |
| `animations` | `glitch_use_digits` | bool | `false` | ✅ | цифры в переборе |
| `animations` | `glitch_color` | string? | `None` | ✅ | цвет глитча (default accent_cyan) |
| `ipc` | `enabled` | bool | `true` | ✅ | вкл IPC сервер |
| `ipc` | `socket_path` | string? | `None` | ❌ | путь сокета |
| `ipc` | `socket_mode` | u32 | `0o600` | ✅ | права сокета |
| `x11` | `dri3` | bool | `true` | ❌ | DRI3 + DMA-BUF |
| `x11` | `display` | string | `":1"` | ❌ | display Xephyr |
| `x11` | `screen_size` | (u16,u16) | `(1920,1080)` | ❌ | размер Xephyr |
| `x11` | `xtest_input` | bool | `true` | ❌ | XTest extension |
| `x11` | `hardware_cursor` | bool | `true` | ❌ | DRM cursor plane |
| `x11` | `auto_place_windows` | bool | `true` | ❌ | авто-place X11 |
| `x11` | `overlay_planes` | bool | `true` | ❌ | DRM overlay planes |
| `audio` | `start_pipewire_pulse` | bool | `true` | ❌ | запуск PipeWire |
| `audio` | `start_wireplumber` | bool | `true` | ❌ | запуск wireplumber |
| `audio` | `default_volume` | u32 | `70` | ❌ | начальная громкость |
| `portal` | `start_portal` | bool | `true` | ❌ | запуск портала |
| `portal` | `service_name` | string | `"...SuperHot"` | ❌ | DBus имя |
| `portal` | `object_path` | string | `"/org/..."` | ❌ | DBus путь |
| `gamepad` | `enabled` | bool | `true` | ✅ | обработка геймпада |
| `gamepad` | `steam_passthrough` | bool | `true` | ✅ | evdev passthrough |
| `gamepad` | `stick_sensitivity` | u32 | `50` | ✅ | чувствительность стика |
| `gamepad.keymap` | (key,value) | string→string | defaults | ✅ | маппинг кнопок |

---

## Горячие клавиши (по умолчанию)

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

Все биндинги можно переназначить или добавить свои — см. [`[[keybindings]]`](#keybindings--горячие-клавиши).

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
3. PAM аутентификация через service `login` (если собрано с `--features pam`)
4. Fallback: проверка через `/etc/shadow` + `crypt(3)`
5. При успехе: переключение на UID/GID пользователя, загрузка user config, запуск WM
6. При ошибке: показ сообщения, возврат к Welcome

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

## Live reload конфигурации

Inotify-наблюдатель на `config.toml` (через `libc::inotify_add_watch` в отдельном потоке).

При изменении файла — debounce (`live_reload.debounce_ms`, по умолчанию 250 мс), затем перечитывание.

### Применяется на лету (без перезапуска WM)

- `theme.*` — пересобирается `Theme` и сразу используется при рендере
- `keybindings` — новые биндинги активны сразу
- `window_rules` — применяются к новым окнам
- `animations.*` — параметры анимаций
- `ipc.*` (кроме `socket_path` — требует перезапуска)
- `live_reload.*`
- `general.glitch_intensity`, `general.framerate`, `general.gap/border/outer_padding/status_bar_height`
- `popups.*`

### Требуют перезапуска (логируется как warning)

- `general.font`, `general.font_size` — нужны новая загрузка PSF
- `general.workspace_count` — пересоздание структур Workspaces
- `monitors` — reinit DRM/KMS
- `x11.display`, `x11.screen_size` — reinit Xephyr
- `x11.dri3`, `x11.overlay_planes` — reinit DRM planes

### Логирование

В логе (например, `journalctl -u superhot-tty@tty1`) видно:
```
INFO  config change detected, reloading...
INFO  config diff: {"theme": true, "keybindings": false, "window_rules": false, ...}
INFO    → theme reloaded
```

`src/config/watcher.rs` — `ConfigWatcher` + `ConfigDiff` (детектор изменений).

---

## Glitch-анимации MCD-style

Три типа анимаций, все с corner-to-corner reveal (TL → BR диагональ).

### 1. Workspace transition (`animations.workspace_transition = true`)

При переключении workspaces (через `Mod4+N`, IPC `workspace N`, или window rule):

**Фаза 1: Transition** (`ws_transition_ms`, по умолчанию 250 мс)
- Все символы экрана (терминальные ячейки, разделители, X11-окна как квадраты █▓) начинают перебираться случайными символами:
  - Заглавные A-Z (если `glitch_use_alpha = true`)
  - Квадраты с разной заливкой: ░ ▒ ▓ █ ■ □ ▢ ▣ ▤ ▥ ▦ ▧ ▨ ▩ ▀ ▄ ▌ ▐ (если `glitch_use_blocks = true`)
  - Опционально цифры 0-9 (`glitch_use_digits`)
- Скорость перебора: `chars_per_sec` (по умолчанию 60 chars/sec)
- Каждая ячейка имеет свой псевдо-случайный сдвиг фазы, чтобы ячейки менялись не одновременно

**Фаза 2: Manifest** (`ws_manifest_ms`, по умолчанию 200 мс)
- Поверх перебора проявляется целевой ws:
  - Ячейки целевого ws, в которых есть символ, заменяют glitch-символ (появление)
  - Ячейки, которых в целевом ws нет (но были в старом) — остаются в glitch (исчезновение)
- Перебор для остальных ячеек продолжается
- Прогресс — от 0% до 100% за `ws_manifest_ms`

**Фаза 3: Reveal** (`ws_reveal_ms`, по умолчанию 250 мс)
- Diagonal corner-to-corner: от левого верхнего угла к правому нижнему символы фиксируются в финальном состоянии (terminal cells из целевого ws)
- Ячейка фиксируется если её нормализованное диагональное расстояние `(col/(cols-1) + row/(rows-1)) / 2` ≤ прогрессу reveal
- Создаёт эффект "волны" от TL к BR

### 2. New window (`animations.new_window = true`)

При создании нового X11-окна или нативного терминального tile:

**Фаза 1: Fill** (`new_window_fill_ms`, по умолчанию 600 мс)
- Квадрат нового окна заливается перебором (фон глитча — тёмный, символы glitch_color)
- Реальное содержимое окна скрыто

**Фаза 2: Reveal** (`new_window_reveal_ms`, по умолчанию 250 мс)
- Corner-to-corner внутри rect окна: символы фиксируются, видно реальный терминал/окно
- Тот же diagonal-алгоритм, что и у ws transition

### 3. Random glitch (`animations.random_glitch = true`)

Спонтанный быстрый глитч по случайному под-прямоугольнику экрана:

- **Длительность:** `random_glitch_ms` (по умолчанию 120 мс — короче, чем ws transition)
- **Частота:** в среднем раз в `random_glitch_every_frames` кадров (по умолчанию 360), умножается на `general.glitch_intensity` (0.0..1.0, по умолчанию 0.15)
  - Формула: `P(glitch this frame) = glitch_intensity / random_glitch_every_frames`
  - При defaults: 0.15 / 360 ≈ 0.00042 на кадр, ~раз в 40 секунд при 60 FPS
- **Скорость перебора:** `random_chars_per_sec` (по умолчанию 220 chars/sec — значительно быстрее, чем у ws/new_window)
- **Corner-to-corner reveal** как и у остальных, но быстрый
- **Цвета:** чередующиеся glitch_color и accent_magenta (для мигающего эффекта)

### Настройка цвета

```toml
[animations]
# glitch_color = "#00F0FF"   # default = theme.accent_cyan
```

### Триггер вручную через IPC

```bash
shtty-msg "glitch"
```

Запускает random glitch немедленно (вероятность 100%).

### Внутреннее устройство

`src/render/glitch.rs`:
- `AnimationManager` — менеджер всех активных анимаций
- `ActiveAnimation` — состояние одной анимации (kind, started, total_duration, snapshots)
- `CharSnapshot` — снимок экрана как сетка символов (для ws transition: old + new)
- `snapshot_workspace()` — делает снимок текущего workspace (терминалы + X11 как квадраты)
- `random_glitch_char()` — случайный символ из набора (A-Z + блоки + опц. цифры)

---

## IPC сокет (i3-msg совместимый)

UNIX-доменный сокет (по умолчанию `$XDG_RUNTIME_DIR/superhot-tty.sock` или `/tmp/superhot-tty-$UID.sock`). Протокол: JSON-запросы и JSON-ответы.

### Запросы

```json
{ "type": "command", "cmd": "workspace 2" }
{ "type": "command", "cmd": "workspace next" }
{ "type": "command", "cmd": "workspace prev" }
{ "type": "command", "cmd": "move to workspace 3" }
{ "type": "command", "cmd": "exec firefox" }
{ "type": "command", "cmd": "exec --no-startup-id alacritty" }
{ "type": "command", "cmd": "exec discord -- --gpu-rasterization" }
{ "type": "command", "cmd": "kill" }
{ "type": "command", "cmd": "reload" }
{ "type": "command", "cmd": "restart" }
{ "type": "command", "cmd": "quit" }
{ "type": "command", "cmd": "split horizontal" }
{ "type": "command", "cmd": "split vertical" }
{ "type": "command", "cmd": "focus left" }
{ "type": "command", "cmd": "focus right" }
{ "type": "command", "cmd": "focus up" }
{ "type": "command", "cmd": "focus down" }
{ "type": "command", "cmd": "fullscreen toggle" }
{ "type": "command", "cmd": "layout toggle" }
{ "type": "command", "cmd": "launcher" }
{ "type": "command", "cmd": "glitch" }
{ "type": "get_workspaces" }
{ "type": "get_config" }
{ "type": "get_focused" }
{ "type": "get_version" }
```

### Ответы

```json
{ "status": "ok", "result": "switched to workspace 2" }
{ "status": "ok", "result": "[{\"num\":1,\"name\":\"MAIN\",\"current\":true,\"tiles\":2}, ...]" }
{ "status": "ok", "result": "{\"name\":\"superhot-tty\",\"version\":\"0.5.0\",\"libvterm\":true}" }
{ "status": "error", "error": "unknown command: foo" }
{ "status": "error", "error": "workspace requires argument" }
```

### CLI утилита `shtty-msg`

Standalone binary (не зависит от основного crate):

```bash
# Команды
shtty-msg "workspace 2"
shtty-msg "workspace next"
shtty-msg "move to workspace 3"
shtty-msg "exec firefox"
shtty-msg "exec --no-startup-id alacritty"
shtty-msg "kill"
shtty-msg "reload"
shtty-msg "split vertical"
shtty-msg "focus left"
shtty-msg "fullscreen toggle"
shtty-msg "launcher"
shtty-msg "glitch"

# Запросы
shtty-msg --get-workspaces
shtty-msg --get-config
shtty-msg --get-focused
shtty-msg --get-version

# Помощь
shtty-msg
shtty-msg --help
```

### Пример: скрипт "переключить на следующий ws и запустить терминал"

```bash
#!/bin/bash
shtty-msg "workspace next"
sleep 0.3
shtty-msg "exec alacritty"
```

### Пример: i3status-rs совместимый скрипт

```bash
#!/bin/bash
# Получить список workspaces и текущий
WS=$(shtty-msg --get-workspaces | jq -r '.result' | jq '.[] | select(.current==true) | .name')
echo "Workspace: $WS"
```

### Права сокета

`ipc.socket_mode` (по умолчанию `0o600`, только владелец). Для разделения между пользователями — создайте группу и поменяйте права.

### Внутреннее устройство

- `src/ipc/mod.rs` — `IpcServer`, `IpcRequest`, `IpcResponse`, `parse_i3_command()`, `shell_split()`
- `src/bin/shtty_msg.rs` — standalone CLI binary (без зависимости от основного crate)
- IPC сервер запускается в отдельном потоке, главный цикл WM опрашивает `try_recv()` каждый кадр

---

## Полная xterm совместимость (libvterm)

Runtime FFI к `libvterm.so.0` через `libloading`. Если библиотека доступна — VTerm проксирует туда все байты от PTY и синхронизирует grid. Если нет — fallback на расширенный built-in ANSI-парсер.

### Проверка доступности libvterm

```bash
shtty-msg --get-version
# {"status":"ok","result":"{\"name\":\"superhot-tty\",\"version\":\"0.5.0\",\"libvterm\":true}"}
```

`"libvterm":true` — используется libvterm, `"libvterm":false` — fallback built-in парсер.

Или в логе при старте WM:
```
INFO  libvterm backend active (full xterm compatibility)
# или
INFO  libvterm not available — using built-in minimal ANSI parser
```

### Установка libvterm

**Arch Linux:**
```bash
sudo pacman -S libvterm
```

**Debian/Ubuntu:**
```bash
sudo apt install libvterm-dev
```

**Fedora:**
```bash
sudo dnf install libvterm-devel
```

### Built-in парсер (fallback)

Если libvterm недоступна, используется расширенный built-in парсер. Покрывает:

**CSI sequences:**
- `H`/`f` — cursor position (CSI n;m H)
- `A`/`B`/`C`/`D` — cursor up/down/fwd/back
- `E`/`F` — CNL/CPL (cursor next/prev line)
- `G`/`d` — column/row set
- `J` — erase display (0=cursor-to-end, 1=start-to-cursor, 2=all, 3=scrollback)
- `K` — erase line (0/1/2)
- `m` — SGR (colors, bold, italic, underline, reverse, ...)
- `r` — DECSTBM (scroll region)
- `n` — device status report (CSI 6n → cursor position)
- `L`/`M` — insert/delete lines
- `P` — delete chars
- `@` — insert blanks
- `S`/`T` — scroll up/down
- `s`/`u` — save/restore cursor (ANSI.SYS)
- `h`/`l` — ANSI modes (7=autowrap, 4=insert)

**Private modes (CSI ?):**
- `1049` — alt screen (xterm)
- `47`/`1047` — alt screen (older variants)
- `25` — cursor visibility
- `7` — autowrap
- `1` — application cursor keys
- `12` — cursor blink

**ESC sequences:**
- `D` — IND (index, down + scroll if needed)
- `E` — NEL (next line)
- `M` — RI (reverse index)
- `c` — RIS (full reset)
- `7`/`8` — DECSC/DECRC (save/restore cursor + attrs)
- `=`/`>` — application/normal keypad
- `P` — DCS (Device Control String, игнорируется до ST)
- `\\` — ST (String Terminator)

**OSC sequences:**
- `0;title` / `2;title` — window title (синхронизируется с заголовком tile)
- `4;N;COLOR` — palette set (игнорируется, у нас 16-color палитра)
- `8;params;uri` — hyperlink (игнорируется)
- `52;clipboard` — clipboard (игнорируется)

**SGR (Select Graphic Rendition):**
- `0` — reset all attrs
- `1`/`22` — bold on/off
- `3`/`23` — italic on/off
- `4`/`24` — underline on/off
- `7`/`27` — reverse on/off
- `30-37` / `40-47` — 16-color fg/bg
- `38;5;N` / `48;5;N` — 256-color
- `38;2;R;G;B` / `48;2;R;G;B` — truecolor (аппроксимируется в ближайший 16-color)
- `90-97` / `100-107` — bright fg/bg

### Внутреннее устройство

- `src/term/libvterm.rs` — `LibVTermHandle`, FFI bindings через `libloading`
- `src/term/vterm.rs` — `VTerm` с опциональным libvterm backend
- При `VTerm::new(cols, rows)` — пытается загрузить libvterm; если успешно, использует её
- При `feed()` — если libvterm активна, проксирует байты туда и синхронизирует grid
- При недоступности — встроенный state machine (Ground/Esc/Csi/CsiPrivate/CsiGt/Osc/Dcs)

---

## X11 встраивание (Xephyr + DRI3 + overlay planes)

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

## Звук и screen share

### PipeWire

superhot-tty запускает (если `[audio]` включён):
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
- `superhot-tty-0.5.0-1-x86_64.pkg.tar.zst` (Arch)
- `superhot-tty_0.5.0_amd64.deb` (Debian)
- `superhot-tty-0.5.0-1.x86_64.rpm` (Fedora)
- `superhot-tty-packages-0.5.0.tar.gz` (все 3 в одном архиве)

### Требования для сборки

- `cargo`, `rust` — для бинарника
- `makepkg` (Arch) или `cargo-deb` (Debian) или `cargo-rpm` (Fedora)
- Или `fpm` как универсальный fallback

---

## Структура проекта

```
superhot-tty/
├── Cargo.toml                  # manifest (2 binary: superhot-tty, shtty-msg)
├── build-packages.sh           # генерация 3 пакетов
├── README.md
├── LICENSE
├── config/
│   └── default.toml            # полный пример конфига со всеми секциями
├── skel/
│   └── zshrc.example           # пример .zshrc с MCD-стилем
├── systemd/
│   └── superhot-tty@.service
├── packaging/
│   ├── arch/
│   │   ├── PKGBUILD
│   │   └── superhot-tty.install
│   ├── debian/                 # (используется cargo-deb)
│   └── rpm/
│       └── superhot-tty.spec
├── debian/
│   ├── control
│   ├── postinst
│   └── prerm
└── src/
    ├── main.rs                 # entry: login → WM flow, IPC handler
    ├── login/mod.rs            # PAM login screen
    ├── config/
    │   ├── mod.rs              # TOML config (все секции v0.5)
    │   ├── window_rules.rs     # window rules engine
    │   └── watcher.rs          # live-reload inotify watcher (NEW v0.5)
    ├── launcher/mod.rs         # rofi-like launcher (.desktop scanner)
    ├── drm/
    │   ├── kms.rs              # DRM/KMS ioctls
    │   ├── multi_monitor.rs    # multi-monitor backend
    │   ├── cursor.rs           # hardware DRM cursor plane
    │   ├── planes.rs           # hardware DRM overlay planes (0% CPU X11)
    │   └── fbdev.rs            # legacy fallback
    ├── render/
    │   ├── canvas.rs           # direct framebuffer canvas
    │   ├── font.rs             # PSF1/PSF2 font loader
    │   ├── text.rs             # text renderer
    │   └── glitch.rs           # MCD-style glitch animations (NEW v0.5)
    ├── term/
    │   ├── pty.rs              # PTY (openpty + fork/execvp)
    │   ├── vterm.rs            # built-in ANSI parser (extended v0.5)
    │   └── libvterm.rs         # libvterm FFI bindings (NEW v0.5)
    ├── layout/
    │   ├── mod.rs              # BSP/i3 tile tree
    │   └── workspaces.rs       # 10 workspaces
    ├── input/
    │   ├── keyboard.rs         # evdev keyboard
    │   ├── mouse.rs            # evdev mouse + software cursor
    │   └── gamepad.rs          # evdev/SDL2 gamepad
    ├── x11/
    │   ├── compositor.rs       # Xephyr + Composite + Damage
    │   ├── dri3.rs             # DRI3/DMA-BUF FFI (libloading libxcb-dri3)
    │   └── dmabuf.rs           # DMA-BUF helpers
    ├── ipc/
    │   └── mod.rs              # i3-msg-compatible IPC socket (NEW v0.5)
    ├── bin/
    │   └── shtty_msg.rs        # standalone IPC CLI binary (NEW v0.5)
    ├── audio/mod.rs            # PipeWire launcher
    ├── portal/mod.rs           # xdg-desktop-portal backend
    └── ui/
        ├── theme.rs            # MCD color palette (16 ANSI colors)
        ├── popup.rs            # MCD-styled popups
        └── mod.rs
```

---

## История версий

### v0.5 (текущая)
- ✅ **Live reload конфигурации** — inotify watcher на `config.toml`, debounce, hot-apply theme/keybindings/window_rules/animations/ipc, warn для перезапускаемых полей — `src/config/watcher.rs`
- ✅ **Анимации перехода между workspaces (glitch MCD-style)** — три фазы: transition (перебор) → manifest (проявление нового ws) → reveal (corner-to-corner фиксация). Плюс new-window анимация и random glitch — `src/render/glitch.rs`
- ✅ **IPC сокет (i3-msg-совместимый протокол)** — UNIX-domain сокет, JSON-команды, CLI `shtty-msg` — `src/ipc/mod.rs`, `src/bin/shtty_msg.rs`
- ✅ **Полная xterm совместимость (libvterm)** — runtime FFI к `libvterm.so.0` через `libloading`, fallback на расширенный built-in парсер (CSI/OSC/DCS, SGR truecolor, DEC modes, save/restore cursor, DECSC/DECRC) — `src/term/libvterm.rs`, `src/term/vterm.rs`

### v0.4
- ✅ **Полная DRI3/DMA-BUF реализация (FFI к xcb-dri3)** — `src/x11/dri3.rs`
- ✅ **Hardware DRM cursor plane** — `src/drm/cursor.rs`
- ✅ **Hardware DRM overlay planes (0% CPU для X11)** — `src/drm/planes.rs`

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

### v0.2
- ✅ Initial superhot-tty: tile tree, DRM/KMS direct, MCD theme

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

### Q: Live reload не работает
**A:** Проверьте:
1. `[live_reload] enabled = true` в конфиге
2. Файл, который вы редактируете, — тот, что загрузился (см. лог: `loaded config from ...`)
3. `live_reload.debounce_ms` не слишком большой
4. В логе: `config change detected, reloading...`

### Q: IPC не работает — shtty-msg не подключается
**A:**
1. `[ipc] enabled = true`
2. Проверьте путь сокета: `shtty-msg` (без аргументов показывает путь)
3. В логе: `IPC socket listening at ...`
4. Проверьте права: `ls -la $XDG_RUNTIME_DIR/superhot-tty.sock`

### Q: Glitch-анимации тормозят
**A:** Уменьшите:
- `animations.chars_per_sec` (по умолчанию 60 — попробуйте 30)
- `animations.ws_transition_ms` / `ws_manifest_ms` / `ws_reveal_ms`
- Выключите random glitch: `animations.random_glitch = false`
- Уменьшите `general.glitch_intensity` (0.0 = выключено)

### Q: libvterm не загружается
**A:** Проверьте:
1. `ldconfig -p | grep vterm` — установлена ли библиотека
2. `shtty-msg --get-version` — `"libvterm":true/false`
3. В логе: `libvterm backend active` или `libvterm not available`
4. Установите: `sudo pacman -S libvterm` / `sudo apt install libvterm-dev` / `sudo dnf install libvterm-devel`

### Q: Как откатиться к стандартному getty?
```bash
sudo systemctl disable superhot-tty@tty1
sudo systemctl enable getty@tty1
sudo systemctl daemon-reload
sudo reboot
```

### Q: Как сделать свой glitch-цвет?
**A:**
```toml
[animations]
glitch_color = "#FF00FF"   # любой hex
```

### Q: Как полностью выключить анимации?
**A:**
```toml
[animations]
workspace_transition = false
new_window = false
random_glitch = false
```

### Q: Можно ли использовать superhot-tty как обычный WM в X11?
**A:** Нет. superhot-tty — это замена `agetty`, работает напрямую с DRM/KMS (без X server на основном экране). X11 используется только через Xephyr для совместимости с X-приложениями.

---

## Устранение неполадок

### Логи

```bash
# Systemd логи superhot-tty
journalctl -u superhot-tty@tty1 -f

# Логи текущей сессии
journalctl -u superhot-tty@tty1 --since "1 hour ago"

# Фильтр по уровню
journalctl -u superhot-tty@tty1 -p err
```

### Отладка

```bash
# Проверить DRM устройства
ls -la /dev/dri/
cat /sys/class/drm/*/status

# Проверить коннекторы
cat /sys/class/drm/card0-*/status

# Проверить Xephyr
DISPLAY=:1 xdpyinfo | head

# Проверить IPC сокет
ls -la $XDG_RUNTIME_DIR/superhot-tty.sock
shtty-msg --get-version

# Проверить libvterm
ldconfig -p | grep vterm
```

### Частые проблемы

| Симптом | Причина | Решение |
|---------|---------|---------|
| Чёрный экран после reboot | DRM не получил master | Проверить лог: `journalctl -u superhot-tty@tty1` |
| Login не работает | PAM не настроен | Установить libpam0g-dev, собрать с `--features pam` |
| Окна X11 не появляются | Xephyr не запущен | Проверить: `pgrep Xephyr` |
| Нет звука | PipeWire не стартовал | `systemctl --user status pipewire` |
| Скриншот не работает | Portal не зарегистрирован | Проверить: `dbus-send --session --print-reply --dest=org.freedesktop.DBus / org.freedesktop.DBus.ListNames` |
| Glitch-анимации не работают | Выключены в конфиге | Проверить `[animations]` секцию |
| IPC не отвечает | Сокет не создан | Проверить `[ipc] enabled = true` и логи |
| Live reload не работает | Watcher выключен | Проверить `[live_reload] enabled = true` |
| Терминал глючит с цветами | libvterm недоступна | Установить libvterm |

---

## Лицензия

MIT License. См. [LICENSE](LICENSE).
