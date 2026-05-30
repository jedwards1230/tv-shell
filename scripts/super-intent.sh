#!/bin/bash
# super-intent.sh — keyboard global-escape for the game-shell.
#
# Hyprland binds the bare Super press to this script (press-only; no hold —
# the reset multi-stroke is counted QML-side as three rapid Super presses).
# It injects `intent home` into the input daemon's control surface, which
# re-broadcasts `intent:home` to the QML shell (returnToShell).
#
# This is the ONLY keyboard escape, so it must be robust at early boot when
# the daemon socket may not exist yet: retry a few times with a short backoff
# before giving up. Uses socat or nc -U (whichever is installed).

SOCK="${GAME_SHELL_SOCK:-/run/user/$(id -u)/game-shell-input.sock}"

send_intent() {
    if command -v socat >/dev/null 2>&1; then
        printf 'intent home\n' | socat - "UNIX-CONNECT:$SOCK" >/dev/null 2>&1
    elif command -v nc >/dev/null 2>&1; then
        printf 'intent home\n' | nc -U -w1 "$SOCK" >/dev/null 2>&1
    else
        return 127
    fi
}

# Retry briefly so a Super press during the daemon's startup window still lands.
for _ in 1 2 3 4 5; do
    if [ -S "$SOCK" ] && send_intent; then
        exit 0
    fi
    sleep 0.2
done

exit 1
