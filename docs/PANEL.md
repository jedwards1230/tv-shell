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
| Processes | tv-shell systemd user units (daemon/shell/panel) with per-unit restart; Hyprland active window/clients/monitors via IPC; read-only top-processes snapshot (`ps`, CPU-sorted, no kill action in v1) |
| Settings | grouped typed forms over `settings.json` via `get-config`/`set-config` (shallow merge — unmentioned keys, notably the daemon-owned `keyBindings`/`perGameBindings`/`perPlayerBindings`, are left untouched); those daemon-owned binding keys are shown read-only pending a Controllers page; read-only `config.toml` view (the edit path is deferred — editing still requires a manual edit + daemon/panel restart via the Dev page); raw JSON escape hatch with an explicit shallow-merge/`null`-deletes warning for keys not modeled as typed fields (e.g. `widgets`, `cecDeviceNames`) |
| Widgets | per-widget enabled/order/size/prefs editors (`widgets.<id>` subtree) |
| Tools | IPC console grouped by domain — Navigation (intent/key), Apps (list/launch/recents), Bluetooth (power/scan/list/connect-disconnect-pair-trust), Network (status/wifi/throughput/ping), Power (can-suspend/battery), System (sys-status/sys-metrics/storage-status/build-info/controllerdb); plus a raw-line escape hatch with a warning on commands owned by another page's guarded flow. CEC and controller/pads/bindings commands are deferred to the M4 Controllers/CEC pages. |
| Controllers | pads, battery, rumble test, bindings editor, capture, controller DB, grab/release/handoff |
| CEC | device scan, active-source switching, power on/off, health, wedge diagnosis + recovery |
| Dev | deploy/build/restart daemon/restart shell/reboot with tier labels + confirms; screenshot viewer (provenance sha/branch/version/captured-at, proxied via `/dev/screenshot`) |
| Logs | shell + daemon log tails with filters |

## Running locally

Build with `scripts/build-panel.sh` (outputs to `target/release/tv-shell-panel`)
or `cargo run -p tv-shell-panel` for a dev loop. It reads `[panel]` from
`~/.config/tv-shell/config.toml` and serves on `127.0.0.1:8091` by default (see
`config/config.toml.example`). Installed systems run it as
`tv-shell-panel.service`, started by `scripts/tv-shell-session.sh`.

## Milestones

- [x] M1 — crate scaffold, IPC client, app shell/nav, Dashboard, Logs, Dev page
- [x] M2 — Settings + Widgets editors
  - [x] Settings editor
  - [x] Widgets editor
- [x] M3 — Tools console, Processes, screenshot viewer
  - [x] Tools console (Navigation/Apps/Bluetooth/Network/Power/System + raw escape hatch)
  - [x] Processes page (systemd units, Hyprland, top processes)
  - [x] Dev screenshot viewer (provenance headers, PNG proxy)
- [ ] M4 — Controllers + CEC (switching, grab handling, wedge recovery)
