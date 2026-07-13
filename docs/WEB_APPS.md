# Web Apps (PWAs)

Let the user add **web apps** (YouTube, Plex, Spotify, …) from **Settings ▸ Web
Apps**, persist them, and have them appear on the home rail and launch like any
other local app — the same UX Plasma Bigscreen ships.

## Mechanism

A web app is just an XDG **`.desktop` file** in `~/.local/share/applications/`.
The daemon already scans that directory (`list-apps`, via the
`freedesktop-desktop-entry` crate), so a web app flows into the home
**Applications** row and `intent app:<wmClass>` **for free** — no new launch
plumbing. Each generated entry carries:

- `Exec=` a **Chromium `--app=<URL>`** launcher, with `--class=<stable-id>`,
  `--ozone-platform=wayland`, and a dedicated per-app `--user-data-dir`.
- `StartupWMClass=<stable-id>` matching `--class`, so the daemon's existing
  window matching (`app:<wmClass>` intent + window lifecycle) works unchanged.
- `X-GameShell-WebApp=true` — a marker key so we can list/edit/remove only our
  own entries and never touch foreign `.desktop` files.

### Why Chromium `--app`, not a QtWebEngine viewer

- Quickshell ships no WebEngine.
- Netflix / Plex need **Widevine**; Chromium provides it (plus hardware decode).
- Reusing the OS browser keeps logins and DRM working with zero extra code.

## Per-service guidance

| Service | Recommended surface | Why |
|---------|---------------------|-----|
| YouTube | **Web app** (Chromium `--app`) | No good native HTPC client; TV web UI is fine. |
| Netflix | **Web app** (Widevine) | Web is the only Linux path; needs Widevine. |
| Plex    | **Native (Flatpak)** | Dedicated 10-foot HTPC client; better than web. |
| Spotify | **Native (Flatpak)** | Mature native client; Connect + gapless. |

Web apps complement native apps — they don't replace the better native client
where one exists.

## Registry schema

The registry is a JSON array of entries. Each entry:

```json
{ "id": "youtube", "name": "YouTube", "url": "https://youtube.com/tv", "wmClass": "gameshell-youtube" }
```

- `id` — stable slug (also the registry key / user-data-dir suffix).
- `name` — display label on the rail and the Settings list.
- `url` — launched via Chromium `--app=<url>`.
- `wmClass` — the `--class` / `StartupWMClass` value; the window-matching key.

**Ownership: the daemon is the sole writer** (mirroring `settings.json`). QML
reads the registry via `SettingsStore.webApps` (a read-only mirror, `noSave` in
the settings schema) and never formats or writes it — the daemon owns
`.desktop` generation and the registry key (see Phases).

## Phases

- **P0 — Foundation (shipped):** this doc + `Web Apps` settings page (read-only
  stub listing registry entries) + `webApps` registry schema in `SettingsStore`
  (daemon-owned mirror) + qmldir / SettingsApp registration. No daemon/IPC yet.
- **P1 — Registry IPC:** daemon `webapp-list` / `webapp-add` / `webapp-remove`
  commands that own the `.desktop` generation + registry file.
- **P2 — Add flow UI:** name + URL entry (needs the on-screen keyboard, #20),
  curated presets (YouTube / Plex / Netflix / Spotify) for one-tap add, icon
  fetch/fallback.
- **P3 — Launcher generation:** Chromium `--app` launcher + `.desktop` writer
  with stable `--class` / `StartupWMClass` and per-app `--user-data-dir`.
- **P4 — Polish:** edit / remove, icon caching, preset refinement, optional
  per-app user-agent.

## Notes

- No credentials are committed — logins live in each web app's own
  `--user-data-dir` on the device.
- `intent settings:webapps` deep-links to the Settings page (generic
  `openSectionById` routing; no special-casing).

Tracking: [#187](https://github.com/jedwards1230/tv-shell/issues/187).
