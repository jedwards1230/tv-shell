#!/bin/bash
set -e

# Ensure XDG runtime dir exists with correct perms (Wayland requires 0700)
mkdir -p "$XDG_RUNTIME_DIR"
chmod 0700 "$XDG_RUNTIME_DIR"

# Symlink game-shell as quickshell config
ln -sfn /opt/game-shell "$HOME/.config/quickshell/game-shell"

# Copy default targets.json if the mounted volume doesn't have one
if [ ! -f /opt/game-shell/targets.json ]; then
    cp /opt/game-shell-defaults/targets.json /opt/game-shell/targets.json
fi

# Start dbus session (required by Qt/Wayland)
eval "$(dbus-launch --sh-syntax)"

# Start Hyprland in background
Hyprland -c /etc/hyprland-dev.conf &
HYPRLAND_PID=$!

# Wait for Hyprland socket
for i in $(seq 1 30); do
    if find "$XDG_RUNTIME_DIR/hypr/" -name ".socket.sock" -print -quit 2>/dev/null | grep -q .; then
        break
    fi
    sleep 0.5
done

# Export Hyprland instance signature for hyprctl
export HYPRLAND_INSTANCE_SIGNATURE=$(ls "$XDG_RUNTIME_DIR/hypr/" 2>/dev/null | tail -1)

# Start wayvnc
wayvnc --output=WL-1 0.0.0.0 5900 &
WAYVNC_PID=$!

# Start noVNC websockify bridge
websockify --web=/opt/novnc 0.0.0.0:6080 localhost:5900 &
NOVNC_PID=$!

echo "Game Shell dev environment ready"
echo "  VNC:   vnc://localhost:5900"
echo "  noVNC: http://localhost:6080/vnc.html"
echo ""
echo "Use 'restart.sh' to reload after QML changes"

# Wait for Hyprland (main process)
wait $HYPRLAND_PID
