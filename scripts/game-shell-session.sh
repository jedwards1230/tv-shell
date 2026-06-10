#!/bin/bash
# Game Shell session wrapper — launched by SDDM via .desktop file.
# Starts the input daemon, then hands off to Hyprland which auto-starts Quickshell.

SHELL_DIR="${GAME_SHELL_DIR:-/opt/game-shell}"

export XDG_CURRENT_DESKTOP=Hyprland
# Export the resolved install dir so PATH-based hyprland binds (super-intent.sh)
# and the daemon (re-exec target) can find the install tree without a literal.
export GAME_SHELL_DIR="$SHELL_DIR"
# Streaming targets live under the per-user config dir by default.
export GAME_SHELL_TARGETS="${GAME_SHELL_TARGETS:-${XDG_CONFIG_HOME:-$HOME/.config}/game-shell/targets.json}"
export GAME_SHELL_SOCK="/run/user/$(id -u)/game-shell-input.sock"
# Put the install tree's scripts on PATH so hyprland `exec` binds resolve
# super-intent.sh (and friends) by name regardless of install prefix.
export PATH="$SHELL_DIR/scripts:$PATH"

# Optional per-machine daemon overrides (HTTP bridge bind, auth toggle, etc.) —
# not tracked in the repo so a box can opt into the LAN HTTP bridge locally.
DAEMON_ENV="${XDG_CONFIG_HOME:-$HOME/.config}/game-shell/daemon.env"
if [ -f "$DAEMON_ENV" ]; then set -a; . "$DAEMON_ENV"; set +a; fi

# Start the Rust input/backend daemon. It is the sole backend: gamepad
# grab/release, settings I/O, app discovery, Bluetooth/network/power, Hyprland
# reads, and Sunshine pre-flight all flow through it. Build with
# `cargo build --release` and install to `$SHELL_DIR/bin/game-shell-input`.
"${GAME_SHELL_INPUT_BIN:-$SHELL_DIR/bin/game-shell-input}" &
INPUT_PID=$!

cleanup() {
    kill "$INPUT_PID" 2>/dev/null
    wait "$INPUT_PID" 2>/dev/null
}
trap cleanup EXIT

# Hyprland 0.55+ prefers launching via the start-hyprland wrapper (it sets up the
# --watchdog-fd); launching Hyprland directly prints a startup warning (#198).
# start-hyprland forwards everything after `--` to Hyprland, so the config path
# rides through unchanged. Fall back to a direct launch on older Hyprland that
# ships no wrapper.
if command -v start-hyprland >/dev/null 2>&1; then
    exec start-hyprland -- -c "$SHELL_DIR/config/hyprland.conf"
else
    exec Hyprland -c "$SHELL_DIR/config/hyprland.conf"
fi
