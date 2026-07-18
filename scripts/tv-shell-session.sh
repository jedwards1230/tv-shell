#!/bin/bash
# TV Shell session wrapper — launched by SDDM via .desktop file.
# Starts the input daemon, then hands off to Hyprland which auto-starts Quickshell.

# Install root: TV_SHELL_DIR (legacy GAME_SHELL_DIR honored), default /opt/tv-shell.
SHELL_DIR="${TV_SHELL_DIR:-${GAME_SHELL_DIR:-/opt/tv-shell}}"

export XDG_CURRENT_DESKTOP=Hyprland
# Export the resolved install dir so PATH-based hyprland binds (super-intent.sh)
# and the daemon (re-exec target) can find the install tree without a literal.
# Both the current TV_SHELL_* and the legacy GAME_SHELL_* names are exported one
# release so a not-yet-updated consumer (an old Quickshell during a mid-migration
# git pull) still resolves them; drop the legacy exports next cycle.
export TV_SHELL_DIR="$SHELL_DIR"
export GAME_SHELL_DIR="$SHELL_DIR"
# Streaming targets live under the per-user config dir by default.
TARGETS_DEFAULT="${XDG_CONFIG_HOME:-$HOME/.config}/tv-shell/targets.json"
export TV_SHELL_TARGETS="${TV_SHELL_TARGETS:-${GAME_SHELL_TARGETS:-$TARGETS_DEFAULT}}"
export GAME_SHELL_TARGETS="$TV_SHELL_TARGETS"
export TV_SHELL_SOCK="/run/user/$(id -u)/tv-shell-input.sock"
export GAME_SHELL_SOCK="$TV_SHELL_SOCK"
# Put the install tree's scripts on PATH so hyprland `exec` binds resolve
# super-intent.sh (and friends) by name regardless of install prefix.
export PATH="$SHELL_DIR/scripts:$PATH"

# Per-machine daemon options (HTTP/MCP/CEC/Plex/Steam) are NOT sourced here
# anymore — the daemon reads a typed ~/.config/tv-shell/config.toml directly,
# so the bearer token never enters this script's (or any child's) environment.

# Start the Rust input/backend daemon. It is the sole backend: gamepad
# grab/release, settings I/O, app discovery, Bluetooth/network/power, Hyprland
# reads, and Sunshine pre-flight all flow through it. Build with
# `cargo build --release` and install to `$SHELL_DIR/bin/tv-shell-input`.
#
# Preferred path: run it as a `systemd --user` unit (tv-shell-input.service).
# That gives journald log capture with unit metadata, cgroup CPU/mem accounting
# (node_exporter's systemd collector sees per-unit usage with zero app code), and
# single-instance + restart semantics (which also kills the recurring "duplicate
# quickshell instance"-style stacking from a re-launched session). The daemon
# self-discovers everything it needs (install root from its own path, socket) and
# reads its typed config from ~/.config/tv-shell/config.toml directly, so the
# unit needs no env wiring from here.
#
# Fallback: if `systemctl --user` is unavailable (no user systemd manager, no
# user bus) OR a dev override `TV_SHELL_INPUT_BIN` (legacy `GAME_SHELL_INPUT_BIN`)
# is set (the unit's ExecStart is the installed binary and can't honor an
# arbitrary override), launch the daemon as a bare background process exactly as
# before. The session must never be bricked by a missing user manager.
INPUT_PID=""
INPUT_UNIT=""
INPUT_BIN_OVERRIDE="${TV_SHELL_INPUT_BIN:-${GAME_SHELL_INPUT_BIN:-}}"
INPUT_BIN="${INPUT_BIN_OVERRIDE:-$SHELL_DIR/bin/tv-shell-input}"

if [ -z "$INPUT_BIN_OVERRIDE" ] \
    && command -v systemctl >/dev/null 2>&1 \
    && systemctl --user show-environment >/dev/null 2>&1; then
    # Stop any stale instance from a previous (crashed/un-cleaned) session so a
    # re-launch never stacks two daemons, then start fresh.
    systemctl --user reset-failed tv-shell-input.service >/dev/null 2>&1 || true
    if systemctl --user start tv-shell-input.service; then
        INPUT_UNIT="tv-shell-input.service"
    fi
fi

if [ -z "$INPUT_UNIT" ]; then
    # Fallback: bare background process (legacy path).
    "$INPUT_BIN" &
    INPUT_PID=$!
fi

# Shared user-systemd-availability gate for the units below (Quickshell is
# started by Hyprland's exec-once, so its lifecycle is independent of the input
# daemon's dev-override path — gate purely on user-systemd availability). Clear
# any stale failed state from a previous (crashed/un-cleaned) session so this
# session's `start` isn't refused by a lingering StartLimit failure.
HAVE_USER_SYSTEMD=""
if command -v systemctl >/dev/null 2>&1 \
    && systemctl --user show-environment >/dev/null 2>&1; then
    HAVE_USER_SYSTEMD="1"
    systemctl --user reset-failed tv-shell-quickshell.service >/dev/null 2>&1 || true
fi

# The web control panel (tv-shell-panel) runs as its own `systemd --user` unit,
# started here right after the daemon — it is NOT compositor-dependent (unlike
# Quickshell below), so it doesn't need to wait for Hyprland. Best-effort and
# strictly non-fatal: the panel is a convenience surface, and the session must
# never be bricked by it failing to start (a missing binary, a bad [panel] bind
# in config.toml, port already in use, etc). No bare-process fallback — if user
# systemd isn't available the panel simply doesn't run this session.
if [ -n "$HAVE_USER_SYSTEMD" ]; then
    systemctl --user reset-failed tv-shell-panel.service >/dev/null 2>&1 || true
    systemctl --user start tv-shell-panel.service >/dev/null 2>&1 || true
fi

# Quickshell runs as its own `systemd --user` unit (tv-shell-quickshell.service),
# started by Hyprland's exec-once once the compositor is up (see config/
# hyprland.conf).

cleanup() {
    if [ -n "$INPUT_UNIT" ]; then
        systemctl --user stop "$INPUT_UNIT" >/dev/null 2>&1
    elif [ -n "$INPUT_PID" ]; then
        kill "$INPUT_PID" 2>/dev/null
        wait "$INPUT_PID" 2>/dev/null
    fi
    # Quickshell and the panel are both started outside this trap (Quickshell by
    # the compositor's exec-once, the panel above), but run under the user
    # manager and would outlive Hyprland — stop both on session exit so neither
    # can survive into (and race) the next session.
    if [ -n "$HAVE_USER_SYSTEMD" ]; then
        systemctl --user stop tv-shell-quickshell.service >/dev/null 2>&1 || true
        systemctl --user stop tv-shell-panel.service >/dev/null 2>&1 || true
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
