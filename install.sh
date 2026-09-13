#!/usr/bin/env bash
# install.sh — установка shtty (тайловый TTY window manager) на Arch Linux.
# Запускать от root: sudo ./install.sh
#
# Что делает:
#   - Создаёт системного пользователя 'shtty' для privilege separation
#   - Собирает и ставит бинарник + systemd unit
#   - TOML конфиг /etc/shtty/config.toml
#   - Каталог /etc/shtty/font-fallbacks/ для символ-шрифтов (иконки)
#   - Отключает getty@tty1 и включает shtty@tty1

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

echo_red()   { echo -e "\e[31m$*\e[0m"; }
echo_green() { echo -e "\e[32m$*\e[0m"; }
echo_yellow(){ echo -e "\e[33m$*\e[0m"; }
echo_blue()  { echo -e "\e[34m$*\e[0m"; }

if [[ $EUID -ne 0 ]]; then
    echo_red "Run as root: sudo ./install.sh"
    exit 1
fi

echo_blue "==> Creating 'shtty' system user for privilege separation..."
# The login screen runs as this unprivileged user. PAM auth happens in the
# root parent process via fork+socketpair. The user needs:
#   - video, render, input groups: to access DRM/input devices inherited from root
#   - tty group: to use the controlling terminal
#   - NOT shadow group: prevents direct /etc/shadow reads (auth goes via parent)
if ! id "shtty" &>/dev/null; then
    useradd --system \
        --no-create-home \
        --home-dir / \
        --shell /usr/sbin/nologin \
        --groups video,input,render,tty \
        --comment "shtty login screen user" \
        shtty
    echo_green "Created system user 'shtty'"
else
    echo_yellow "User 'shtty' already exists — ensuring group membership"
    for grp in video input render tty; do
        if ! id -nG shtty | grep -qw "$grp"; then
            usermod -aG "$grp" shtty
        fi
    done
fi

echo_blue "==> Checking dependencies..."
# Основные зависимости.
DEPS=(
    rust
    cargo
    gcc
    pkgconf
    systemd
    # X11 встраивание:
    xorg-server-xvfb
    xorg-server-xephyr  # fallback if Xvfb is unavailable
    # Звук:
    pipewire
    pipewire-pulse
    wireplumber
    # Portal для screen share:
    xdg-desktop-portal
    # Шрифты: freetype + fontconfig для TTF рендеринга (через fc-match находит
    # системный monospace TTF — DejaVu Sans Mono, JetBrains Mono, etc.).
    # kbd — для PSF fallback если TTF недоступен.
    freetype2
    fontconfig
    kbd
    # Символ-шрифты: Nerd Font иконки (терминалы, prompt) + базовое покрытие
    # символов. Без них Nerd Font-иконки рендерятся как '?'.
    ttf-dejavu
    ttf-nerd-fonts-symbols
    noto-fonts
    # zsh по умолчанию:
    zsh
)
# Опциональные зависимости (warn если нет, но продолжаем).
OPT_DEPS=(
    "libsdl2-dev:gamepad-sdl2 фичи (маппинг кнопок вне Steam)"
    "xdg-desktop-portal-gtk:GTK file chooser portal"
    "xdg-desktop-portal-gnome:GNOME portal integration"
)

missing=()
for dep in "${DEPS[@]}"; do
    if ! pacman -Qi "$dep" >/dev/null 2>&1; then
        missing+=("$dep")
    fi
done
if [[ ${#missing[@]} -gt 0 ]]; then
    echo_yellow "Missing dependencies: ${missing[*]}"
    read -rp "Install them now via pacman? [y/N] " ans
    if [[ "${ans,,}" == "y" ]]; then
        pacman -Sy --noconfirm --needed "${missing[@]}"
    else
        echo_red "Cannot continue without dependencies."
        exit 1
    fi
fi

echo_blue "==> Checking optional dependencies..."
for entry in "${OPT_DEPS[@]}"; do
    pkg="${entry%%:*}"
    desc="${entry##*:}"
    if ! pacman -Qi "$pkg" >/dev/null 2>&1 && ! pkg-config --exists "$pkg" 2>/dev/null; then
        echo_yellow "Optional: $pkg ($desc) — not installed"
    fi
done

# SDL2 detection.
SDL2_FEATURE=""
if pkg-config --exists sdl2 2>/dev/null; then
    SDL2_FEATURE="--features gamepad-sdl2"
    echo_green "SDL2 found — enabling gamepad-sdl2 feature"
else
    echo_yellow "SDL2 not found — gamepad will use evdev passthrough (Steam Input works natively)"
    echo_yellow "  To enable SDL2 mapping: install libsdl2-dev and rebuild with --features gamepad-sdl2"
fi

# PAM detection — enables real PAM authentication via libpam.
# Without this, the WM falls back to a crypt()-based auth that reads
# /etc/shadow directly. The crypt() fallback works for most cases, but:
#   - It bypasses pam_unix's account/session modules
#   - On systems where crypt() doesn't support the hash algorithm
#     (e.g. yescrypt on older libxcrypt), auth silently fails
#   - PAM is the standard Linux auth stack and handles all edge cases
# So we enable it whenever libpam is available.
PAM_FEATURE=""
if pkg-config --exists pam 2>/dev/null || [[ -f /usr/lib/libpam.so || -f /usr/lib/x86_64-linux-gnu/libpam.so || -f /usr/lib64/libpam.so ]]; then
    PAM_FEATURE="--features pam"
    echo_green "PAM (libpam) found — enabling pam feature for real PAM authentication"
else
    echo_yellow "PAM (libpam) not found — using crypt() fallback (reads /etc/shadow directly)"
    echo_yellow "  For full PAM support: install pam/libpam0g-dev and rebuild with --features pam"
fi

ALL_FEATURES="$SDL2_FEATURE $PAM_FEATURE"

echo_blue "==> Building shtty (release)..."
cd "$SCRIPT_DIR"
cargo build --release $ALL_FEATURES

echo_blue "==> Installing binaries to /usr/local/bin..."
install -Dm755 target/release/shtty /usr/local/bin/shtty
install -Dm755 target/release/shtty-msg /usr/local/bin/shtty-msg

echo_blue "==> Installing systemd unit..."
install -Dm644 systemd/shtty@.service /etc/systemd/system/shtty@.service

echo_blue "==> Installing default config..."
install -d -m755 /etc/shtty
# Каталог для пользовательских fallback-шрифтов: любые .ttf/.otf отсюда
# подключаются как источник недостающих символов (иконки и т.п.).
install -d -m755 /etc/shtty/font-fallbacks
# Всегда перезаписываем config.toml последней версией.
# Пользовательские настройки могут быть в ~/.config/shtty/config.toml
install -Dm644 config/default.toml /etc/shtty/config.toml
echo_green "Installed /etc/shtty/config.toml (updated)"

# Дизейблим стандартный getty на tty1.
echo_blue "==> Disabling default getty on tty1..."
# На случай если crash-loop protection ранее включил getty и дизейблил наш сервис —
# принудительно возвращаемся в нормальное состояние.
systemctl stop getty@tty1.service 2>/dev/null || true
systemctl disable getty@tty1.service 2>/dev/null || true
# Удаляем старый override.conf (если был) и создаём новый, который
# превращает getty@tty1 в no-op (ExecStart=-/bin/false).
rm -rf /etc/systemd/system/getty@tty1.service.d
mkdir -p /etc/systemd/system/getty@tty1.service.d
cat > /etc/systemd/system/getty@tty1.service.d/override.conf <<'EOF'
[Service]
ExecStart=
ExecStart=-/bin/false
EOF

# Включаем наш unit (force — снимает previous disabled state).
echo_blue "==> Enabling shtty@tty1..."
# Очищаем crash state file — если были предыдущие падения, не хотим
# сразу попасть в crash loop detection при первом запуске после install.
rm -f /run/shtty-crashes
systemctl daemon-reload
# systemctl enable может вернуть ошибку если unit уже enabled — это ОК.
# Используем --force чтобы переустановить symlink'и (полезно если что-то
# было в неconsistente состоянии после crash-loop).
systemctl enable shtty@tty1.service 2>&1 || true
# Reset failure state — иначе systemd может отказаться стартовать из-за
# старых failed попыток (StartLimitHit).
systemctl reset-failed shtty@tty1.service 2>/dev/null || true

# Kernel cmdline для DRM modeset.
echo_blue "==> Checking kernel cmdline for DRM modeset..."
if [[ -f /etc/default/grub ]]; then
    if ! grep -q "nvidia-drm.modeset=1" /etc/default/grub; then
        echo_yellow "WARNING: nvidia-drm.modeset=1 не найден в /etc/default/grub"
        echo_yellow "Если у вас NVIDIA, добавьте 'nvidia-drm.modeset=1' в GRUB_CMDLINE_LINUX_DEFAULT"
        echo_yellow "и обновите grub: sudo grub-mkconfig -o /boot/grub/grub.cfg"
    fi
fi

# User groups.
echo_blue "==> Checking user groups..."
if [[ -n "${SUDO_USER:-}" ]]; then
    for grp in video input render audio; do
        if ! id -nG "$SUDO_USER" | grep -qw "$grp"; then
            echo_yellow "Adding $SUDO_USER to $grp group..."
            usermod -aG "$grp" "$SUDO_USER"
        fi
    done
fi

# PipeWire systemd user services (для звука).
echo_blue "==> Enabling PipeWire user services..."
if [[ -n "${SUDO_USER:-}" ]]; then
    sudo -u "$SUDO_USER" systemctl --user enable pipewire.service pipewire-pulse.service wireplumber.service 2>/dev/null || true
    sudo -u "$SUDO_USER" systemctl --user enable xdg-desktop-portal.service 2>/dev/null || true
fi

echo_green "==> Installation complete!"
echo ""
echo_blue "Next steps:"
echo "  1. Перезагрузитесь: sudo reboot"
echo "  2. На tty1 автоматически запустится shtty: логин → пароль → Enter"
echo "  3. Для переключения на обычный TTY: Ctrl+Alt+F2"
echo "  4. Mod4+D — launcher (читает .desktop файлы)"
echo "  5. Mod4+1..9 — workspaces"
echo "  6. Mod4+Enter — новый терминал (zsh)"
echo "  7. Mod4+E — открыть X11 плитку (для ручного запуска: DISPLAY=:1 discord)"
echo "  8. Mod4+Shift+1..9 — переместить окно на другой workspace"
echo "  9. Mod4+R — resize mode (HJKL)"
echo ""
echo_blue "Configuration:"
echo "  /etc/shtty/config.toml  — основной конфиг"
echo "  /etc/shtty/font.ttf     — кастомный шрифт (опционально)"
echo "  /etc/shtty/font-fallbacks/ — .ttf/.otf файлы с недостающими символами (иконки)"
echo ""
echo_blue "Audio (PipeWire):"
echo "  pactl set-sink-volume @DEFAULT_SINK@ 80%   — громкость"
echo "  pactl set-sink-mute @DEFAULT_SINK@ toggle — mute"
echo ""
echo_blue "Screen share:"
echo "  Discord/Slack: выберите 'shtty' в источниках экрана"
echo "  OBS: добавьте ScreenCast source (через xdg-desktop-portal)"
echo ""
echo_blue "Gamepad:"
echo "  Steam Input работает нативно (evdev passthrough)"
echo "  Для маппинга кнопок вне Steam: cargo build --features gamepad-sdl2"
echo ""
echo_yellow "Если что-то сломалось — Ctrl+Alt+F2 для обычного getty, и:"
echo_yellow "  sudo systemctl disable shtty@tty1"
echo_yellow "  sudo systemctl enable getty@tty1"
echo_yellow "  sudo rm /etc/systemd/system/getty@tty1.service.d/override.conf"
echo_yellow "  sudo systemctl daemon-reload && sudo reboot"
