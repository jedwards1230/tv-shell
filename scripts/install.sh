#!/usr/bin/env bash
# Install game-shell on a Linux machine: build the input daemon, lay the install
# tree under a prefix, register the Wayland session, and scaffold the per-user
# config dir from the shipped *.example files.
#
# This is the generic, distribution-agnostic install path — the same steps the
# homelab Ansible role used to hand-roll, now owned by the repo so anyone can
# install game-shell without that private tooling. It does NOT install system
# dependencies (Hyprland, Quickshell, Rust, build libs) — run scripts/install-deps.sh
# first, or install them via your package manager. See docs/INSTALL.md.
#
# Usage:
#   sudo ./scripts/install.sh [--prefix DIR] [--user NAME] [options]
#
#   --prefix DIR        Install root (default: /opt/game-shell). The shell itself
#                       is prefix-agnostic and resolves this at runtime; /opt is
#                       only this installer's default.
#   --user NAME         User whose ~/.config gets scaffolded and who owns the
#                       prefix (default: $SUDO_USER, else the invoking user).
#   --session-dir DIR   Where to write the .desktop (default: /usr/share/wayland-sessions).
#   --no-build          Skip building the daemon (reuse an existing binary).
#   --features LIST     Cargo features for the daemon (default: cec,mcp).
#   -h, --help          Show this help.
#
# Re-runnable: rebuilds the daemon, refreshes the install tree and session file,
# and never clobbers existing per-user config (only fills in missing files).
set -euo pipefail

PREFIX="/opt/game-shell"
SESSION_DIR="/usr/share/wayland-sessions"
TARGET_USER="${SUDO_USER:-$(id -un)}"
DO_BUILD=1
FEATURES="cec,mcp"

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

die() { echo "install: $*" >&2; exit 1; }
log() { echo "install: $*"; }

while [ $# -gt 0 ]; do
    case "$1" in
        --prefix)      PREFIX="${2:?--prefix needs a value}"; shift 2 ;;
        --user)        TARGET_USER="${2:?--user needs a value}"; shift 2 ;;
        --session-dir) SESSION_DIR="${2:?--session-dir needs a value}"; shift 2 ;;
        --no-build)    DO_BUILD=0; shift ;;
        --features)    FEATURES="${2:?--features needs a value}"; shift 2 ;;
        -h|--help)     sed -n '2,30p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'; exit 0 ;;
        *)             die "unknown argument: $1 (try --help)" ;;
    esac
done

# Resolve the target user's home (works whether or not we're under sudo).
TARGET_HOME="$(getent passwd "$TARGET_USER" | cut -d: -f6)"
[ -n "$TARGET_HOME" ] || die "could not resolve home for user '$TARGET_USER'"
CONFIG_DIR="$TARGET_HOME/.config/game-shell"
QS_LINK="$TARGET_HOME/.config/quickshell/game-shell"

# Writing under /opt + /usr/share needs root; the per-user bits get chowned back.
if [ "$(id -u)" -ne 0 ]; then
    die "must run as root (writes to $PREFIX and $SESSION_DIR) — re-run with sudo"
fi

log "prefix=$PREFIX user=$TARGET_USER home=$TARGET_HOME features=$FEATURES"

# 1. Build the daemon (canonical script owns the feature flags / profile).
if [ "$DO_BUILD" -eq 1 ]; then
    log "building game-shell-input ..."
    GAME_SHELL_FEATURES="$FEATURES" "$REPO_ROOT/scripts/build-daemon.sh"
fi
DAEMON_BIN="$REPO_ROOT/target/release/game-shell-input"
[ -f "$DAEMON_BIN" ] || die "daemon binary not found at $DAEMON_BIN (drop --no-build, or build it first)"

# 2. Lay down the install tree under $PREFIX. When the repo already lives at the
#    prefix (the in-place / dev-bridge layout), skip copying source over itself.
install -d -m755 "$PREFIX/bin"
if [ "$REPO_ROOT" != "$PREFIX" ]; then
    log "copying install tree to $PREFIX ..."
    for d in shell config scripts; do
        install -d -m755 "$PREFIX/$d"
        cp -a "$REPO_ROOT/$d/." "$PREFIX/$d/"
    done
else
    log "repo is the prefix — installing in place, no copy"
fi
install -m755 "$DAEMON_BIN" "$PREFIX/bin/game-shell-input"

# 3. Register the Wayland session, rewriting Exec to the resolved prefix.
install -d -m755 "$SESSION_DIR"
SESSION_FILE="$SESSION_DIR/game-shell-wayland.desktop"
log "writing session file $SESSION_FILE"
cat > "$SESSION_FILE" <<EOF
[Desktop Entry]
Type=Application
Name=Game Shell (Wayland)
Comment=Quickshell game streaming launcher on Hyprland
Exec=$PREFIX/scripts/game-shell-session.sh
DesktopNames=Hyprland
EOF

# 4. Per-user setup: Quickshell config symlink + config dir seeded from examples.
log "linking Quickshell config -> $PREFIX/shell"
install -d -m755 "$(dirname "$QS_LINK")"
ln -sfn "$PREFIX/shell" "$QS_LINK"

install -d -m755 "$CONFIG_DIR"
# Seed real config from *.example without ever clobbering a user's edits.
seed() { # seed <example-name> <dest-name>
    local src="$REPO_ROOT/config/$1" dst="$CONFIG_DIR/$2"
    if [ -f "$src" ] && [ ! -e "$dst" ]; then
        cp "$src" "$dst"; log "seeded $dst (from $1)"
    fi
}
seed daemon.env.example   daemon.env
seed targets.json.example targets.json
chmod 600 "$CONFIG_DIR/daemon.env" 2>/dev/null || true

# 5. Hand the prefix + per-user files back to the target user.
chown -R "$TARGET_USER" "$PREFIX" 2>/dev/null || true
chown -R "$TARGET_USER" "$CONFIG_DIR" "$(dirname "$QS_LINK")" 2>/dev/null || true

log "done. Select 'Game Shell (Wayland)' in your display manager, then log in."
log "Edit $CONFIG_DIR/daemon.env and targets.json to taste (see docs/INSTALL.md)."
