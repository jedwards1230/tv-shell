#!/bin/bash
# Restart Quickshell inside the dev container and check for errors.

SIG=$(ls "$XDG_RUNTIME_DIR/hypr/" 2>/dev/null | tail -1)
export HYPRLAND_INSTANCE_SIGNATURE="$SIG"

# Kill existing Quickshell
killall quickshell 2>/dev/null
sleep 1

# Restart via Hyprland
hyprctl dispatch exec 'quickshell -c game-shell > /tmp/qs-log.txt 2>&1'
sleep 2

# Check for errors (filter icon warnings which are expected)
ERRORS=$(grep -E 'WARN|ERROR' /tmp/qs-log.txt 2>/dev/null | grep -vc 'Could not load icon' || true)

if [ "$ERRORS" -gt 0 ]; then
    echo "⚠ $ERRORS warning(s) found:"
    grep -E 'WARN|ERROR' /tmp/qs-log.txt | grep -v 'Could not load icon'
    exit 1
else
    echo "✓ Quickshell restarted cleanly"
fi
