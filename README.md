# shtty v0.6

**Тайловый оконный менеджер для Linux-консоли (DRM/KMS) с собственным login screen и X11-встраиванием**

> Полная замена `agetty`: вход по логину/паролю, тайловая раскладка, workspaces, multi-monitor, launcher (.desktop), X11-окна через встроенный X-сервер (DRI3/DMA-BUF + hardware cursor + overlay planes), window rules, autostart, live-reload конфигурации, i3-msg-совместимый IPC-сокет, PipeWire звук и xdg-desktop-portal для screen share.

---

## Содержание

- [Возможности](#возможности)
- [Архитектура](#архитектура)
- [Быстрый старт](#быстрый-старт)
- [Установка](#установка)
- [Экран входа](#экран-входа)
- [Шрифты и иконки](#шрифты-и-иконки)
- [Конфигурация](#конфигурация)
- [Горячие клавиши (по умолчанию)](#горячие-клавиши-по-умолчанию)
- [X11-окна](#x11-окна)
- [IPC (i3-msg совместимый)](#ipc-i3-msg-совместимый)
- [Multi-monitor](#multi-monitor)
- [Звук и screen share](#звук-и-screen-share)
- [Сборка пакетов](#сборка-пакетов)
- [Структура проекта](#структура-проекта)
- [Устранение неполадок](#устранение-неполадок)
- [Лицензия](#лицензия)

---

## Возможности

- **Login screen с privilege separation** — простой вход: логин → Enter → пароль → Enter. UI работает от непривилегированного системного пользователя, PAM-аутентификация — в root-родителе через socketpair.
- **Тайловая раскладка** — BSP-сплиты, фокус/перемещение/swap, resize mode, fullscreen, tabs.
- **Workspaces** — 1..9 + 0 (10 штук), привязка к мониторам.
- **Multi-monitor** — прямая работа с DRM/KMS, привязка workspaces к коннекторам.
- **X11-встраивание** — приложения запускаются на встроенном X-сервере (Xvfb) и отображаются в плитках. DRI3 + DMA-BUF для GPU-ускорения, hardware cursor, overlay planes.
- **Launcher** — rofi-подобный (Mod4+D), читает `.desktop` файлы.
- **Window rules** — авторазмещение окон по WM_CLASS/заголовку: workspace, размер, позиция, fullscreen.
- **Autostart** — команды трёх типов: `command` (фон), `x11` (графические), `terminal` (нативный терминал).
- **Status bar** — polybar/waybar-стиль, модули: workspaces, clock, cpu, memory, battery, network, text, custom.
- **Live reload** — правки config.toml применяются на лету (тема, биндинги, bar, window rules).
- **IPC** — UNIX-сокет с i3-msg-совместимым протоколом + утилита `shtty-msg`.
- **Шрифты** — TTF через freetype (полный Unicode: кириллица, box-drawing, CJK) + PSF fallback + цепочка fallback-шрифтов для Nerd Font иконок.
- **Звук** — автостарт PipeWire/PulseAudio-стека, геймпад через evdev (Steam Input) или SDL2.

## Архитектура

```
┌──────────────────────────────────────────────────────────┐
│ systemd shtty@tty1.service (root)                        │
│   1. Открыть DRM master + /dev/input/event*              │
│   2. fork() → ребёнок drop к 'shtty' → login screen      │
│   3. Родитель (root) — PAM auth через socketpair         │
│   4. Успех → drop к вошедшему пользователю → WM + IPC    │
└──────────────────────────────────────────────────────────┘
        │
        ▼
┌──────────────┐  ┌──────────────┐  ┌───────────────────┐
│ DRM/KMS      │  │ evdev input  │  │ X11 (Xvfb :1)     │
│ framebuffer  │  │ клавиатура/  │  │ окна → плитки     │
│ + planes     │  │ мышь/пад     │  │ (DRI3/XTest)      │
└──────────────┘  └──────────────┘  └───────────────────┘
```

Дочерний процесс login screen никогда не работает от root: уязвимость в рендеринге или разборе ввода не даёт эскалации привилегий. IPC-сокет создаётся после смены пользователя с правами 0600 + SO_PEERCRED.

## Быстрый старт

```bash
git clone https://github.com/TumRedSun/SH-tty.git
cd SH-tty
sudo ./install.sh      # Arch Linux: сборка + установка + systemd
sudo reboot
```

После перезагрузки на tty1 стартует shtty: введите логин, Enter, пароль, Enter.

Ручная сборка (без install.sh):

```bash
cargo build --release
# бинарники: target/release/shtty, target/release/shtty-msg
```

Зависимости для сборки: rust/cargo, pkg-config, freetype2 (libfreetype-dev), libx11/x11rb — всё опционально гасится фичами (`--no-default-features` для сборки без freetype → PSF-шрифты).

## Установка

install.sh (Arch Linux) делает:

1. Создаёт системного пользователя `shtty` (группы video, input, render, tty — **без** shadow).
2. Ставит зависимости через pacman (включая шрифты `ttf-dejavu`, `ttf-nerd-fonts-symbols`, `noto-fonts`).
3. Собирает release-бинарники и ставит в `/usr/local/bin/`.
4. Устанавливает `systemd/shtty@.service` → `/etc/systemd/system/`.
5. Устанавливает конфиг в `/etc/shtty/config.toml`, создаёт `/etc/shtty/font-fallbacks/`.
6. Отключает `getty@tty1`, включает `shtty@tty1`.

Откат на обычный getty:

```bash
sudo systemctl disable shtty@tty1
sudo systemctl enable getty@tty1
sudo rm -rf /etc/systemd/system/getty@tty1.service.d
sudo systemctl daemon-reload
```

## Экран входа

Простой поток без приветственных экранов:

```
              SHTTY
      Tiling Window Manager
         12:34:56

         Логин: user▌
   Enter — продолжить, Esc — сбросить

         Пароль: ****▌
   Enter — войти, Esc — назад
```

- Ввод логина → Enter → ввод пароля → Enter → аутентификация.
- Ошибка → Enter повторяет ввод пароля.
- Язык подписей: `[login] language = "ru"` или `"en"`.

Аутентификация: PAM (feature `pam`, включается install.sh при наличии libpam) либо fallback через `/etc/shadow` + `crypt()` с constant-time сравнением.

## Шрифты и иконки

Порядок загрузки основного шрифта:

1. `/etc/shtty/font.ttf` (или `.otf`) — пользовательский override.
2. `/etc/shtty/font.psfu` (или `.psf`) — PSF override.
3. `fc-match` по `general.font` (например `DejaVu Sans Mono`), затем любой monospace.
4. PSF из `/usr/share/kbd/consolefonts/` (kbd).
5. Встроенный процедурный 8x16 (только ASCII).

TTF рендерится через freetype в 1-битные ячейки — полное Unicode-покрытие: кириллица, греческий, box-drawing (htop/vim/tmux), блоки.

**Fallback-цепочка для иконок.** Основной monospace-шрифт не содержит Nerd Font иконок (U+E000–U+F8FF), Powerline-символов и многих спецзнаков — вместо них рисуется `?`. shtty автоматически подключает до 6 символ-шрифтов как fallback:

1. `/etc/shtty/font-fallbacks/*.ttf|otf` — пользовательский каталог.
2. Известные семейства через fc-match: `Symbols Nerd Font (Mono)`, `Noto Sans Symbols (2)`, `Symbola`, `Unifont`, `Noto Sans Math`, `Powerline`.
3. Скан `/usr/share/fonts`, `/usr/local/share/fonts`, `~/.local/share/fonts` по маскам (nerd/symbols/symbola/unifont/...).

Подключаются только шрифты, добавляющие **новые** codepoints; эмодзи-шрифты пропускаются (1-битный рендер не умеет цвет). Логи покажут, сколько шрифтов подключено и какие codepoints добавлены.

Пакеты: Arch — `ttf-nerd-fonts-symbols`, `noto-fonts`; Debian — `fonts-noto-core`, `fonts-symbola`.

## Конфигурация

Пути (по приоритету): `$XDG_CONFIG_HOME/shtty/config.toml` → `~/.config/shtty/config.toml` → `/etc/shtty/config.toml`.

Полный рабочий пример — [config/default.toml](config/default.toml). Основные секции:

### [general]
```toml
shell = "zsh"          # шелл нативных терминалов
font = "DejaVu Sans Mono"  # семейство для fc-match
font_size = 16         # высота глифа в пикселях
gap = 4                # отступ между плитками
border = 1             # ширина бордюра
outer_padding = 4      # отступ от краёв экрана
framerate = 60         # FPS лимит
workspace_count = 10   # 1..9 + 0
```

### [theme]
Все цвета в hex `#RRGGBB`: `bg`, `tile_bg_active/inactive`, `border_active/inactive`, `border_x11`, `fg_default`, `fg_dim`, `accent_magenta`, `accent_cyan`, `popup_bg`, `popup_border`, `error`. Палитра по умолчанию — неоновая (фиолет/магента/циан), полностью переопределяется.

### [login]
```toml
title = ""             # пусто → "SHTTY"
subtitle = ""          # пусто → "Tiling Window Manager" / "Тайловый менеджер"
language = "en"        # подписи: "en" | "ru"
show_clock = true
show_hint = true       # подсказки под полями ввода
pam_service = "login"
```

### [bar]
```toml
enabled = true
position = "bottom"    # top | bottom
height = 24
```
Модули `[[bar.modules]]`: `type` = workspaces | clock | cpu | memory | battery | network | text | custom; `position` = left/center/right; `format`, `color`, `refresh_ms`, `cmd` (для text/custom).

### [x11]
```toml
dri3 = true
display = ":1"         # дисплей встроенного X-сервера
screen_size = [1920, 1080]
xtest_input = true     # форвардинг клавиатуры в X11-окна
hardware_cursor = true
auto_place_windows = true
overlay_planes = true  # вывод X11-окон через hardware overlay (0% CPU)
```

### [ipc]
```toml
enabled = true
# socket_path = "/run/user/1000/shtty.sock"  # пусто = авто
socket_mode = 0o600
```

### Прочее
- `[[workspaces]]` — имя и `on_init` для каждого ws.
- `[[monitors]]` — привязка workspaces к коннекторам (`eDP-1`, `HDMI-A-1`...), позиция.
- `[[window_rules]]` — `match_class/match_title/match_app_id`, `workspace`, `size`, `position`, `fullscreen`, `focus`, `regex`.
- `[[autostart]]` — `type` = command | x11 | terminal, `cmd`, `args`, `delay_ms`, `workspace`.
- `[launcher]` — `desktop_paths`, `custom_entries`, `terminal_shell`.
- `[popups]` — `duration_frames`, `max_width_pct`.
- `[audio]` — автостарт pipewire/pipewire-pulse/wireplumber.
- `[portal]` — бэкенд xdg-desktop-portal (`org.freedesktop.impl.portal.desktop.shtty`).
- `[gamepad]` — маппинг кнопок в клавиши, `steam_passthrough`.
- `[live_reload]` — watcher inotify, `debounce_ms`.

Live reload применяет на лету: theme, keybindings, window_rules, bar, ipc, popups. Требуют перезапуска: general.font, x11.*, monitors.

## Горячие клавиши (по умолчанию)

| Клавиши | Действие |
|---|---|
| Mod4+Enter | новый терминал |
| Mod4+D | launcher |
| Mod4+E | X11-клиент (xterm) |
| Mod4+H/J/K/L | фокус влево/вниз/вверх/вправо |
| Mod4+Shift+H/J/K/L | перемещение окна |
| Mod4+Ctrl+H/J/K/L | swap |
| Mod4+V | вертикальный сплит |
| Mod4+R | resize mode (HJKL, Esc) |
| Mod4+Q | закрыть окно |
| Mod4+F | fullscreen |
| Mod4+Space | цикл фокуса |
| Mod4+W | переключение направления сплита |
| Mod4+1..9, 0 | workspace |
| Mod4+Shift+1..9, 0 | переместить окно на workspace |
| Mod4+Ctrl+R | reload конфига |
| Mod4+Shift+E | выход |
| Mod4+P | popup из скрипта |

Все биндинги переопределяются в `[[keybindings]]` (см. default.toml).

## X11-окна

shtty поднимает Xvfb на дисплее `:1` и встраивает X11-окна в плитки:

- Новое X11-окно → новая плитка на текущем workspace (или по `[[window_rules]]`).
- Клавиатура форвардится через XTest (клик мышью по плитке → фокус).
- Рендеринг: DRI3/DMA-BUF через overlay planes (GPU, 0% CPU) либо CPU-blit fallback.
- Запуск вручную из терминала: `DISPLAY=:1 discord`.

## IPC (i3-msg совместимый)

```bash
shtty-msg 'workspace 2'
shtty-msg 'exec firefox'
shtty-msg 'reload'
shtty-msg 'kill'
shtty-msg 'get_workspaces'
shtty-msg 'get_config'
shtty-msg 'get_focused'
shtty-msg 'get_version'
```

UNIX-сокет: `$XDG_RUNTIME_DIR/shtty.sock` (или `/tmp/shtty-$UID.sock`), права 0600, проверка SO_PEERCRED — команды принимает только вошедший пользователь.

## Multi-monitor

Каждый коннектор получает свои workspaces:

```toml
[[monitors]]
connector = "eDP-1"
workspaces = [1, 3, 5, 7, 9]
position = "primary"

[[monitors]]
connector = "HDMI-A-1"
workspaces = [2, 4, 6, 8, 10]
position = "right-of eDP-1"
```

Поддерживаются `resolution`/`refresh_rate`/`enabled`. Пустая секция — все workspaces на primary.

## Звук и screen share

shtty автоматически стартует `pipewire`, `pipewire-pulse`, `wireplumber` (от имени пользователя) и бэкенд xdg-desktop-portal — screen share в OBS/Discord через портал (`org.freedesktop.impl.portal.desktop.shtty`).

## Сборка пакетов

```bash
./build-packages.sh   # .deb, .rpm, pacman-пакет из текущего тега
```

Готовые спеки: `debian/`, `packaging/rpm/shtty.spec`, `packaging/arch/PKGBUILD`.

## Структура проекта

```
src/
  main.rs            — privsep: login fork, WM event loop, IPC handlers
  login/             — login screen + privilege separation + PAM FFI
  drm/               — DRM/KMS backend, planes, hardware cursor, multi-monitor
  render/            — canvas, шрифты (TTF/PSF + fallback chain), текст
  layout/            — тайловая раскладка, workspaces
  input/             — evdev клавиатура/мышь/геймпад
  x11/               — Xvfb/Xephyr композитор, DRI3, dmabuf
  term/              — PTY + vterm (xterm-совместимый эмулятор)
  ui/                — тема, бар, popups
  config/            — TOML конфиг, live-reload watcher, window rules
  launcher/          — .desktop сканер + рендер
  audio/, portal/    — PipeWire, xdg-desktop-portal
  ipc/               — UNIX-сокет сервер
  bin/shtty_msg.rs   — CLI для IPC
```

## Устранение неполадок

**Чёрный экран после входа.** Смотрите `journalctl -u shtty@tty1 -b`. Если WM падал 3+ раза за 60 секунд, он сам откатывается на getty (`/run/shtty-crashes` хранит таймстампы падений; `sudo rm /run/shtty-crashes` для сброса).

**Иконки/символы — `?` или квадраты.** Основной шрифт не содержит этих глифов, а fallback не нашёл источник. Установите `ttf-nerd-fonts-symbols`/`noto-fonts` (Arch) или положите любой .ttf в `/etc/shtty/font-fallbacks/`, затем перезапустите WM. В журнале строка `font fallback chain: N additional font(s)` подтверждает подключение.

**Кириллица — `?`.** Системный monospace TTF без кириллицы. Установите `ttf-dejavu`/`fonts-dejavu`.

**X11-окно не реагирует на клавиатуру.** Кликните по его плитке — фокус синхронизируется с X-сервером через `set_input_focus`.

**Сломалась графика на NVIDIA.** Добавьте `nvidia-drm.modeset=1` в `GRUB_CMDLINE_LINUX_DEFAULT` и пересоберите grub.

**Вернуться на обычный TTY.** Ctrl+Alt+F2 — стандартный getty на tty2 остаётся доступен.

## Лицензия

MIT — см. [LICENSE](LICENSE).
