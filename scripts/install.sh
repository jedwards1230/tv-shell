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
#   --session-exec CMD  Exec= line for the session .desktop (default:
#                       <prefix>/scripts/game-shell-session.sh). Override when a
#                       deployment wraps the session (e.g. a site launcher that
#                       runs site-specific setup before the repo launcher).
#   --no-build          Skip building the daemon (reuse an existing binary if any).
#   --features LIST     Cargo features for the daemon (default: cec,mcp).
#   -h, --help          Show this help.
#
# Re-runnable: rebuilds the daemon, refreshes the install tree and session file,
# and never clobbers existing per-user config (only fills in missing files).
set -euo pipefail

PREFIX="/opt/game-shell"
SESSION_DIR="/usr/share/wayland-sessions"
SESSION_EXEC=""
TARGET_USER="${SUDO_USER:-}"
DO_BUILD=1
FEATURES="cec,mcp"

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

die() { echo "install: $*" >&2; exit 1; }
log() { echo "install: $*"; }

while [ $# -gt 0 ]; do
    case "$1" in
        --prefix)       PREFIX="${2:?--prefix needs a value}"; shift 2 ;;
        --user)         TARGET_USER="${2:?--user needs a value}"; shift 2 ;;
        --session-dir)  SESSION_DIR="${2:?--session-dir needs a value}"; shift 2 ;;
        --session-exec) SESSION_EXEC="${2:?--session-exec needs a value}"; shift 2 ;;
        --no-build)     DO_BUILD=0; shift ;;
        --features)     FEATURES="${2:?--features needs a value}"; shift 2 ;;
        -h|--help)      sed -n '2,33p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'; exit 0 ;;
        *)              die "unknown argument: $1 (try --help)" ;;
    esac
done

# Default the session Exec to the repo launcher under the resolved prefix.
SESSION_EXEC="${SESSION_EXEC:-$PREFIX/scripts/game-shell-session.sh}"

# Resolve the target user: --user wins, else $SUDO_USER, else the invoking user.
# Guard the footgun where someone runs as plain root (e.g. `sudo -i`) with no
# --user — that would install everything root-owned and break the shell for
# normal users. We check the *resolved* user (not $SUDO_USER directly) so an
# explicit --user still works under `become`/sudo that doesn't export SUDO_USER.
TARGET_USER="${TARGET_USER:-$(id -un)}"
if [ "$(id -u)" -eq 0 ] && [ "$TARGET_USER" = "root" ]; then
    die "running as root with no target user — pass --user NAME (or run via sudo) so the install isn't root-owned"
fi

# Resolve the target user's home (works whether or not we're under sudo).
TARGET_HOME="$(getent passwd "$TARGET_USER" | cut -d: -f6)"
[ -n "$TARGET_HOME" ] || die "could not resolve home for user '$TARGET_USER' (does the user exist?)"
CONFIG_DIR="$TARGET_HOME/.config/game-shell"
QS_LINK="$TARGET_HOME/.config/quickshell/game-shell"

# Writing under /opt + /usr/share needs root; the per-user bits get chowned back.
if [ "$(id -u)" -ne 0 ]; then
    die "must run as root (writes to $PREFIX and $SESSION_DIR) — re-run with sudo"
fi

log "prefix=$PREFIX user=$TARGET_USER home=$TARGET_HOME features=$FEATURES"

# Validate the prefix is creatable AND writable up front — don't fail with a
# cryptic error only after a multi-minute daemon build (read-only FS, missing
# parent, no space, etc.).
mkdir -p "$PREFIX" || die "cannot create prefix directory: $PREFIX (check permissions / parent path)"
( touch "$PREFIX/.install-write-check" && rm -f "$PREFIX/.install-write-check" ) \
    || die "cannot write to $PREFIX (read-only mount, permissions, or no space?)"

# 1. Build the daemon (canonical script owns the feature flags / profile). The
#    workspace builds to the repo-root target/ (see scripts/build-daemon.sh).
DAEMON_BIN="$REPO_ROOT/target/release/game-shell-input"
if [ "$DO_BUILD" -eq 1 ]; then
    log "building game-shell-input (features=$FEATURES) ..."
    GAME_SHELL_FEATURES="$FEATURES" "$REPO_ROOT/scripts/build-daemon.sh" || die "daemon build failed"
    [ -f "$DAEMON_BIN" ] || die "build finished but $DAEMON_BIN is missing"
    log "daemon build succeeded"
fi

# 2. Lay down the install tree under $PREFIX. When the repo already lives at the
#    prefix (the in-place / dev-bridge layout), skip copying source over itself.
install -d -m755 "$PREFIX/bin"
if [ "$REPO_ROOT" != "$PREFIX" ]; then
    log "copying install tree to $PREFIX ..."
    for d in shell config scripts; do
        install -d -m755 "$PREFIX/$d" || die "cannot create $PREFIX/$d"
        cp -a "$REPO_ROOT/$d/." "$PREFIX/$d/" || die "failed to copy $d/ to $PREFIX/$d (permissions or space?)"
    done
else
    log "repo is the prefix — installing in place, no copy"
fi
# Install the freshly built binary. With --no-build and no build artifact (e.g. a
# python-fallback deploy, or target/ was cleaned after a prior install), leave any
# already-installed binary in place rather than failing.
if [ -f "$DAEMON_BIN" ]; then
    install -m755 "$DAEMON_BIN" "$PREFIX/bin/game-shell-input"
elif [ -x "$PREFIX/bin/game-shell-input" ]; then
    log "no build artifact at $DAEMON_BIN — keeping the installed daemon binary"
else
    log "WARNING: no daemon binary built or installed (continuing — shell will use any fallback)"
fi

# 3. Register the Wayland session, rewriting Exec to the resolved prefix.
install -d -m755 "$SESSION_DIR"
SESSION_FILE="$SESSION_DIR/game-shell-wayland.desktop"
log "writing session file $SESSION_FILE"
cat > "$SESSION_FILE" <<EOF
[Desktop Entry]
Type=Application
Name=Game Shell (Wayland)
Comment=Quickshell game streaming launcher on Hyprland
Exec=$SESSION_EXEC
DesktopNames=Hyprland
EOF

# 4. Per-user setup: Quickshell config symlink + config dir seeded from examples.
log "linking Quickshell config -> $PREFIX/shell"
install -d -m755 "$(dirname "$QS_LINK")"
ln -sfn "$PREFIX/shell" "$QS_LINK" || die "failed to create symlink $QS_LINK"
[ -d "$QS_LINK" ] || die "symlink $QS_LINK is broken — target $PREFIX/shell does not exist"

install -d -m755 "$CONFIG_DIR"
# Seed real config from *.example without ever clobbering a user's edits.
seed() { # seed <example-name> <dest-name>
    local src="$REPO_ROOT/config/$1" dst="$CONFIG_DIR/$2"
    if [ -f "$src" ] && [ ! -e "$dst" ]; then
        cp "$src" "$dst"; log "seeded $dst (from $1)"
    fi
}
seed config.toml.example  config.toml
seed targets.json.example targets.json
# config.toml holds no secret inline (the bearer token lives in a separate 0600
# file referenced by [http].token_file), so it needs no special mode. If an
# operator still has a token file beside it, leave their permissions alone.

# 5. Hand the prefix + per-user files back to the target user. Failure here is
#    fatal — silently leaving the tree root-owned would block the user (and the
#    dev bridge) from editing or rebuilding later.
chown -R "$TARGET_USER" "$PREFIX" \
    || die "failed to chown $PREFIX to $TARGET_USER (check permissions / filesystem type)"
chown -R "$TARGET_USER" "$CONFIG_DIR" "$(dirname "$QS_LINK")" \
    || die "failed to chown config dirs to $TARGET_USER (does $TARGET_USER own ~/.config?)"

log "done. Select 'Game Shell (Wayland)' in your display manager, then log in."
log "Edit $CONFIG_DIR/daemon.env and targets.json to taste (see docs/INSTALL.md)."
