# Web Apps (PWAs from Settings)

Tracking issue: **#187**. Let the user add a web app (YouTube, Plex, Spotify, …)
from **Settings → Web Apps**, persist it, and have it appear on the home rail and
launch like any other local app — the same feature Plasma Bigscreen ships.

This doc captures the research, a per-service recommendation, the chosen
mechanism, and a phased plan. The first slice (P0) lands the doc, a stubbed
settings page, and the registry schema; later phases add the daemon-owned
launcher generation.

---

## 1. How Plasma Bigscreen does it

Inspected directly in the upstream tree (`repos/plasma-bigscreen/`):

- A **KCM** (`kcms/webapps/`) provides the "Web Apps" settings UI.
- `WebAppManager::addApp()` writes an XDG **`.desktop` file** to
  `~/.local/share/applications/` (`webappmanager.cpp`), with:
  - `Exec=plasma-bigscreen-webapp --name "<name>" [--agent "<UA>"] "<url>"`
  - `Icon=`, `Name=`, `URL=`
  - marker keys `X-KDE-Bigscreen-Webapp-Id` + `X-KDE-Bigscreen-Webapp-UserAgent`
    so the manager can list/edit/remove only its own entries.
- After writing it runs `kbuildsycoca6` to refresh the app database, and the
  entry then shows on the homescreen like any native app.
- `plasma-bigscreen-webapp` (`webapp-viewer/`) is a **QtWebEngine** app — a
  fullscreen `WebEngineView` (`webapp-viewer/qml/WebView.qml`, `import QtWebEngine`)
  with Widevine/fullscreen/screen-capture enabled.

**Takeaway:** the durable artifact is a `.desktop` launcher in the per-user
applications dir + a marker key. The *viewer* (QtWebEngine vs Chromium) is an
implementation detail behind that launcher.

Sources: upstream `kcms/webapps/webappmanager.cpp`, `kcms/webapps/webappcreator.cpp`,
`webapp-viewer/qml/WebView.qml`.

---

## 2. Why this maps cleanly onto game-shell

game-shell already has the whole back half of this feature:

- The **daemon scans `/usr/share/applications` then `~/.local/share/applications`**
  for `.desktop` entries and serves them as `list-apps`
  (`daemon/src/apps.rs::app_dirs()`, `docs/IPC_PROTOCOL.md`).
- `AppDiscoveryManager` feeds those into the home **Applications** row.
- `app:<wmClass>` intent + `WindowMatcher` + `AppLifecycleManager` launch and
  track any app by its `StartupWMClass`.

So a web app written as a `.desktop` file in `~/.local/share/applications/` with
a `StartupWMClass` that matches the launched window's class **flows through the
entire existing system with zero new launch/lifecycle plumbing.** That is the
core decision: web apps are just generated `.desktop` launchers.

---

## 3. Viewer choice: Chromium `--app` (not QtWebEngine)

Bigscreen ships a bespoke QtWebEngine viewer. game-shell should **not** — and
doesn't need to:

- Quickshell has no WebEngine module; bundling a QtWebEngine app is a new C++/Qt
  binary to build and package (against the repo's "no build tooling for the
  shell" grain).
- The target services need **Widevine DRM** (Netflix, Plex web, Spotify web).
  Chromium ships Widevine; a stock QtWebEngine build often does not.
- Chromium `--app=<URL>` gives a chromeless, single-site window with hardware
  video decode — the exact 10-foot kiosk surface we want.

### Launch shape

```
chromium --app=<URL> \
  --class=<stable-id> \
  --ozone-platform=wayland \
  --user-data-dir=$XDG_DATA_HOME/game-shell/webapps/<id> \
  [--start-fullscreen]
```

- `--app=<URL>` → chromeless application window.
- `--class=<stable-id>` → **forces a stable Wayland `app_id`/window class.**
  By default Chromium derives the class from the URL (e.g.
  `chrome-youtube.com__-Default`), which is brittle. Forcing `--class` lets the
  generated `.desktop` `StartupWMClass=<stable-id>` match exactly, so
  `WindowMatcher` and `app:<stable-id>` work.
- `--user-data-dir` per app → isolated cookies/login so each web app keeps its
  own session (e.g. separate Plex vs YouTube logins).
- `--ozone-platform=wayland` → native Wayland window under Hyprland.

> Verify on game-client-1 which Chromium/Chrome binary is present (Fedora 43:
> `chromium` or `google-chrome-stable` via flatpak/rpm) and whether `--class`
> or the newer `--app-id`/`--name` sets the Hyprland class. This is the one
> detail that can only be confirmed on-device. If `--class` is ignored, fall
> back to reading the derived class once via `hypr-clients` and writing it into
> the `.desktop` `StartupWMClass`.

---

## 4. Per-service recommendation

| Service  | Native Linux app (Fedora)                     | Recommendation | Why |
|----------|-----------------------------------------------|----------------|-----|
| **Plex** | **Yes** — `tv.plex.PlexHTPC` Flatpak (10-foot UI) | **Native** (Flatpak) | Purpose-built 10-foot HTPC client; better than a web window. Web app only as fallback. |
| **YouTube** | No native app | **Web app** (`--app=https://youtube.com`) | No Linux client exists; PWA/`--app` is the recommended desktop experience. |
| **Spotify** | **Yes** — `com.spotify.Client` Flatpak (community, official build) | **Native** (Flatpak) preferred; web app works | Native has MPRIS + offline; see also #22. Web app fine for a quick add. |
| **Netflix** | No native app | **Web app, with caveats** | Widevine **L3** under Linux/Chromium caps Netflix at ~720p (1080p only via protocol nudges; 4K needs L1 + HDCP 2.2, unavailable). Set expectations in the UI. |
| **Generic / dashboards** (Home Assistant #24, internal sites) | n/a | **Web app** | Arbitrary URL is exactly the web-app case. |

**Policy for the picker:** offer curated presets but, where a first-class native
Flatpak exists (Plex, Spotify), surface a hint nudging the native install
instead of a web window. The Web Apps page is the right home for **YouTube,
Netflix, dashboards, and arbitrary URLs**.

Sources:
- Plex HTPC Flatpak — <https://flathub.org/en/apps/tv.plex.PlexHTPC>
- Spotify Flatpak — <https://flathub.org/en/apps/com.spotify.Client>
- YouTube PWA (no native client) — <https://techtrickz.com/how-to/install-youtube-pwa-on-desktop/>
- Netflix Widevine L3 720p / HDCP — <https://dev.to/picklepixel/how-i-made-netflix-give-me-4k-because-apparently-my-browser-wasnt-good-enough-4fa2>
- Chromium `--app` web apps on Hyprland/Wayland — <https://coko7.fr/posts/custom-desktop-web-apps-with-hyprland/>

---

## 5. Registry schema

Source of truth for *which* web apps exist is the set of generated `.desktop`
files (marked `X-GameShell-WebApp=true`), exactly like Bigscreen. A small JSON
registry mirrors them for the UI so the page doesn't have to parse `.desktop`
files in QML.

`~/.config/game-shell/webapps.json` (single-line JSON — same SplitParser
constraint as `settings.json`/`targets.json`):

```json
[{"id":"youtube","name":"YouTube","url":"https://www.youtube.com","icon":"youtube","wmClass":"gameshell-webapp-youtube"}]
```

| Field     | Meaning |
|-----------|---------|
| `id`      | Slug, unique. Drives filenames + `--user-data-dir`. |
| `name`    | Display name (home card + `.desktop` `Name`). |
| `url`     | Target URL passed to `--app=`. |
| `icon`    | Freedesktop icon name or bundled SVG; falls back to a letter initial. |
| `wmClass` | Stable class = `--class` = `.desktop` `StartupWMClass`. Convention: `gameshell-webapp-<id>`. |

**Ownership:** the daemon is the sole writer of config files today
(`settings.json` via `set-config`). P1 extends that with `webapp-*` IPC so the
daemon owns both `webapps.json` **and** the `.desktop`/launcher generation. QML
never formats `.desktop` files (avoids the Bigscreen exec-injection footgun
flagged in `webappmanager.cpp`).

---

## 6. Phased plan

- **P0 — Foundation (this slice).** This doc + a **Web Apps** settings page
  (registered in `qmldir` + `SettingsPanel.sections` + `intent settings:webapps`)
  that lists registry entries with an empty state; `SettingsStore` gains a
  read-through `webApps` mirror loaded over IPC (graceful-empty when the daemon
  has no `webapp-list` yet). No `.desktop` writing yet.
- **P1 — Registry IPC (daemon).** `webapp-list` / `webapp-add` / `webapp-remove`
  commands: daemon owns `webapps.json` + `.desktop` generation + user-data-dir
  layout. Add to `docs/IPC_PROTOCOL.md`.
- **P2 — Add flow (QML).** Name + URL entry (depends on on-screen keyboard #20),
  curated presets (YouTube / Netflix / dashboards) for one-tap add, icon
  fetch/fallback, native-exists hint for Plex/Spotify.
- **P3 — Launcher generation.** Chromium `--app` launcher with stable
  `--class`/`StartupWMClass`; per-app `--user-data-dir`. On-device verification
  of the class flag on game-client-1.
- **P4 — Polish.** Edit/remove, icon caching, optional per-app user-agent,
  presets refinement.

## 7. What's unverified

- The exact Chromium flag that fixes the Hyprland window class on Fedora 43
  (`--class` vs `--app-id`/`--name`) — on-device only.
- Whether the daemon's `list-apps` picks up freshly written `.desktop` files
  without a cache rebuild (it scans the dir live, so likely yes — no
  `kbuildsycoca` equivalent needed, unlike Bigscreen).
- QML in this repo can't render on CI; the P0 page is validated with `qmllint`
  and by matching existing page patterns, not by on-device screenshot.
