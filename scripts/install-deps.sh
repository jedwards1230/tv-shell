#!/usr/bin/env bash
# Install the system dependencies game-shell needs: the Hyprland compositor, the
# Quickshell QML runtime, a Rust toolchain, and the daemon's build libs
# (libudev + pkg-config; the cec feature static-links its own libcec).
#
# Best-effort and distro-aware. It covers the common package-manager bits; where
# a dependency isn't packaged (Quickshell on some distros, Moonlight Qt), it
# prints exactly what you still need rather than guessing. Run it before
# scripts/install.sh. See docs/INSTALL.md for the full picture.
#
# Usage: sudo ./scripts/install-deps.sh
set -euo pipefail

log() { echo "deps: $*"; }
note() { echo "deps: NOTE: $*" >&2; }

[ "$(id -u)" -eq 0 ] || { echo "deps: run as root (installs packages) — use sudo" >&2; exit 1; }

ID=""
[ -r /etc/os-release ] && . /etc/os-release && ID="${ID:-}"

case "$ID" in
    arch|cachyos|endeavouros|manjaro)
        log "detected Arch-family ($ID) — installing via pacman"
        # hyprland, qt6 + wayland runtime, rust, and the daemon build libs.
        pacman -S --needed --noconfirm \
            hyprland \
            qt6-base qt6-declarative qt6-wayland \
            wayland \
            rust cargo \
            systemd-libs pkgconf \
            grim socat
        note "Quickshell isn't in the official repos — install it from the AUR:"
        note "  yay -S quickshell   (or quickshell-git)"
        note "Moonlight Qt for streaming: yay -S moonlight-qt (AUR) or moonlight-qt (flatpak)"
        ;;
    fedora)
        log "detected Fedora — installing build/runtime deps via dnf"
        dnf install -y \
            qt6-qtbase qt6-qtdeclarative qt6-qtwayland \
            wayland-devel \
            cargo rust \
            systemd-devel pkgconf-pkg-config \
            grim socat
        note "Hyprland: enable a COPR (e.g. solopasha/hyprland) or build from source."
        note "Quickshell: build from source — https://quickshell.org/docs/"
        note "Moonlight Qt: flatpak install flathub com.moonlight_stream.Moonlight"
        ;;
    *)
        note "unrecognized distro '${ID:-unknown}'. Install these yourself:"
        note "  - Hyprland (Wayland compositor)"
        note "  - Quickshell (QML runtime)            https://quickshell.org"
        note "  - Rust toolchain (cargo, rustc >= 1.75)"
        note "  - libudev + pkg-config (daemon build)"
        note "  - grim, socat (screenshots, super-key socket)"
        note "  - Moonlight Qt (game streaming client)"
        exit 1
        ;;
esac

log "done. Next: sudo ./scripts/install.sh"
