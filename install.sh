#!/usr/bin/env bash
# install.sh — установка superhot-tty v0.2 на Arch Linux.
# Запускать от root: sudo ./install.sh
#
# Что нового в v0.2:
#   - TOML конфиг /etc/SH-tty/config.toml
#   - Workspaces 1-9 + перемещение окон между ними
#   - Launcher Super+D (rofi-подобный, читает .desktop файлы)
#   - Mouse + софтверный курсор MCD-стиля
#   - Gamepad (evdev passthrough для Steam + опционально SDL2 для маппинга)
#   - PipeWire audio stack
#   - xdg-desktop-portal backend для screen share в OBS/Discord
#   - DRI3/DMA-BUF GPU-ускорение X11 (infrastructure)
#   - zsh по умолчанию, TERM=xterm-256color

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

echo_blue "==> Creating 'superhot-tty' system user for privilege separation..."
# The login screen runs as this unprivileged user. PAM auth happens in the
# root parent process via fork+socketpair. The user needs:
#   - video, render, input groups: to access DRM/input devices inherited from root
#   - tty group: to use the controlling terminal
#   - NOT shadow group: prevents direct /etc/shadow reads (auth goes via parent)
if ! id "superhot-tty" &>/dev/null; then
    useradd --system \
        --no-create-home \
        --home-dir / \
        --shell /usr/sbin/nologin \
        --groups video,input,render,tty \
        --comment "superhot-tty login screen user" \
        superhot-tty
    echo_green "Created system user 'superhot-tty'"
else
    echo_yellow "User 'superhot-tty' already exists — ensuring group membership"
    for grp in video input render tty; do
        if ! id -nG superhot-tty | grep -qw "$grp"; then
            usermod -aG "$grp" superhot-tty
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
    xorg-server-xephyr
    # Звук:
    pipewire
    pipewire-pulse
    wireplumber
    # Portal для screen share:
    xdg-desktop-portal
    # Шрифты:
    kbd
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

echo_blue "==> Building superhot-tty v0.2 (release)..."
cd "$SCRIPT_DIR"
cargo build --release $SDL2_FEATURE

echo_blue "==> Installing binary to /usr/local/bin/superhot-tty..."
install -Dm755 target/release/superhot-tty /usr/local/bin/superhot-tty

echo_blue "==> Installing systemd unit..."
install -Dm644 systemd/superhot-tty@.service /etc/systemd/system/superhot-tty@.service

echo_blue "==> Installing default config..."
install -d -m755 /etc/SH-tty
# Всегда перезаписываем config.toml последней версией.
# Пользовательские настройки могут быть в ~/.config/SH-tty/config.toml
install -Dm644 config/default.toml /etc/SH-tty/config.toml
echo_green "Installed /etc/SH-tty/config.toml (updated)"

# Дизейблим стандартный getty на tty1.
echo_blue "==> Disabling default getty on tty1..."
systemctl disable getty@tty1.service 2>/dev/null || true
mkdir -p /etc/systemd/system/getty@tty1.service.d
cat > /etc/systemd/system/getty@tty1.service.d/override.conf <<'EOF'
[Service]
ExecStart=
ExecStart=-/bin/false
EOF

# Включаем наш unit.
echo_blue "==> Enabling superhot-tty@tty1..."
# Очищаем crash state file — если были предыдущие падения, не хотим
# сразу попасть в crash loop detection при первом запуске после install.
rm -f /run/superhot-tty-crashes
systemctl daemon-reload
systemctl enable superhot-tty@tty1.service

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
echo "  2. На tty1 автоматически запустится superhot-tty v0.2"
echo "  3. Для переключения на обычный TTY: Ctrl+Alt+F2"
echo "  4. Mod4+D — launcher (читает .desktop файлы)"
echo "  5. Mod4+1..9 — workspaces"
echo "  6. Mod4+Enter — новый терминал (zsh)"
echo "  7. Mod4+E — открыть X11 плитку (для ручного запуска: DISPLAY=:1 discord)"
echo "  8. Mod4+Shift+1..9 — переместить окно на другой workspace"
echo "  9. Mod4+R — resize mode (HJKL)"
echo ""
echo_blue "Configuration:"
echo "  /etc/SH-tty/config.toml  — основной конфиг"
echo "  /etc/SH-tty/font.psfu    — кастомный шрифт (опционально)"
echo ""
echo_blue "Audio (PipeWire):"
echo "  pactl set-sink-volume @DEFAULT_SINK@ 80%   — громкость"
echo "  pactl set-sink-mute @DEFAULT_SINK@ toggle — mute"
echo ""
echo_blue "Screen share:"
echo "  Discord/Slack: выберите 'SuperHot' в источниках экрана"
echo "  OBS: добавьте ScreenCast source (через xdg-desktop-portal)"
echo ""
echo_blue "Gamepad:"
echo "  Steam Input работает нативно (evdev passthrough)"
echo "  Для маппинга кнопок вне Steam: cargo build --features gamepad-sdl2"
echo ""
echo_yellow "Если что-то сломалось — Ctrl+Alt+F2 для обычного getty, и:"
echo_yellow "  sudo systemctl disable superhot-tty@tty1"
echo_yellow "  sudo systemctl enable getty@tty1"
echo_yellow "  sudo rm /etc/systemd/system/getty@tty1.service.d/override.conf"
echo_yellow "  sudo systemctl daemon-reload && sudo reboot"
