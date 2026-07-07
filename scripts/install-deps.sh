#!/usr/bin/env bash
# Install the system dependencies tv-shell needs: the Hyprland compositor, the
# Quickshell QML runtime, a Rust toolchain, and the daemon's build libs
# (libudev + pkg-config; the cec feature static-links its own libcec).
#
# Best-effort and distro-aware. It covers the common package-manager bits; where
# a dependency isn't packaged (Quickshell on some distros, Moonlight Qt), it
# prints exactly what you still need rather than guessing. Run it before
# scripts/install.sh. See docs/INSTALL.md for the full picture.
#
# Two tiers:
#   - Runtime deps (always): Hyprland, Quickshell, Qt6, Rust, build libs, grim,
#     socat — what the shell can't run without.
#   - Optional apps (--with-apps): the launchable apps the shell's app-launcher
#     surfaces on the home screen — chromium, Moonlight, Plex HTPC, Spotify, and
#     VacuumTube (YouTube's 10-foot "leanback" TV UI). The shell runs fine
#     without them; they're home-screen entries, not dependencies.
#
# Usage: sudo ./scripts/install-deps.sh [--with-apps]
set -euo pipefail

log() { echo "deps: $*"; }
note() { echo "deps: NOTE: $*" >&2; }

WITH_APPS=0
for arg in "$@"; do
    case "$arg" in
        --with-apps) WITH_APPS=1 ;;
        -h | --help)
            echo "Usage: sudo $0 [--with-apps]"
            echo "  --with-apps   also install the optional launchable apps"
            echo "                (chromium, Moonlight, Plex HTPC, Spotify, VacuumTube)"
            exit 0
            ;;
        *) note "ignoring unknown argument: $arg" ;;
    esac
done

[ "$(id -u)" -eq 0 ] || {
    echo "deps: run as root (installs packages) — use sudo" >&2
    exit 1
}

ID=""
[ -r /etc/os-release ] && . /etc/os-release && ID="${ID:-}"

# --- optional launchable apps (--with-apps) --------------------------------

# Ensure flatpak + the Flathub remote are present; return non-zero (with a hint)
# if flatpak itself isn't installed so the caller can skip the Flatpak apps.
ensure_flatpak_flathub() {
    if ! command -v flatpak >/dev/null 2>&1; then
        note "flatpak not installed — install it to add the Flatpak apps, then re-run:"
        case "$1" in
            arch | cachyos | endeavouros | manjaro) note "  pacman -S flatpak" ;;
            fedora) note "  dnf install -y flatpak" ;;
            *) note "  install flatpak via your package manager" ;;
        esac
        return 1
    fi
    flatpak remote-add --if-not-exists flathub \
        https://flathub.org/repo/flathub.flatpakrepo 2>/dev/null || true
}

install_optional_apps() {
    local id="$1"
    log "installing optional launchable apps (--with-apps)"

    # chromium — from the official repos (kiosk / web-app launching).
    case "$id" in
        arch | cachyos | endeavouros | manjaro)
            pacman -S --needed --noconfirm chromium || note "chromium install failed" ;;
        fedora) dnf install -y chromium || note "chromium install failed" ;;
        *) note "install chromium via your package manager" ;;
    esac

    # GUI apps via Flathub: Moonlight, Plex HTPC, Spotify, VacuumTube (YouTube TV).
    if ensure_flatpak_flathub "$id"; then
        flatpak install -y --noninteractive flathub \
            com.moonlight_stream.Moonlight \
            tv.plex.PlexHTPC \
            com.spotify.Client \
            rocks.shy.VacuumTube || note "one or more Flatpak apps failed to install"

        # VacuumTube (YouTube leanback) renders best on native Wayland — without
        # this it falls back to XWayland and loses HDR. Force the Electron Ozone
        # Wayland hint so it can submit HDR to the compositor's color management.
        # (HDR itself is opt-in inside VacuumTube via its `wayland_hdr` setting,
        # and needs an HDR-capable display already in HDR mode.)
        flatpak override --system --env=ELECTRON_OZONE_PLATFORM_HINT=auto rocks.shy.VacuumTube \
            2>/dev/null || note "could not set VacuumTube Wayland override"
    fi

    # The shell's in-app Moonlight streaming path shells out to a native
    # `moonlight` on PATH (see the System Integration table) — the Flatpak above
    # only provides the GUI client. Install the native package too if you stream
    # from inside the shell.
    case "$id" in
        arch | cachyos | endeavouros | manjaro)
            note "Moonlight streaming CLI: yay -S moonlight-qt (AUR) for a native 'moonlight' on PATH" ;;
        fedora)
            note "Moonlight streaming CLI: install moonlight-qt for a native 'moonlight' on PATH" ;;
    esac
}

# --- runtime dependencies (always) -----------------------------------------

case "$ID" in
    arch | cachyos | endeavouros | manjaro)
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
        # systemd-devel provides the libudev headers + libudev.pc on Fedora
        # (there is no separate libudev-devel package) — the daemon's libudev-sys
        # build needs those plus pkgconf. The cec feature static-links its libcec.
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
        [ "$WITH_APPS" -eq 1 ] && note "  - --with-apps: chromium, Moonlight, Plex HTPC, Spotify, VacuumTube"
        exit 1
        ;;
esac

if [ "$WITH_APPS" -eq 1 ]; then
    install_optional_apps "$ID"
fi

log "done. Next: sudo ./scripts/install.sh"
