Name:       shtty
Version:    0.6.0
Release:    1%{?dist}
Summary:    Tile-based TTY window manager with X11 embedding
License:    MIT
URL:        https://github.com/TumRedSun/SH-tty
Source0:    https://github.com/TumRedSun/SH-tty/archive/v%{version}.tar.gz

BuildRequires:  rust >= 1.70
BuildRequires:  cargo
BuildRequires:  gcc
BuildRequires:  pkgconf-pkg-config
BuildRequires:  pam-devel
BuildRequires:  freetype-devel
BuildRequires:  fontconfig-devel
Requires:       systemd
Requires:       kbd
Requires:       zsh
Requires:       xorg-x11-server-Xvfb
Requires:       pipewire
Requires:       pipewire-pulse
Requires:       wireplumber
Requires:       xdg-desktop-portal
Requires:       pam
Requires:       freetype
Requires:       fontconfig
Requires:       dejavu-sans-mono-fonts
Recommends:     SDL2-devel
Recommends:     xdg-desktop-portal-gtk

%description
SHTTY is a tile-based window manager for the Linux console.
It replaces agetty on tty1 and works directly with DRM/KMS
(no X11/Wayland backend required).

Features:
  * Tile-based BSP/i3 layout
  * 10 workspaces with multi-monitor binding
  * Rofi-like launcher (Super+D)
  * X11 window embedding via Xvfb + XComposite
  * DRI3+DMA-BUF GPU acceleration infrastructure
  * PipeWire audio stack
  * xdg-desktop-portal ScreenCast backend
  * Mouse + gamepad (evdev + optional SDL2)
  * TOML configuration (~/.config/shtty/config.toml)
  * PAM login screen (login → password → Enter)
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
install -Dm755 target/release/shtty %{buildroot}%{_bindir}/shtty

# Systemd unit.
install -Dm644 systemd/shtty@.service %{buildroot}%{_unitdir}/shtty@.service

# Default config.
install -Dm644 config/default.toml %{buildroot}%{_sysconfdir}/shtty/config.toml

# Example zshrc.
install -Dm644 skel/zshrc.example %{buildroot}%{_datadir}/shtty/skel/zshrc.example

# README + LICENSE.
install -Dm644 README.md %{buildroot}%{_docdir}/shtty/README.md
install -Dm644 LICENSE %{buildroot}%{_licensedir}/shtty/LICENSE

%post
echo ""
echo "==> SHTTY installed!"
echo ""
echo "To enable:"
echo "  sudo systemctl disable getty@tty1"
echo "  sudo systemctl enable shtty@tty1"
echo "  sudo reboot"
echo ""
echo "Config: /etc/shtty/config.toml"
echo "User config: ~/.config/shtty/config.toml"
echo ""

%preun
if [ $1 -eq 0 ]; then
    # Removal — откат к getty.
    if systemctl is-enabled shtty@tty1 >/dev/null 2>&1; then
        systemctl disable shtty@tty1 || true
    fi
    if [ -f /etc/systemd/system/getty@tty1.service.d/override.conf ]; then
        rm -f /etc/systemd/system/getty@tty1.service.d/override.conf
        systemctl enable getty@tty1 2>/dev/null || true
    fi
    systemctl daemon-reload || true
fi

%files
%{_bindir}/shtty
%{_unitdir}/shtty@.service
%config(noreplace) %{_sysconfdir}/shtty/config.toml
%{_datadir}/shtty/skel/zshrc.example
%{_docdir}/shtty/README.md
%{_licensedir}/shtty/LICENSE

%changelog
* Sat Jul 12 2026 SHTTY contributors <shtty@users.noreply.github.com> - 0.6.0-1
- v0.3: window rules, multi-monitor, login screen, autostart, 3 packages
- v0.2: TOML config, workspaces, launcher, mouse, gamepad, PipeWire, portal
- v0.1: initial release
