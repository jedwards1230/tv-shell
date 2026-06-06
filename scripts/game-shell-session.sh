#!/bin/bash
# Game Shell session wrapper — launched by SDDM via .desktop file.
# Starts the input daemon, then hands off to Hyprland which auto-starts Quickshell.

SHELL_DIR="${GAME_SHELL_DIR:-/opt/game-shell}"

export XDG_CURRENT_DESKTOP=Hyprland
export GAME_SHELL_TARGETS="${GAME_SHELL_TARGETS:-$SHELL_DIR/targets.yaml}"
export GAME_SHELL_SOCK="/run/user/$(id -u)/game-shell-input.sock"

# Optional per-machine daemon overrides (HTTP bridge bind, auth toggle, etc.) —
# not tracked in the repo so a box can opt into the LAN HTTP bridge locally.
DAEMON_ENV="${XDG_CONFIG_HOME:-$HOME/.config}/game-shell/daemon.env"
if [ -f "$DAEMON_ENV" ]; then set -a; . "$DAEMON_ENV"; set +a; fi

# Hand the CEC adapter to the daemon: the kernel pulse8-cec service (Bigscreen's
# CEC owner) holds /dev/ttyACM0 via inputattach; stop it so libcec can open the
# serial port. Restored on exit so the next Bigscreen login gets kernel CEC back.
# Best-effort: a host without pulse8-cec (or without the passwordless sudoers
# entry deployed by homelab-ansible) still launches the session cleanly.
if systemctl list-unit-files pulse8-cec.service >/dev/null 2>&1; then
    sudo -n systemctl stop pulse8-cec.service 2>/dev/null || true
fi

# Start the Rust input/backend daemon. It is the sole backend: gamepad
# grab/release, settings I/O, app discovery, Bluetooth/network/power, Hyprland
# reads, and Sunshine pre-flight all flow through it. Build with
# `cargo build --release` and install to `$SHELL_DIR/bin/game-shell-input`.
"$SHELL_DIR/bin/game-shell-input" &
INPUT_PID=$!

cleanup() {
    kill "$INPUT_PID" 2>/dev/null
    wait "$INPUT_PID" 2>/dev/null

    # Restore kernel CEC ownership for the next Bigscreen/Plasma login. Same
    # best-effort guards as the start-side stop above.
    if systemctl list-unit-files pulse8-cec.service >/dev/null 2>&1; then
        sudo -n systemctl start pulse8-cec.service 2>/dev/null || true
    fi
}
trap cleanup EXIT

exec Hyprland -c "$SHELL_DIR/config/hyprland.conf"
