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

# Per-machine daemon options (HTTP/MCP/CEC/Plex/Steam) are NOT sourced here
# anymore — the daemon reads a typed ~/.config/game-shell/config.toml directly,
# so the bearer token never enters this script's (or any child's) environment.

# Start the Rust input/backend daemon. It is the sole backend: gamepad
# grab/release, settings I/O, app discovery, Bluetooth/network/power, Hyprland
# reads, and Sunshine pre-flight all flow through it. Build with
# `cargo build --release` and install to `$SHELL_DIR/bin/game-shell-input`.
#
# Preferred path: run it as a `systemd --user` unit (game-shell-input.service).
# That gives journald log capture with unit metadata, cgroup CPU/mem accounting
# (node_exporter's systemd collector sees per-unit usage with zero app code), and
# single-instance + restart semantics (which also kills the recurring "duplicate
# quickshell instance"-style stacking from a re-launched session). The daemon
# self-discovers everything it needs (install root from its own path, socket,
# daemon.env), so the unit needs no env wiring from here.
#
# Fallback: if `systemctl --user` is unavailable (no user systemd manager, no
# user bus) OR a dev override `GAME_SHELL_INPUT_BIN` is set (the unit's ExecStart
# is the installed binary and can't honor an arbitrary override), launch the
# daemon as a bare background process exactly as before. The session must never
# be bricked by a missing user manager.
INPUT_PID=""
INPUT_UNIT=""
INPUT_BIN="${GAME_SHELL_INPUT_BIN:-$SHELL_DIR/bin/game-shell-input}"

if [ -z "${GAME_SHELL_INPUT_BIN:-}" ] \
    && command -v systemctl >/dev/null 2>&1 \
    && systemctl --user show-environment >/dev/null 2>&1; then
    # Stop any stale instance from a previous (crashed/un-cleaned) session so a
    # re-launch never stacks two daemons, then start fresh.
    systemctl --user reset-failed game-shell-input.service >/dev/null 2>&1 || true
    if systemctl --user start game-shell-input.service; then
        INPUT_UNIT="game-shell-input.service"
    fi
fi

if [ -z "$INPUT_UNIT" ]; then
    # Fallback: bare background process (legacy path).
    "$INPUT_BIN" &
    INPUT_PID=$!
fi

cleanup() {
    if [ -n "$INPUT_UNIT" ]; then
        systemctl --user stop "$INPUT_UNIT" >/dev/null 2>&1
    elif [ -n "$INPUT_PID" ]; then
        kill "$INPUT_PID" 2>/dev/null
        wait "$INPUT_PID" 2>/dev/null
    fi
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
