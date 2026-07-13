Name:       superhot-tty
Version:    0.3.0
Release:    1%{?dist}
Summary:    SuperHot MCD-styled TTY window manager
License:    MIT
URL:        https://github.com/TumRedSun/SH-tty
Source0:    https://github.com/TumRedSun/SH-tty/archive/v%{version}.tar.gz

BuildRequires:  rust >= 1.70
BuildRequires:  cargo
BuildRequires:  gcc
BuildRequires:  pkgconf-pkg-config
BuildRequires:  pam-devel
Requires:       systemd
Requires:       kbd
Requires:       zsh
Requires:       xorg-x11-server-Xephyr
Requires:       pipewire
Requires:       pipewire-pulse
Requires:       wireplumber
Requires:       xdg-desktop-portal
Requires:       pam
Recommends:     SDL2-devel
Recommends:     xdg-desktop-portal-gtk

%description
SuperHot TTY is a tile-based window manager for the Linux console,
styled after SuperHot: Mind Control Delete. It replaces agetty on tty1
and works directly with DRM/KMS (no X11/Wayland backend).

Features:
  * Tile-based BSP/i3 layout
  * 10 workspaces with multi-monitor binding
  * Rofi-like launcher (Super+D)
  * X11 window embedding via Xephyr + XComposite
  * DRI3+DMA-BUF GPU acceleration infrastructure
  * PipeWire audio stack
  * xdg-desktop-portal ScreenCast backend
  * Mouse + gamepad (evdev + optional SDL2)
  * TOML configuration (~/.config/SH-tty/config.toml)
  * PAM login screen with MCD theming
  * Window rules for automatic placement
  * Autostart commands

%prep
%setup -q -n SH-tty-%{version}

%build
if pkg-config --exists sdl2 2>/dev/null; then
    cargo build --release --features gamepad-sdl2
else
    cargo build --release
fi

%install
# Binary.
install -Dm755 target/release/superhot-tty %{buildroot}%{_bindir}/superhot-tty

# Systemd unit.
install -Dm644 systemd/superhot-tty@.service %{buildroot}%{_unitdir}/superhot-tty@.service

# Default config.
install -Dm644 config/default.toml %{buildroot}%{_sysconfdir}/SH-tty/config.toml

# Example zshrc.
install -Dm644 skel/zshrc.example %{buildroot}%{_datadir}/SH-tty/skel/zshrc.example

# README + LICENSE.
install -Dm644 README.md %{buildroot}%{_docdir}/SH-tty/README.md
install -Dm644 LICENSE %{buildroot}%{_licensedir}/SH-tty/LICENSE

%post
echo ""
echo "==> SuperHot TTY installed!"
echo ""
echo "To enable:"
echo "  sudo systemctl disable getty@tty1"
echo "  sudo systemctl enable superhot-tty@tty1"
echo "  sudo reboot"
echo ""
echo "Config: /etc/SH-tty/config.toml"
echo "User config: ~/.config/SH-tty/config.toml"
echo ""

%preun
if [ $1 -eq 0 ]; then
    # Removal — откат к getty.
    if systemctl is-enabled superhot-tty@tty1 >/dev/null 2>&1; then
        systemctl disable superhot-tty@tty1 || true
    fi
    if [ -f /etc/systemd/system/getty@tty1.service.d/override.conf ]; then
        rm -f /etc/systemd/system/getty@tty1.service.d/override.conf
        systemctl enable getty@tty1 2>/dev/null || true
    fi
    systemctl daemon-reload || true
fi

%files
%{_bindir}/superhot-tty
%{_unitdir}/superhot-tty@.service
%config(noreplace) %{_sysconfdir}/SH-tty/config.toml
%{_datadir}/SH-tty/skel/zshrc.example
%{_docdir}/SH-tty/README.md
%{_licensedir}/SH-tty/LICENSE

%changelog
* Sat Jul 12 2026 SuperHot TTY contributors <superhot-tty@users.noreply.github.com> - 0.3.0-1
- v0.3: window rules, multi-monitor, login screen, autostart, 3 packages
- v0.2: TOML config, workspaces, launcher, mouse, gamepad, PipeWire, portal
- v0.1: initial release
