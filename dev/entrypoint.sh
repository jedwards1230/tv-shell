#!/bin/bash
set -e

# Dev container runs everything as root inside the privileged container —
# simplest way to avoid GID mismatch issues with /dev/dri across hosts.

export XDG_RUNTIME_DIR=/run/user/0
export WAYLAND_DISPLAY=wayland-1
export QT_QPA_PLATFORM=wayland
export HOME=/root
export PATH=/usr/local/bin/stubs:/usr/local/bin:/usr/bin:/bin

mkdir -p "$XDG_RUNTIME_DIR"
chmod 0700 "$XDG_RUNTIME_DIR"

# Cache + config dirs for Hyprland
mkdir -p "$HOME/.config/quickshell" "$HOME/.cache/hyprland" "$HOME/.local/share/hyprland"

# Symlink game-shell as quickshell config
ln -sfn /opt/game-shell "$HOME/.config/quickshell/game-shell"

# Copy default targets.json if the mounted volume doesn't have one
if [ ! -f /opt/game-shell/targets.json ]; then
    cp /opt/game-shell-defaults/targets.json /opt/game-shell/targets.json
fi

# seatd as a background daemon
seatd -g root &
SEATD_PID=$!
for i in $(seq 1 20); do
    [ -S /run/seatd.sock ] && break
    sleep 0.2
done

# dbus session
eval "$(dbus-launch --sh-syntax)"

# Start Hyprland
Hyprland -c /etc/hyprland-dev.conf &
HYPRLAND_PID=$!

# Wait for Hyprland socket
for i in $(seq 1 30); do
    if find "$XDG_RUNTIME_DIR/hypr/" -name ".socket.sock" -print -quit 2>/dev/null | grep -q .; then
        break
    fi
    sleep 0.5
done

export HYPRLAND_INSTANCE_SIGNATURE=$(ls "$XDG_RUNTIME_DIR/hypr/" 2>/dev/null | tail -1)

# VNC + noVNC
wayvnc 0.0.0.0 5900 &
websockify --web=/opt/novnc 0.0.0.0:6080 localhost:5900 &

echo "Game Shell dev environment ready"
echo "  VNC:   vnc://localhost:5900"
echo "  noVNC: http://localhost:6080/vnc.html"

wait $HYPRLAND_PID
