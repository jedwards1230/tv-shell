# Installing game-shell

A standalone install on any Linux box with Hyprland + Quickshell. The repo ships
the installer and config examples, so you don't need any external deployment
tooling. For the on-device dev/iteration loop instead, see the Development
section in [CLAUDE.md](../CLAUDE.md).

## What gets installed

- The Rust input daemon (`game-shell-input`) → `<prefix>/bin/`
- The QML shell + Hyprland config + scripts → `<prefix>/` (default prefix `/opt/game-shell`)
- A Wayland session entry → `/usr/share/wayland-sessions/game-shell-wayland.desktop`
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

### Daemon config — `config.toml` (optional)

Per-machine daemon options, read directly by the daemon at startup: the LAN HTTP
bridge / MCP server binds, the auth toggle, CEC lifecycle, and the optional Plex /
Steam home-screen widgets. Every key is documented inline in
`config/config.toml.example`. The shared bearer token is **never inline** — it
lives in a separate `0600` file that `[http] token_file` points at (e.g.
`openssl rand -hex 32 > ~/.config/game-shell/http-token`). Leave the file
untouched and the shell still boots fully.

> **Startup safety check:** the daemon **refuses to start** if a control surface
> is bound to a non-loopback address with dev tools on and auth effectively off
> (an unauthenticated RCE surface). Set `[dev] allow_insecure_lan = true` to
> override that on a box that genuinely wants the unauthenticated LAN dev loop.

#### Migrating an existing deploy (daemon.env → config.toml)

This release replaced the old `daemon.env` env file with `config.toml`. The
session script no longer sources `daemon.env`, so a box that relied on it for the
LAN bridge / MCP server **will silently lose those settings** (the surface just
won't come up) — and a box that ran a non-loopback bind with dev tools + no auth
will now hit the startup-refusal above. On such a box the deploy is no longer just
"restart the shell"; do it in this order so the daemon doesn't refuse to start:

1. Run the installer/migration so the new tree + `config.toml.example` are in place.
2. **Write `~/.config/game-shell/config.toml`** translating the box's old
   `daemon.env` (and, if it ran LAN + dev + no-auth, include `[dev]
   allow_insecure_lan = true` — without it the daemon refuses to start).
3. **Then** restart the daemon/shell.

For the canonical agent-native dev box (LAN bind + dev tools + auth off on a
trusted, firewalled single-user LAN), the equivalent `config.toml` is:

```toml
[http]
bind = "0.0.0.0:8089"
auth_enabled = false

[mcp]
bind = "0.0.0.0:8090"
dev = true

[dev]
# Required: without this the daemon refuses to start (LAN bind + dev + no auth).
allow_insecure_lan = true
```

To **lock such a box down** instead of preserving the insecure loop, drop the
`[dev]` block, set `[http] auth_enabled = true`, and point `[http] token_file` at
a `0600` token file (`openssl rand -hex 32 > ~/.config/game-shell/http-token`).

### Display tuning — `hyprland-local.conf` (optional)

`config/hyprland.conf` sources `~/.config/game-shell/hyprland-local.conf` if
present (ignored if absent). Drop your `monitor=` line and any HDR/VRR quirks
there — `config/hyprland.conf.example` is a worked 4K@120 10-bit HDR example.

## 4. Log in

Pick **Game Shell (Wayland)** in your display manager and log in. For a kiosk box
you'll usually also want autologin into this session — that's display-manager
specific (SDDM `autologin`, plasmalogin, GDM) and intentionally left to you.

## Optional: LAN control surface

Setting `[http] bind` / `[mcp] bind` in `config.toml` exposes the daemon's control
surface (screenshots, intents, MCP tools) over the network. See
[CONTROL_SURFACE.md](CONTROL_SURFACE.md). Firewall those ports yourself.
