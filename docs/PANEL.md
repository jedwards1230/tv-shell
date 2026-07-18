# tv-shell-panel — Web Control Panel

`tv-shell-panel` is a LAN-only web control panel for tv-shell: server-rendered
HTML + HTMX over the daemon's existing control surface. It runs as its own
`systemd --user` unit beside the daemon, so it stays available to rebuild or
restart a wedged daemon — the recovery path that previously required remote
config management.

> Status: under construction on the `panel-staging` branch. This document is the
> panel's living doc — each milestone extends it.

## Architecture

- **Crate**: `panel/` (workspace member) → binary `tv-shell-panel`.
- **Stack**: axum + askama templates + vendored `htmx.min.js` (no CDN; the panel
  must render when the network or the rest of the system is broken).
- **Bind**: `[panel]` section in `config.toml` (`enabled`, `bind`, default
  `127.0.0.1:8091`; `token_file` reserved for future auth — v1 is LAN-only).
- **Unit**: `config/tv-shell-panel.service`, installed by `scripts/install.sh`,
  started by the session script.

### Three data tiers

1. **Unix-socket IPC** (primary) — the daemon's newline-text protocol
   (`docs/IPC_PROTOCOL.md`): status, system info, storage, settings
   (`get-config`/`set-config` — the daemon remains sole writer of
   `settings.json`), widgets subtree, bluetooth, network, power, CEC,
   controllers/bindings, apps, intents/keys.
2. **Daemon HTTP bridge** (dev ops) — `/dev/deploy|build|restart-shell|
   restart-daemon|logs`, `/screenshot` (`docs/CONTROL_SURFACE.md`).
3. **Direct exec** (recovery + system) — `systemctl --user` restarts,
   `build-daemon.sh` when the daemon is down, `journalctl`, process list,
   reboot/suspend via logind. The UI labels which tier each action uses;
   destructive actions are confirmed and single-flight.

## Pages

| Page | Contents |
|---|---|
| Dashboard | unit status, build info, system/storage tiles, pad fleet, quick actions |
| Processes | systemd user units, Hyprland clients, top processes |
| Settings | typed forms over `settings.json` via `set-config`; raw JSON escape hatch; `config.toml` editor with restart prompt |
| Widgets | per-widget enabled/order/size/prefs editors (`widgets.<id>` subtree) |
| Tools | full IPC/intent console grouped by domain |
| Controllers | pads, battery, rumble test, bindings editor, capture, controller DB, grab/release/handoff |
| CEC | device scan, active-source switching, power on/off, health, wedge diagnosis + recovery |
| Dev | deploy/build/restart daemon/restart shell/reboot with tier labels + confirms; screenshot viewer with provenance |
| Logs | shell + daemon log tails with filters |

## Milestones

- [ ] M1 — crate scaffold, IPC client, app shell/nav, Dashboard, Logs, Dev page
- [ ] M2 — Settings + Widgets editors
- [ ] M3 — Tools console, Processes, screenshot viewer
- [ ] M4 — Controllers + CEC (switching, grab handling, wedge recovery)
