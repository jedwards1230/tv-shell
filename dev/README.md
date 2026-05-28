# Local Dev Environment

Run the game-shell UI locally in a headless Hyprland container — no TV required.

## Quick Start

```bash
docker compose -f dev/docker-compose.yml up -d --build
open http://localhost:6080/vnc.html
```

First build takes ~10 minutes (Quickshell compiles from source). Subsequent starts are instant.

## Usage

### View the shell

Open http://localhost:6080/vnc.html in any browser. Use keyboard arrow keys + Enter/Escape to navigate.

### Iterate on QML

QML files are volume-mounted. Edit locally, then restart Quickshell:

```bash
docker compose -f dev/docker-compose.yml exec game-shell-dev restart.sh
```

### Take a screenshot

```bash
docker compose -f dev/docker-compose.yml exec game-shell-dev grim /tmp/screenshot.png
docker compose -f dev/docker-compose.yml cp game-shell-dev:/tmp/screenshot.png ./screenshot.png
```

### Stop

```bash
docker compose -f dev/docker-compose.yml down
```

## Devcontainer

Open this repo in VS Code and select "Reopen in Container". The shell auto-starts and is viewable at http://localhost:6080/vnc.html.

## Architecture

- **Arch Linux** base image (x86_64)
- **Hyprland** in headless mode (`WLR_BACKENDS=headless`)
- **Quickshell** renders the shell at 1920x1080
- **wayvnc** exposes the display as VNC on port 5900
- **noVNC + websockify** provides a web viewer on port 6080
- **Stub scripts** replace hardware tools (moonlight, wpctl, etc.) with mock data

## Platform Notes

The Arch image is x86_64 only. On Apple Silicon Macs, Docker Desktop runs it via Rosetta 2 emulation — performance is adequate for QML rendering.

## Stubs

Hardware-dependent commands are stubbed in `dev/stubs/`:

| Tool | Behavior |
|------|----------|
| wpctl | Returns volume 0.75, accepts set commands |
| bluetoothctl | Reports powered, lists fake paired devices |
| nmcli | Reports wired connection, fake WiFi list |
| moonlight | Lists fake apps, pair succeeds |
| cec-client | No-op |
| systemctl | Blocks power ops, passes other commands |

To customize stub behavior, edit the scripts in `dev/stubs/`.
