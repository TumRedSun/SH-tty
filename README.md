# superhot-tty

**Тайловый оконный менеджер для Linux-консоли в эстетике SuperHot: Mind Control Delete**

> Это не X11, не Wayland и не простой TTY. Это замена `agetty`, которая работает напрямую с framebuffer через DRM/KMS, рисует неоновые рамки в стиле MCD, умеет встраивать X11-окна в тайлы, запускать программы через launcher (Super+D), переключать 9 workspaces, работать с геймпадом, передавать звук через PipeWire и шарить экран в OBS/Discord через xdg-desktop-portal.

---

## Содержание

- [Идея](#идея)
- [Ключевые возможности](#ключевые-возможности)
- [Скриншот концепции](#скриншот-концепции)
- [Архитектура](#архитектура)
- [Установка](#установка)
- [Конфигурация](#конфигурация)
- [Горячие клавиши](#горячие-клавиши)
- [Запуск X11 приложений](#запуск-x11-приложений)
- [Workspaces и перемещение окон](#workspaces-и-перемещение-окон)
- [Звук (PipeWire)](#звук-pipewire)
- [Screen sharing в OBS/Discord](#screen-sharing-в-obsdiscord)
- [Геймпады](#геймпады)
- [Терминальная эмуляция](#терминальная-эмуляция)
- [GPU ускорение X11](#gpu-ускорение-x11)
- [Безопасность](#безопасность)
- [Откат к стандартному getty](#откат-к-стандартному-getty)
- [Структура проекта](#структура-проекта)
- [Roadmap](#roadmap)
- [FAQ](#faq)
- [Лицензия](#лицензия)

---

## Идея

Представьте, что **tmux встретился с i3wm и SuperHot: MCD**, а потом всё это работает не внутри X11/Wayland, а прямо на TTY — заменяет `agetty` на tty1 и сразу после загрузки показывает вам тайловый рабочий стол.

- Как в **tmux** — у вас несколько терминалов в одном экране.
- Как в **i3** — тайловый layout с бинарным деревом, workspaces, фокусом по hjkl.
- Как в **SuperHot: MCD** — глубокий фиолетовый фон, неоновые магента/циан акценты, глитч-рамки с RGB-сдвигом, всплывающие popups с corner-brackets.

Но в отличие от обычных TTY:
- Внутри тайла можно открыть **X11-окно** (Discord, браузер, Steam) — оно рендерится как часть тайла, через Xephyr + Composite.
- Поддерживается **мышь** с софтверным курсором MCD-стиля.
- Работают **геймпады** — Steam Input нативно через evdev, плюс опциональный маппинг кнопок в клавиши через SDL2.
- Звук через **PipeWire** (с PulseAudio совместимостью).
- **Screen sharing** в OBS/Discord через xdg-desktop-portal backend.
- **GPU-ускорение** через DRI3+DMA-BUF infrastructure (для браузеров и Steam).

---

## Ключевые возможности

### Менеджер окон
- **Тайловый layout** в стиле BSP/i3: бинарное дерево тайлов, split horizontal/vertical, ratio resize
- **9 workspaces** с независимыми layout-деревьями
- **Перемещение окон** по тайловой сетке и между workspaces
- **Fullscreen** toggle для отдельного тайла
- **Resize mode** для точной настройки размеров

### Запуск программ
- **Rofi-подобный launcher** `Mod4+D` — читает `.desktop` файлы, навигация стрелками + Enter
- **Кастомные команды** в конфиге (`[launcher.custom_entries]`)
- **Прямой запуск** X11 приложений через `DISPLAY=:1 <cmd>` или `Mod4+E`

### Терминал
- **zsh по умолчанию** (пользователь настраивает свой `.zshrc`)
- `TERM=xterm-256color` для максимальной совместимости
- Встроенная ANSI/VT100 state machine: 16/256-color, alt screen, scroll region, cursor positioning
- Пример `.zshrc` с ASCII-рамками в стиле MCD (см. `skel/zshrc.example`)

### X11 встраивание
- **Xephyr** на `:1` как отдельный X-сервер
- **Composite redirect** — захват top-level окон как отдельных элементов
- **DRI3 + DMA-BUF infrastructure** для GPU-ускорения (VAAPI/NVDEC в браузере, Steam)
- Авто-привязка новых X11 окон к активной плитке

### Ввод
- **Клавиатура** через evdev (эксклюзивный grab, как у agetty)
- **Мышь** через evdev + софтверный курсор MCD-стиля (неоновый крестик с glow)
- **Геймпады**: evdev passthrough (Steam Input нативно) + опционально SDL2 для маппинга кнопок

### Звук и screen share
- **PipeWire** + pipewire-pulse + wireplumber (полная PulseAudio совместимость)
- **xdg-desktop-portal backend** для screen sharing в OBS/Discord/Slack

### Эстетика
- **MCD неоновая палитра**: deep purple фон, neon magenta/cyan акценты
- **Глитч-рамки** с RGB-сдвигом (R/G/B слои со смещением)
- **Corner-brackets** на popup окнах (как в MCD HUD)
- Полностью настраиваемая тема через TOML

---

## Скриншот концепции

```
+--------------------------------------------------------------------+
│ superhot-tty v0.2                                          [ws:1]  │
+--------------------------------------------------------------------+
│ ╭─ zsh ─────────────────╮ ╭─ discord (x11) ─────────────────────╮ │
│ │ user@host ~           │ │                                      │ │
│ │ ╰─❯ ls                │ │    [Discord UI rendered here         │ │
│ │ file1.txt  file2.txt  │ │     via Xephyr + XComposite          │ │
│ │                       │ │     + DMA-BUF GPU acceleration]      │ │
│ │ ╰─❯ _                 │ │                                      │ │
│ ╰───────────────────────╯ ╰──────────────────────────────────────╯ │
│ ╭─ firefox (x11) ────────╮ ╭─ zsh ──────────────────────────────╮ │
│ │                        │ │ user@host ~/projects               │ │
│ │  [Browser with VAAPI   │ │ ╰─❯ cargo build --release          │ │
│ │   video acceleration]  │ │    Finished in 1m 23s              │ │
│ │                        │ │ ╰─❯ _                              │ │
│ ╰────────────────────────╯ ╰─────────────────────────────────────╯ │
+--------------------------------------------------------------------+
│ 1:MAIN  2:WEB  3:DEV  4:CHAT  5:MEDIA            Mod4+D: launcher │
+--------------------------------------------------------------------+
```

---

## Архитектура

```
+--------------------------------------------------------------------+
|                       superhot-tty v0.2                            |
|                                                                    |
|  +---------+  +-----------+  +----------+  +-------------------+   |
|  | Keyboard|  |  Mouse    |  | Gamepad  |  |   Config (TOML)   |   |
|  | (evdev) |  |  (evdev)  |  | (SDL2/   |  |   /etc/superhot-  |   |
|  |         |  | +cursor   |  |  passthru)|  |   tty/config.toml|   |
|  +----+----+  +-----+-----+  +----+-----+  +---------+---------+   |
|       |              \            /                   |             |
|       +-------+-------+----+-----+--------+-----------+             |
|               |            |              |                         |
|        +------v----+  +----v----+  +------v------+                  |
|        |  Launcher |  |  Layout |  |  Workspaces |                  |
|        |  (.desktop|  |  (BSP/  |  |  (1-9, ws   |                  |
|        |   scanner)|  |   i3)   |  |   switch)   |                  |
|        +------+----+  +----+----+  +------+------+                  |
|               |            |              |                         |
|        +------v------------v--------------v------+                  |
|        |              Canvas + PSF Font           |                 |
|        |          (MCD palette, neon borders)     |                 |
|        +-------------------+----------------------+                 |
|                            |                                       |
|                    +-------v--------+                              |
|                    |  DRM/KMS       |  ← /dev/dri/card0             |
|                    |  fbdev (fallback) ← /dev/fb0                   |
|                    +----------------+                              |
|                                                                    |
|  Sidecar процессы:                                                 |
|   +----------------+  +-------------+  +------------------+         |
|   | Xephyr :1      |  | PipeWire +  |  | xdg-desktop-     |         |
|   | (X11 server)   |  | pulse + wp  |  | portal backend   |         |
|   +----------------+  +-------------+  +------------------+         |
+--------------------------------------------------------------------+
```

**Слои стека:**

1. **DRM/KMS** (`src/drm/`) — прямой доступ к GPU через `/dev/dri/card0`. Modeset, dumb buffer, page-flip. Fallback на legacy `/dev/fb0` если DRM недоступен.

2. **Canvas + Font** (`src/render/`) — программный 32bpp canvas с примитивами (fill_rect, neon_border, blit). PSF1/PSF2 шрифт из `/usr/share/kbd/consolefonts/`.

3. **PTY + VTerm** (`src/term/`) — `openpty` + `fork` + `execvp` запускает zsh. Встроенная ANSI state machine обрабатывает вывод PTY.

4. **Layout** (`src/layout/`) — бинарное дерево тайлов в стиле BSP. Split h/v, ratio resize, focus navigation. Workspaces — отдельный модуль с 9 независимыми деревьями.

5. **Input** (`src/input/`) — клавиатура, мышь и геймпад через evdev (`/dev/input/event*`). EVIOCGRAB эксклюзивный захват клавиатуры.

6. **X11 compositor** (`src/x11/`) — Xephyr на `:1` + `XComposite` redirect + `XDamage` для инкрементальных обновлений + `XGetImage` для захвата пикселей. DRI3+DMA-BUF infrastructure в `src/x11/dmabuf.rs`.

7. **Audio** (`src/audio/`) — запускает pipewire, pipewire-pulse, wireplumber.

8. **Portal** (`src/portal/`) — DBus service `org.freedesktop.impl.portal.desktop.SuperHot` с интерфейсом ScreenCast.

9. **Launcher** (`src/launcher/`) — сканирует `.desktop` файлы, fuzzy-поиск, MCD-styled popup.

10. **Config** (`src/config/`) — TOML конфиг с секциями для всего.

---

## Установка

### Требования

- **Arch Linux** (или любой дистрибутив с systemd)
- **DRM/KMS совместимое ядро** (Intel/AMD/NVIDIA)
- Для NVIDIA: `nvidia-drm.modeset=1` в параметрах ядра
- Доступ к `/dev/dri/card0` и `/dev/input/event*`

### Зависимости

**Обязательные:**
```bash
sudo pacman -S rust cargo gcc pkgconf systemd kbd zsh \
               xorg-server-xephyr \
               pipewire pipewire-pulse wireplumber \
               xdg-desktop-portal
```

**Опциональные:**
```bash
# Для SDL2 маппинга геймпада вне Steam:
sudo pacman -S sdl2

# Для GTK file chooser portal:
sudo pacman -S xdg-desktop-portal-gtk
```

### Установка

```bash
git clone https://github.com/TumRedSun/SH-tty.git
cd SH-tty
sudo ./install.sh
sudo reboot
```

`install.sh` делает следующее:
1. Проверяет зависимости, предлагает установить через pacman
2. Собирает release-бинарник (`cargo build --release`)
3. Копирует бинарник в `/usr/local/bin/superhot-tty`
4. Устанавливает systemd unit в `/etc/systemd/system/superhot-tty@.service`
5. Устанавливает конфиг по умолчанию в `/etc/superhot-tty/config.toml`
6. Дизейблит стандартный `getty@tty1`
7. Включает `superhot-tty@tty1`
8. Добавляет текущего пользователя в группы `video`, `input`, `render`, `audio`
9. Включает pipewire user services

### Сборка с SDL2 gamepad support

Если установлен `libsdl2-dev`, можно собрать с поддержкой маппинга кнопок геймпада:

```bash
sudo pacman -S sdl2
cargo build --release --features gamepad-sdl2
```

Без этой фичи геймпад работает в режиме evdev passthrough — Steam Input обрабатывает контроллер сам.

---

## Конфигурация

Основной конфиг: **`/etc/superhot-tty/config.toml`**

Полный пример — в `config/default.toml`. Ниже — основные секции.

### General

```toml
[general]
shell = "zsh"                    # shell для терминалов
font = "Lat2-Terminus16"         # PSF шрифт
font_size = 16
gap = 4                          # зазор между плитками (px)
border = 1                       # толщина рамки (px)
outer_padding = 4                # внешний отступ (px)
status_bar_height = 24
framerate = 60                   # FPS target
glitch_intensity = 0.15          # 0..1 — сила глитч-эффектов MCD
```

### Theme

Все цвета — в формате `#RRGGBB`.

```toml
[theme]
bg = "#0A0716"                   # фон экрана
tile_bg_inactive = "#120E24"     # фон неактивной плитки
tile_bg_active = "#0F0A1E"       # фон активной плитки
border_inactive = "#3A2D5C"      # рамка неактивной
border_active = "#FF2E97"        # рамка активной (неоновая магента)
border_x11 = "#00F0FF"           # рамка X11 плитки (неоновый циан)
fg_default = "#E6E1F0"           # текст по умолчанию
fg_dim = "#7A6F96"               # тусклый текст
accent_magenta = "#FF2E97"       # акцент магента
accent_cyan = "#00F0FF"          # акцент циан
popup_bg = "#140B2E"
popup_border = "#FF2E97"
error = "#FF4D4D"
```

### Key bindings

Каждый биндинг — отдельный `[[keybindings]]` блок.

```toml
[[keybindings]]
key = "d"                        # клавиша (буква/цифра/Return/Space/F1/...)
mods = ["Super"]                 # модификаторы: Super, Ctrl, Alt, Shift
action = { type = "launcher" }   # действие

[[keybindings]]
key = "Return"
mods = ["Super"]
action = { type = "terminal" }

[[keybindings]]
key = "1"
mods = ["Super"]
action = { type = "workspace", n = 1 }

[[keybindings]]
key = "1"
mods = ["Super", "Shift"]
action = { type = "move_to_workspace", n = 1 }

[[keybindings]]
key = "h"
mods = ["Super"]
action = { type = "focus", dir = "left" }

[[keybindings]]
key = "h"
mods = ["Super", "Shift"]
action = { type = "move", dir = "left" }

[[keybindings]]
key = "h"
mods = ["Super", "Alt"]
action = { type = "resize", dir = "left", delta = 0.05 }

[[keybindings]]
key = "e"
mods = ["Super"]
action = { type = "spawn_x11", cmd = "xterm", args = [] }
```

**Все поддерживаемые действия:**

| Action                                  | Описание                                    |
|-----------------------------------------|---------------------------------------------|
| `terminal`                              | Новый терминальный тайл                     |
| `launcher`                              | Открыть launcher (Super+D)                  |
| `spawn { cmd, args }`                   | Запустить команду                           |
| `spawn_x11 { cmd, args }`               | Запустить X11 приложение в новой плитке     |
| `split_horizontal` / `split_vertical`   | Split активного тайла                       |
| `focus { dir }`                         | Фокус в направлении                         |
| `move { dir }`                          | Переместить окно в направлении              |
| `swap { dir }`                          | Swap с соседом                              |
| `workspace { n }`                       | Переключиться на workspace N (1..9)         |
| `move_to_workspace { n }`               | Переместить окно на workspace N             |
| `close`                                 | Закрыть тайл                                |
| `fullscreen`                            | Fullscreen toggle                           |
| `resize_mode`                           | Resize mode (HJKL)                          |
| `resize { dir, delta }`                 | Resize split в направлении                  |
| `cycle_focus`                           | Cycle focus по всем тайлам                  |
| `quit`                                  | Выход                                       |

`dir` — одно из: `left`, `right`, `up`, `down`.

### Workspaces

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
```

### Launcher

```toml
[launcher]
max_rows = 12
x11_display = ":1"
desktop_paths = [
    "/usr/share/applications",
    "/usr/local/share/applications",
    "~/.local/share/applications",
]

[launcher.custom_entries]
"terminal: bash" = "bash"
"reload config" = "superhot-tty-reload"
```

### Audio

```toml
[audio]
start_pipewire_pulse = true       # PulseAudio совместимость
start_wireplumber = true          # session manager
default_volume = 70
```

### Portal (screen share)

```toml
[portal]
start_portal = true
service_name = "org.freedesktop.impl.portal.desktop.SuperHot"
object_path = "/org/freedesktop/portal/desktop"
```

### Gamepad

```toml
[gamepad]
enabled = true
steam_passthrough = true          # evdev passthrough для Steam
stick_sensitivity = 50            # 1..100

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

### X11

```toml
[x11]
dri3 = true                       # DRI3 + DMA-BUF GPU-ускорение
display = ":1"                    # Xephyr display
screen_size = [1920, 1080]
xtest_input = true                # XTest для ввода в X11 окна
hardware_cursor = true            # DRM hardware cursor plane
```

---

## Горячие клавиши

`Mod4` = Super/Windows key.

| Hotkey              | Действие                                  |
|---------------------|-------------------------------------------|
| `Mod4+D`            | **Launcher** (rofi-подобный)              |
| `Mod4+Enter`        | Новый терминал (zsh)                      |
| `Mod4+V`            | Split vertical                            |
| `Mod4+H/J/K/L`      | Фокус left/down/up/right                  |
| `Mod4+Shift+H/J/K/L`| Переместить окно в направлении            |
| `Mod4+Ctrl+H/J/K/L` | Swap с соседом                            |
| `Mod4+1..9`         | Workspace N                               |
| `Mod4+Shift+1..9`   | Переместить окно на workspace N           |
| `Mod4+Q`            | Закрыть тайл                              |
| `Mod4+F`            | Fullscreen toggle                         |
| `Mod4+R`            | Resize mode (HJKL)                        |
| `Mod4+E`            | Открыть X11-плитку                        |
| `Mod4+Space`        | Cycle focus                               |
| `Mod4+Alt+H/J/K/L`  | Resize split в направлении                |
| `Mod4+W`            | Toggle layout (план)                      |
| `Mod4+Shift+E`      | Quit                                      |
| `Esc`               | Выйти из resize mode / закрыть popups     |

Все биндинги можно переназначить в конфиге.

---

## Запуск X11 приложений

### Способ 1: Через launcher (рекомендуется)

1. Нажмите `Mod4+D` — откроется launcher в центре экрана
2. Начните вводить имя программы (например, "firefox", "discord", "steam")
3. `↑↓` или `k/j` для навигации по списку
4. `Enter` для запуска — приложение откроется в новой X11-плитке на текущем workspace

Launcher читает `.desktop` файлы из стандартных директорий:
- `/usr/share/applications`
- `/usr/local/share/applications`
- `~/.local/share/applications`

### Способ 2: Вручную

1. Нажмите `Mod4+E` — создаётся пустая X11-плитка с надписью "X11 TILE — run: DISPLAY=:1 discord"
2. Из любого терминала внутри superhot-tty выполните:
   ```bash
   DISPLAY=:1 discord
   ```
3. Новое X-окно автоматически привяжется к выделенной плитке

### Способ 3: Через конфиг

Добавьте биндинг в `config.toml`:

```toml
[[keybindings]]
key = "b"
mods = ["Super"]
action = { type = "spawn_x11", cmd = "firefox", args = ["--new-window", "https://example.com"] }
```

---

## Workspaces и перемещение окон

### Переключение workspaces

- `Mod4+1` — MAIN
- `Mod4+2` — WEB
- `Mod4+3` — DEV
- ...
- `Mod4+9`

Имена workspaces настраиваются в `[[workspaces]]` секции конфига.

### Перемещение окон между workspaces

1. Сделайте окно активным (через `Mod4+H/J/K/L`)
2. Нажмите `Mod4+Shift+N` где N — целевой workspace
3. Окно перенесётся на workspace N, layout текущего workspace обновится

Состояние layout каждого workspace сохраняется независимо при переключении.

### Перемещение окон внутри workspace

- `Mod4+Shift+H/J/K/L` — переместить активное окно в направлении (swap с соседним тайлом)
- `Mod4+Ctrl+H/J/K/L` — swap с соседом (аналогично move)
- `Mod4+Alt+H/J/K/L` — resize split в направлении (delta = 0.05)

---

## Звук (PipeWire)

superhot-tty автоматически запускает полный PipeWire стек:

| Процесс            | Роль                              |
|---------------------|-----------------------------------|
| `pipewire`          | основной daemon                   |
| `pipewire-pulse`    | PulseAudio совместимость          |
| `wireplumber`       | session manager (route audio)     |

Приложения (Discord, браузеры, Steam) видят PulseAudio API и работают нативно.

### Управление звуком

```bash
# Громкость.
pactl set-sink-volume @DEFAULT_SINK@ 80%
pactl set-sink-volume @DEFAULT_SINK@ +10%
pactl set-sink-volume @DEFAULT_SINK@ -10%

# Mute toggle.
pactl set-sink-mute @DEFAULT_SINK@ toggle

# Список выходов.
pactl list sinks short

# Список источников (микрофоны).
pactl list sources short
```

### Bluetooth наушники

PipeWire + WirePlumber поддерживают Bluetooth из коробки. Подключение через `bluetoothctl` или `wpctl`:

```bash
bluetoothctl
[bluetooth]# power on
[bluetooth]# scan on
[bluetooth]# pair <MAC>
[bluetooth]# connect <MAC>
```

---

## Screen sharing в OBS/Discord

superhot-tty регистрирует DBus service `org.freedesktop.impl.portal.desktop.SuperHot` с интерфейсом `org.freedesktop.impl.portal.ScreenCast`.

### Discord / Slack

1. В Discord нажмите "Share Screen"
2. В диалоге выбора источника выберите "SuperHot" monitor
3. Stream начнётся через PipeWire

### OBS Studio

1. Add Source → **ScreenCast (Portal)**
2. Выберите "SuperHot"
3. OBS получит кадры через PipeWire

### Проверка работы портала

```bash
# Проверить DBus service.
busctl --user list | grep SuperHot

# Проверить xdg-desktop-portal.
systemctl --user status xdg-desktop-portal
```

---

## Геймпады

### Steam (нативно)

Steam Input работает через evdev — superhot-tty не вмешивается. Просто запустите Steam:

```bash
DISPLAY=:1 steam
```

Все контроллеры (Xbox, PS4/PS5, Switch Pro, 8BitDo, Steam Deck) работают в Steam играх.

### Не-Steam сценарий (опционально)

Если собрать с `--features gamepad-sdl2`, можно мапить кнопки геймпада в клавиши:

```toml
[gamepad]
enabled = true
steam_passthrough = true
stick_sensitivity = 50

[gamepad.keymap]
"a" = "Return"
"b" = "Escape"
"x" = "space"
"y" = "Tab"
"dpad_up" = "k"           # Mod4+K = фокус вверх
"dpad_down" = "j"
"dpad_left" = "h"
"dpad_right" = "l"
"start" = "Super"
"back" = "Super"
"left_shoulder" = "bracketleft"
"right_shoulder" = "bracketright"
```

Это позволяет управлять WM через геймпад без Steam.

### Поддерживаемые контроллеры

Через SDL2 (если собрано с `--features gamepad-sdl2`):
- Xbox 360 / Xbox One / Xbox Series X|S
- PlayStation 4 (DualShock)
- PlayStation 5 (DualSense)
- Nintendo Switch Pro Controller
- 8BitDo (все модели)
- Steam Deck
- Любой контроллер из [SDL GameControllerDB](https://github.com/gabomdq/SDL_GameControllerDB)

Через evdev (всегда): любой HID-совместимый геймпад.

---

## Терминальная эмуляция

### Shell

- По умолчанию запускается **zsh** (если установлен)
- Fallback на **bash** если zsh не найден
- Можно изменить в конфиге: `shell = "fish"` или любой другой

### Переменные окружения

При запуске PTY устанавливаются:

| Переменная        | Значение                          |
|-------------------|-----------------------------------|
| `TERM`            | `xterm-256color`                  |
| `COLORTERM`       | `truecolor`                       |
| `LANG`            | `en_US.UTF-8` (если не задан)     |
| `SUPERHOT_TTY`    | `1` (для тем zsh)                 |
| `DISPLAY`         | `:1` (для X11 приложений)         |

### Поддержка ANSI

Встроенная state machine поддерживает:

- **CSI sequences**: cursor position (H/f), cursor up/down/fwd/back (A/B/C/D), line/column absolute (d/G)
- **Erase**: display (J), line (K)
- **SGR** (colors): 16 base colors, 256-color (38;5;N), truecolor (38;2;R;G;B — мапится в палитру)
- **Modes**: alt screen (?1049), cursor visibility (?25), scroll region (DECSTBM)
- **Scrolling**: scroll up (SU), scroll down (SD), insert/delete lines (L/M)
- **Edit**: insert/delete chars (@/P)
- **OSC**: window title (0;title, 2;title)
- **Special**: BEL, BS, HT, LF, CR

### Пример .zshrc

В `skel/zshrc.example` — готовая тема с ASCII-рамками в стиле MCD:

```
╭─ user@host ~
╰─❯ ls
file1.txt  file2.txt
╭─❯ _
```

Скопируйте в `~/.zshrc` или адаптируйте под себя.

---

## GPU ускорение X11

### Почему это важно

Без GPU-ускорения:
- Браузер рендерит видео через CPU → 100% CPU, слайд-шоу
- Steam показывает игры через software rendering
- Discord анимации лагают

С GPU-ускорением (DRI3 + DMA-BUF):
- Браузер использует VAAPI/NVDEC для видео → <5% CPU
- Steam аппаратное ускорение
- Discord плавные анимации

### Как это работает

```
┌─────────────┐    DRI3 PixmapFromBuffer    ┌─────────────┐
│  Xephyr :1  │ ──────────────────────────→ │ DMA-BUF fd  │
│  (X window) │                             │ (GPU memory)│
└─────────────┘                             └──────┬──────┘
                                                   │
                                          DRM_IOCTL_PRIME_FD_TO_HANDLE
                                                   │
                                                   ▼
┌─────────────┐    DRM_IOCTL_MODE_ADDFB2       ┌─────────────┐
│  DRM KMS    │ ←───────────────────────────── │ DRM FB      │
│  (scanout)  │                               │ (hardware)  │
└─────────────┘                               └─────────────┘
       │
       │  DRM atomic commit: assign FB to overlay plane
       ▼
   Monitor output (0% CPU)
```

### Текущий статус

- ✅ DRI3 infrastructure (`src/x11/dmabuf.rs`) — проверка extension, заготовка
- ✅ CompositeNameWindowPixmap + XGetImage fallback (работает, CPU blit)
- 🚧 DRI3PixmapFromBuffer FFI — заготовка, требует xcb-dri3 bindings
- 🚧 DRM hardware overlay planes — план на v0.3

Пока DRI3 полностью не реализован, используется `XGetImage` + CPU blit. Это работает, но нагрузка на CPU выше.

### Проверка GPU ускорения в браузере

После запуска Firefox на нашем display:

```bash
DISPLAY=:1 firefox
# В about:support проверьте "Compositing" → должно быть WebRender
```

---

## Безопасность

### Запуск от root

`superhot-tty` запускается от root (нужно для DRM master и `/dev/input/event*`). Это нормально для single-user desktop конфигурации, но требует осторожности.

### Xephyr без auth

Xephyr запускается с `-ac` (без Xauth). Это означает, что любой локальный процесс может подключиться к `:1`. Безопасно только в single-user окружении. Для multi-user добавьте Xauth файл.

### EVIOCGRAB

Клавиатура захватывается эксклюзивно через `EVIOCGRAB`. Это предотвращает одновременную обработку событий обычным TTY вводом.

### xdg-desktop-portal

Screen share через портал требует явного подтверждения пользователя в диалоге (через xdg-desktop-portal-gtk или аналогичный frontend).

---

## Откат к стандартному getty

Если что-то сломалось, откат к стандартному `agetty`:

```bash
# На tty2 (Ctrl+Alt+F2) залогиньтесь и выполните:
sudo systemctl disable superhot-tty@tty1
sudo rm /etc/systemd/system/getty@tty1.service.d/override.conf
sudo systemctl enable getty@tty1
sudo systemctl daemon-reload
sudo reboot
```

После перезагрузки на tty1 будет стандартный `agetty` с login prompt.

---

## Структура проекта

```
superhot-tty/
├── Cargo.toml                 # Rust манифест
├── install.sh                 # установщик для Arch Linux
├── README.md                  # этот файл
├── config/
│   └── default.toml           # пример конфигурации
├── skel/
│   └── zshrc.example          # пример .zshrc с MCD-стилем
├── systemd/
│   └── superhot-tty@.service  # systemd unit
└── src/
    ├── main.rs                # entry point, event loop, hotkey handling
    ├── config/mod.rs          # TOML config + bindings + actions
    ├── launcher/mod.rs        # rofi-like launcher (.desktop scanner)
    ├── drm/
    │   ├── mod.rs             # Backend enum (DRM vs fbdev)
    │   ├── kms.rs             # DRM/KMS init, modeset, page-flip
    │   └── fbdev.rs           # legacy /dev/fb0 fallback
    ├── render/
    │   ├── mod.rs
    │   ├── canvas.rs          # drawing primitives, neon borders, blit
    │   ├── font.rs            # PSF1/PSF2 font loader
    │   └── text.rs            # glyph rendering
    ├── term/
    │   ├── mod.rs
    │   ├── pty.rs             # openpty + fork + exec (zsh по умолчанию)
    │   └── vterm.rs           # ANSI/VT100 state machine
    ├── layout/
    │   ├── mod.rs             # binary tree tiling (BSP/i3-style)
    │   └── workspaces.rs      # 9 workspaces + move between
    ├── input/
    │   ├── mod.rs
    │   ├── keyboard.rs        # evdev reader, scancode → Key
    │   ├── mouse.rs           # evdev mouse + cursor
    │   └── gamepad.rs         # SDL2 + evdev passthrough
    ├── x11/
    │   ├── mod.rs
    │   ├── compositor.rs      # Xephyr + Composite + Damage + XGetImage
    │   └── dmabuf.rs          # DRI3 + DMA-BUF infrastructure
    ├── audio/mod.rs           # PipeWire stack
    ├── portal/mod.rs          # xdg-desktop-portal ScreenCast
    └── ui/
        ├── mod.rs
        ├── theme.rs           # MCD palette
        └── popup.rs           # SuperHot-style popups
```

---

## Roadmap

### v0.2 (текущая)
- ✅ TOML конфиг
- ✅ Workspaces 1-9
- ✅ Launcher (.desktop scanner)
- ✅ Mouse + cursor
- ✅ Gamepad (evdev + опционально SDL2)
- ✅ PipeWire audio stack
- ✅ xdg-desktop-portal ScreenCast backend
- ✅ DRI3/DMA-BUF infrastructure
- ✅ zsh по умолчанию, TERM=xterm-256color
- ✅ Перемещение окон между workspaces

### v0.3 (план)
- 🚧 Полная DRI3/DMA-BUF реализация (FFI к xcb-dri3)
- 🚧 Hardware DRM cursor plane (вместо софтверного курсора)
- 🚧 Hardware DRM overlay planes для X11 окон (0% CPU)
- 🚧 Конфигурация темы оформления через TOML (live reload)
- 🚧 Анимации перехода между workspaces
- 🚧 Polybar/waybar-совместимый status bar
- 🚧 Поддержка нескольких мониторов
- 🚧 D-Bus интерфейс для управления WM извне
- 🚧 IPC сокет для скриптов (как i3-msg)

### v0.4 (далеко)
- Полная замена libvterm (полная xterm совместимость)
- Поддержка HiDPI масштабирования
- Custom shaders для глитч-эффектов (через DRM atomic planes)
- KMS atomic commit для всех обновлений экрана

---

## FAQ

### Q: Работает ли с NVIDIA proprietary driver?

**A:** Да, если в параметрах ядра указан `nvidia-drm.modeset=1`. Это обязательно для DRM master. Проверьте:

```bash
cat /sys/module/nvidia_drm/parameters/modeset
# должно вывести Y
```

Если нет — добавьте `nvidia-drm.modeset=1` в `GRUB_CMDLINE_LINUX_DEFAULT` в `/etc/default/grub` и обновите:

```bash
sudo grub-mkconfig -o /boot/grub/grub.cfg
```

### Q: Можно ли использовать другой shell (fish, bash)?

**A:** Да. В конфиге:

```toml
[general]
shell = "fish"
```

Или укажите полный путь: `shell = "/usr/bin/fish"`.

### Q: Как добавить свою программу в launcher?

**A:** Три способа:

1. **Создать `.desktop` файл** в `~/.local/share/applications/`:

```ini
[Desktop Entry]
Name=My App
Exec=/usr/bin/myapp
Icon=myapp
Terminal=false
Type=Application
Categories=Utility;
```

2. **Добавить в `[launcher.custom_entries]`** в конфиге:

```toml
[launcher.custom_entries]
"my app" = "/usr/bin/myapp"
"another" = "alacritty -e htop"
```

3. **Забиндить на горячую клавишу**:

```toml
[[keybindings]]
key = "m"
mods = ["Super"]
action = { type = "spawn_x11", cmd = "myapp", args = [] }
```

### Q: Не работает мышь в X11 окне

**A:** Мышь маршрутизируется в X11 окно только если оно активно. Убедитесь что:
1. X11 плитка в фокусе (рамка неоновая магента или циан)
2. Mouse device найден (проверьте лог: `journalctl -u superhot-tty@tty1 | grep mouse`)

### Q: Steam не видит геймпад

**A:** Steam Input требует evdev доступа. Убедитесь что пользователь в группе `input`:

```bash
groups $USER
# должно быть input в списке
```

Если нет:

```bash
sudo usermod -aG input $USER
# перелогиньтесь
```

### Q: Как отладить проблемы?

**A:** Включите debug логи:

```bash
# В systemd unit добавьте:
sudo systemctl edit superhot-tty@tty1
# В открывшемся редакторе:
[Service]
Environment=RUST_LOG=debug
```

Затем:

```bash
sudo systemctl restart superhot-tty@tty1
journalctl -u superhot-tty@tty1 -f
```

### Q: Можно ли запускать Wayland приложения?

**A:** Нет, superhot-tty использует X11 (через Xephyr). Wayland приложения нужно запускать через XWayland внутри Xephyr, но это требует дополнительной настройки. План на v0.4.

### Q: Как переключиться на обычный TTY?

**A:** `Ctrl+Alt+F2` — стандартный getty. `Ctrl+Alt+F1` — обратно в superhot-tty.

### Q: Работает ли с несколькими мониторами?

**A:** В v0.2 — нет, только primary output. Мульти-монитор поддержка планируется в v0.3.

### Q: Как обновить конфиг без перезагрузки?

**A:** На v0.2 — только перезапуск сервиса:

```bash
sudo systemctl restart superhot-tty@tty1
```

Live reload планируется в v0.3.

---

## Лицензия

MIT License. См. [LICENSE](LICENSE).

## Контрибьюшн

PR приветствуются! Особенно:
- Реализация DRI3 FFI (через `xcb-dri3` bindings)
- Hardware DRM cursor plane
- Мульти-монитор поддержка
- Улучшения ANSI совместимости (полная xterm эмуляция)
- Перевод документации на другие языки
