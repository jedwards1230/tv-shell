# Installing game-shell

A standalone install on any Linux box with Hyprland + Quickshell. The repo ships
the installer and config examples, so you don't need any external deployment
tooling. For the on-device dev/iteration loop instead, see the Development
section in [CLAUDE.md](../CLAUDE.md).

## What gets installed

- The Rust input daemon (`game-shell-input`) → `<prefix>/bin/`
- The QML shell + Hyprland config + scripts → `<prefix>/` (default prefix `/opt/game-shell`)
- A Wayland session entry → `/usr/share/wayland-sessions/game-shell-wayland.desktop`
- A `systemd --user` unit for the daemon → `~/.config/systemd/user/game-shell-input.service` (see [SYSTEMD_SETUP.md](SYSTEMD_SETUP.md))
- A Quickshell config symlink → `~/.config/quickshell/game-shell`
- A per-user config dir seeded from examples → `~/.config/game-shell/`

The shell is **prefix-agnostic** and resolves its install root at runtime, so any
`--prefix` works; `/opt/game-shell` is just the installer's default.

## 1. Dependencies

```bash
sudo ./scripts/install-deps.sh
```

Installs what your package manager can provide (Hyprland, Qt6/Wayland, Rust,
`libudev`+`pkg-config`, `grim`, `socat`) and prints what you must add by hand —
typically **Quickshell** (AUR / source) and **Moonlight Qt** (the streaming
client). The `cec` daemon feature static-links its own libcec, so no system
`libcec`/`libcec-dev` is needed.

## 2. Build + install

```bash
sudo ./scripts/install.sh                 # default prefix /opt/game-shell
sudo ./scripts/install.sh --prefix ~/.local/share/game-shell --user "$USER"
```

Re-runnable: it rebuilds the daemon, refreshes the tree and session file, and
never overwrites your existing `~/.config/game-shell` files. Useful flags:
`--no-build` (reuse an existing binary), `--features` (daemon Cargo features,
default `cec,mcp`), `--session-dir`, `--user`. See `./scripts/install.sh --help`.

## 3. Configure

Everything machine-specific lives under `~/.config/game-shell/` and is seeded
from the `config/*.example` files on first install.

### Streaming targets — `targets.json`

Single-line JSON array of Moonlight/Sunshine hosts. Fields:
`name`, `host`, `app`, `resolution`, `fps` (number), `codec`, `hdr` (bool), and
optional `sunshineUser`/`sunshinePass`/`sunshinePort` for pre-flight session
detection. Field semantics are documented in `config/targets.yaml.example`; the
copy-runnable starting point is `config/targets.json.example`. You can also
manage targets in-UI (Settings ▸ Widgets ▸ Moonlight). **Never pretty-print this
file** — the shell parses it line-by-line.

### Daemon env — `daemon.env` (optional)

Per-machine daemon options, sourced by the session wrapper before the daemon
starts: the LAN HTTP bridge / MCP server binds and token, CEC lifecycle, and the
optional Plex / Steam home-screen widgets. Every key is documented inline in
`config/daemon.env.example`. Keep it `chmod 600` (the installer does this) — it
can hold tokens. Leave it untouched and the shell still boots fully.

### Display tuning — `hyprland-local.conf` (optional)

`config/hyprland.conf` sources `~/.config/game-shell/hyprland-local.conf` if
present (ignored if absent). Drop your `monitor=` line and any HDR/VRR quirks
there — `config/hyprland.conf.example` is a worked 4K@120 10-bit HDR example.

## 4. Log in

Pick **Game Shell (Wayland)** in your display manager and log in. For a kiosk box
you'll usually also want autologin into this session — that's display-manager
specific (SDDM `autologin`, plasmalogin, GDM) and intentionally left to you.

## Optional: LAN control surface

Setting `GAME_SHELL_HTTP_BIND` / `GAME_SHELL_MCP_BIND` in `daemon.env` exposes the
daemon's control surface (screenshots, intents, MCP tools) over the network. See
[CONTROL_SURFACE.md](CONTROL_SURFACE.md). Firewall those ports yourself.
