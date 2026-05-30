# Game Shell

Quickshell + Hyprland couch gaming shell for Moonlight streaming.

A minimal 10-foot UI for a dedicated streaming box — controller-navigable home screen, exclusive gamepad input, auto-reconnect, and AV control.

## Architecture

```
SDDM → game-shell-session.sh → Hyprland (kiosk) → Quickshell (shell.qml)
                               └── game-shell-input (Rust daemon: EVIOCGRAB → uinput
                                   + backend IPC; falls back to gamepad-input.py)
```

- **Quickshell** renders the UI via QML on Hyprland's Wayland compositor
- **game-shell-input** (Rust daemon, `rust/`) grabs the gamepad exclusively, emits keyboard/mouse via uinput, and serves the backend IPC (settings, app discovery, Bluetooth/network/power, Hyprland, Sunshine). `input/gamepad-input.py` is kept as an input-only rollback
- **Moonlight** streams games from a Sunshine host
- **living-room-cec** and **end-game-session** scripts handle AV control (deployed separately via Ansible)

## Structure

```
shell.qml                # Entry point — state machine + process management
components/              # QML UI components
  HomeScreen.qml         # Target grid with controller navigation
  StreamCard.qml         # Individual streaming target card
  StatusBar.qml          # Clock, IP, state indicator
  StreamOverlay.qml      # Streaming/reconnecting overlay
  SettingsPanel.qml      # Audio + power controls
  SettingsButton.qml     # Focusable button widget
  Theme.qml              # Colors, fonts, layout constants
rust/                    # Rust backend daemon (game-shell-input) — primary
  src/                   # input/uinput, IPC, config, apps, bluetooth, network, power, hyprland, health
input/
  gamepad-input.py       # Input-only Python daemon — rollback fallback
config/
  hyprland.conf          # Kiosk compositor config
  game-shell.desktop     # SDDM wayland session entry
  targets.yaml.example   # Example streaming targets
scripts/
  game-shell-session.sh  # Session wrapper (starts input daemon + Hyprland)
```

## Configuration

Streaming targets are defined in `targets.yaml`:

```yaml
targets:
  - name: Desktop
    host: 192.168.8.10
    app: Desktop
    resolution: 3840x2160
    fps: 120
    codec: HEVC
    hdr: true
```

## Controls

| Button | Action |
|--------|--------|
| D-pad | Navigate |
| A | Select / Launch |
| B | Back / Settings |
| Y | Tab (future) |
| Home + B (3s) | End session (AV shutdown) |

## Requirements

- Hyprland
- Quickshell
- python3-evdev
- Moonlight Qt (built from source)

## License

Game Shell is free software licensed under the **GNU General Public License v3.0
(GPL-3.0)**. See [LICENSE](LICENSE) for the full terms.
