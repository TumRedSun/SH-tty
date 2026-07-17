#!/usr/bin/env bash
# build-packages.sh — собирает 3 пакета (pacman/.deb/.rpm) из текущего исходника.
#
# Использование:
#   ./build-packages.sh            # собрать все 3 пакета
#   ./build-packages.sh pacman     # только pacman
#   ./build-packages.sh deb        # только .deb
#   ./build-packages.sh rpm        # только .rpm
#
# Требования:
#   - cargo/rust (для сборки бинарника)
#   - makepkg (Arch, для pacman пакета)
#   - cargo-deb (cargo install cargo-deb, для .deb пакета)
#   - cargo-rpm (cargo install cargo-rpm, для .rpm пакета)
#   - Или: fpm как универсальный fallback

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PKG_DIR="${SCRIPT_DIR}/target/packages"
mkdir -p "${PKG_DIR}"

# Версия из Cargo.toml (автоматически, не hardcoded).
VERSION=$(grep '^version' "${SCRIPT_DIR}/Cargo.toml" | head -1 | sed 's/.*"\(.*\)".*/\1/')
PKGNAME="superhot-tty"

echo_blue()   { echo -e "\e[34m$*\e[0m"; }
echo_green()  { echo -e "\e[32m$*\e[0m"; }
echo_yellow() { echo -e "\e[33m$*\e[0m"; }
echo_red()    { echo -e "\e[31m$*\e[0m"; }

# 1. Сначала собираем release-бинарник.
echo_blue "==> Building release binary..."
cd "${SCRIPT_DIR}"
if pkg-config --exists sdl2 2>/dev/null; then
    echo_green "SDL2 found, building with gamepad-sdl2"
    cargo build --release --features gamepad-sdl2
else
    cargo build --release
fi

BUILD_ARCH=$(uname -m)
echo_blue "==> Architecture: ${BUILD_ARCH}"

# 2. Pacman package (Arch Linux).
build_pacman() {
    echo_blue "==> Building pacman package..."
    if ! command -v makepkg >/dev/null 2>&1; then
        echo_yellow "makepkg not found — install pacman package"
        return 1
    fi
    local tmpdir
    tmpdir=$(mktemp -d)
    cp "${SCRIPT_DIR}/packaging/arch/PKGBUILD" "${tmpdir}/"
    cp "${SCRIPT_DIR}/packaging/arch/superhot-tty.install" "${tmpdir}/"
    # Создаём tarball исходников (без .git, target).
    local src_tar="${tmpdir}/${PKGNAME:-superhot-tty}-${VERSION}.tar.gz"
    tar czf "${src_tar}" \
        --exclude='.git' --exclude='target' --exclude='*.deb' --exclude='*.rpm' \
        -C "${SCRIPT_DIR}/.." "$(basename "${SCRIPT_DIR}")"
    # makepkg нужно запустить от non-root.
    cd "${tmpdir}"
    if [[ $EUID -eq 0 ]]; then
        chown -R nobody:nobody "${tmpdir}"
        sudo -u nobody makepkg -sf 2>&1 || return 1
    else
        makepkg -sf 2>&1 || return 1
    fi
    cp *.pkg.tar.zst "${PKG_DIR}/" 2>/dev/null || cp *.pkg.tar.xz "${PKG_DIR}/" 2>/dev/null || true
    rm -rf "${tmpdir}"
    echo_green "==> pacman package: ${PKG_DIR}/"
}

# 3. .deb package (Debian/Ubuntu).
build_deb() {
    echo_blue "==> Building .deb package..."
    if ! command -v cargo-deb >/dev/null 2>&1; then
        echo_yellow "cargo-deb not found, trying to install..."
        cargo install cargo-deb 2>&1 || { echo_red "Failed to install cargo-deb"; return 1; }
    fi
    cd "${SCRIPT_DIR}"
    cargo deb --output "${PKG_DIR}/superhot-tty_${VERSION}_amd64.deb" 2>&1 || {
        echo_yellow "cargo-deb failed, trying fpm fallback..."
        build_with_fpm "deb"
    }
    echo_green "==> .deb package: ${PKG_DIR}/superhot-tty_${VERSION}_amd64.deb"
}

# 4. .rpm package (Fedora/RHEL/openSUSE).
build_rpm() {
    echo_blue "==> Building .rpm package..."
    if ! command -v cargo-rpm >/dev/null 2>&1; then
        echo_yellow "cargo-rpm not found, trying to install..."
        cargo install cargo-rpm 2>&1 || { echo_red "Failed to install cargo-rpm"; return 1; }
    fi
    cd "${SCRIPT_DIR}"
    cargo rpm build 2>&1 || {
        echo_yellow "cargo-rpm failed, trying fpm fallback..."
        build_with_fpm "rpm"
    }
    # Копируем результат.
    find target/rpm -name "*.rpm" -exec cp {} "${PKG_DIR}/" \; 2>/dev/null || true
    echo_green "==> .rpm package: ${PKG_DIR}/"
}

# FPM fallback для .deb/.rpm.
build_with_fpm() {
    local fmt="$1"
    if ! command -v fpm >/dev/null 2>&1; then
        echo_red "fpm not found — install fpm or cargo-deb/cargo-rpm"
        return 1
    fi
    cd "${SCRIPT_DIR}"
    fpm -s dir -t "${fmt}" \
        -n superhot-tty \
        -v "${VERSION}" \
        --license MIT \
        --url "https://github.com/TumRedSun/SH-tty" \
        --description "SuperHot MCD-styled TTY window manager" \
        --depends systemd \
        --depends zsh \
        --depends xserver-xephyr \
        --depends pipewire \
        --depends pipewire-pulse \
        --depends wireplumber \
        --depends xdg-desktop-portal \
        --depends libpam0g \
        target/release/superhot-tty=/usr/local/bin/superhot-tty \
        systemd/superhot-tty@.service=/etc/systemd/system/superhot-tty@.service \
        config/default.toml=/etc/SH-tty/config.toml \
        skel/zshrc.example=/usr/share/SH-tty/skel/zshrc.example \
        -p "${PKG_DIR}/superhot-tty_${VERSION}.${fmt}"
}

# 5. Упаковка в один архив.
make_archive() {
    echo_blue "==> Creating combined archive..."
    cd "${PKG_DIR}"
    tar czf "superhot-tty-packages-${VERSION}.tar.gz" *.pkg.tar.zst *.deb *.rpm 2>/dev/null || \
    tar czf "superhot-tty-packages-${VERSION}.tar.gz" * 2>/dev/null || true
    echo_green "==> Combined: ${PKG_DIR}/superhot-tty-packages-${VERSION}.tar.gz"
}

# 6. Main.
TARGET="${1:-all}"
case "${TARGET}" in
    pacman|arch) build_pacman ;;
    deb) build_deb ;;
    rpm) build_rpm ;;
    all)
        build_pacman || true
        build_deb || true
        build_rpm || true
        make_archive
        ;;
    *)
        echo_red "Unknown target: ${TARGET}"
        echo "Usage: $0 [pacman|deb|rpm|all]"
        exit 1
        ;;
esac

echo_green "==> Done!"
ls -la "${PKG_DIR}/"
