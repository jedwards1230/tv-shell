# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

A 10-foot couch gaming UI built with Quickshell (QML) on Hyprland for a dedicated streaming box (game-client-1). Controller-navigable home screen for Moonlight game streaming, local app launching, and system settings. Designed for 4K@120Hz HDR on an LG C2 via a Denon AVR.

## Architecture

```
SDDM → game-shell-session.sh → Hyprland (kiosk) → Quickshell (shell.qml)
                               └── gamepad-input.py (EVIOCGRAB → uinput)
```

- **shell.qml** is the entry point: state machine (`idle` → `launching` → `streaming` → `reconnecting`) and process management (Moonlight, AV wake, input grab/release)
- **gamepad-input.py** is an async daemon that grabs the gamepad exclusively via evdev, emits keyboard events via uinput, and listens on a Unix socket for `grab`/`release`/`subscribe` commands
- **Theme.qml** is a singleton with all colors, fonts, and layout constants — dark/light/auto modes persisted to `~/.config/game-shell/settings.json`
- **Components** are registered in `components/qmldir` — new components must be added there

## Key Data Flows

- **Streaming targets**: Loaded from `/opt/game-shell/targets.json` at startup. Managed in-UI via MoonlightSettings (add/remove servers). The `targets.yaml.example` is for documentation only — the runtime reads JSON.
- **Input daemon IPC**: QML sends `grab`/`release` commands to the Python daemon via a Unix socket (`/run/user/$UID/game-shell-input.sock`). During streaming, the gamepad is released so Moonlight gets raw input; on return to shell, it's re-grabbed for keyboard emulation.
- **Settings panels**: SettingsPanel uses a Loader to swap between 7 section components (Audio, Bluetooth, Network, Display, Moonlight, Appearance, Power). Each section manages its own system calls via `Quickshell.Io.Process`.

## System Integration

| Tool | Used For |
|------|----------|
| `wpctl` | Audio volume/mute/sink switching (WirePlumber/PipeWire) |
| `bluetoothctl` | Bluetooth device scanning and pairing |
| `nmcli` | Network connection management |
| `hyprctl` | Monitor mode/scale changes, app launching |
| `systemctl` | Power management (suspend/reboot/poweroff) |
| `moonlight` | Game streaming client |
| `living-room-cec` | HDMI-CEC AV receiver control (deployed separately via Ansible) |
| `end-game-session` | AV shutdown on Home+B combo (deployed separately via Ansible) |

## Development

No build step — QML is interpreted by Quickshell at runtime. Deploy by syncing files to `/opt/game-shell/` on game-client-1.

The `game-shell-dev` skill handles the deploy/restart/screenshot cycle for iterating on this repo. Use it for visual verification.

### Python Input Daemon

```bash
pip install evdev  # or: pip install -r input/requirements.txt
```

Requires Linux with evdev and uinput access. The daemon auto-discovers the gamepad by vendor/product ID (defaults: Xbox controller `045e:028e`, configurable via `GAMEPAD_VENDOR`/`GAMEPAD_PRODUCT` env vars).

## Design Constraints

- **10-foot UI at 4K**: All font sizes and layout constants in Theme.qml are sized for couch-distance reading. Don't shrink them.
- **Controller-first navigation**: Every interactive element must be reachable via D-pad (arrow keys) and activatable with A (Enter). B (Escape) always goes back. Focus management is critical.
- **Palette rules**: See `config/palette.md`. Never use gold for text. Crimson for focus/active states. Ember for secondary interactive elements. All overlay backdrops use `Qt.rgba(0, 0, 0, 0.7-0.85)`.
- **No build tooling**: No bundler, no compiler, no package manager for QML. Files are deployed as-is.
