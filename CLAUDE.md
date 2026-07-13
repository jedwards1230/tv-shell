# CLAUDE.md

@CONTRIBUTING.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

A 10-foot couch gaming UI built with [Quickshell](https://quickshell.org/) (QML) on Hyprland. Controller-navigable home screen for Moonlight game streaming, local app launching, and system settings. Designed for 4K@120Hz HDR on OLED displays.

## Architecture

```
SDDM → tv-shell-session.sh → Hyprland (kiosk) → Quickshell (shell.qml)
       │                                  │  ▲ intent:* control-surface stream + Keys
       │  Super* → super-intent.sh ───────┼──┘ (keyboard → intents: menu/home/reset)
                               └── tv-shell-input (Rust daemon: EVIOCGRAB the
                                   gamepad fleet → per-player uinput; backend IPC)
```

- **shell.qml** — entry point: state machine (`idle` → `launching` → `streaming` → `reconnecting`) and process management. Quickshell runs as the `tv-shell-quickshell.service` `systemd --user` unit (started by Hyprland `exec-once`, direct-spawn fallback) — a single-instance guarantee + restart-on-crash that kills the "stacked duplicate Quickshell instances" bug class (#254). See [docs/SYSTEMD_SETUP.md](docs/SYSTEMD_SETUP.md)
- **tv-shell-input** (Rust daemon, `daemon/`) — the sole backend. It owns the **gamepad fleet only**: grabs every connected pad exclusively via evdev (`EVIOCGRAB`, tracked by fd with a DB-match-or-reject discovery gate), manages hot-join/leave with stable per-player slots, and re-presents each pad as a clean per-player virtual gamepad in the game presenter. It emits nav keys + a first-class **`intent` control surface** (`intent <name>` command → `intent:*` broadcast — the closed vocabulary keyboard-escape and automation also ride), plus fleet outputs (rumble/battery/LED), and serves the full Unix-socket IPC (settings, app discovery, Bluetooth/network/power, Hyprland reads, Sunshine). **It does NOT read the keyboard** — the keyboard (K400) belongs to the compositor + QML (Wayland focus / `Keys`); Hyprland binds inject intents via `scripts/super-intent.sh`: bare **`Super` → `intent menu`** (toggle the nav drawer), **`Super+Escape` → `intent home`** (return-to-shell escape), **`Super+Backspace` → `intent home-hold`** (reset), **`Super+Right` → `intent overlay:session`** (open Session QAM). Build with `scripts/build-daemon.sh` (canonical; uses `--features cec,mcp`) or `cargo build --release --features cec,mcp` and install to `$TV_SHELL_DIR/bin/tv-shell-input`; the session script starts it as the `tv-shell-input.service` `systemd --user` unit (bare-process fallback when no user manager / under a `TV_SHELL_INPUT_BIN` dev override) — see [docs/SYSTEMD_SETUP.md](docs/SYSTEMD_SETUP.md)
- **ShellLayout.qml** — hosts every top-level surface (Home, Library, Settings, overlays, drawers) and owns the **ScreenManager** router. shell.qml reaches the shell only through `ShellLayout`'s API (`openSettings`/`closeSettings`, `toggleMenu`, `focusHome`, …), never into a surface's internals.
- **ScreenManager.qml** — minimal navigation model for the secondary-screen layer (Home is the base; Library/Settings open over it). `push("settings", {page})` / `push("library")` / `popToHome()` centralize the imperative show/hide + focus handoff. It does NOT own modal/overlay back-handling or the Settings-internal B-stack — it reacts to each surface's `closed` signal and never intercepts Escape. Visibility/focus **bindings** stay declarative on the surfaces.
- **settings/SettingsApp.qml** — the Settings "app": its own `shell.settings` module (the 10 sidebar pages + sidebar). Public API `open()` / `openPage(id)` / `close()` + `closed` signal. The `widgets`/`moonlight`/`streaming` deep-links are intercepted earlier in `ShellLayout.openSettings` (they route to the Widgets app), so they never reach SettingsApp.
- **widgets/WidgetsApp.qml** — the Widgets "app", mirroring SettingsApp: its own `shell.widgets` module with the same public-API shape (`open()` / `openPage(id)` deep-links into a widget's config, e.g. `"moonlight"` / `close()` + `closed` signal) that `ShellLayout`/`ScreenManager` drive — callers never reach into its internals. It owns the back-stack between two distinct leaf views: **`WidgetList.qml`** (L0 — the widget list, order model + reorder/toggle/configure) and **`WidgetConfig.qml`** (L1 — the manifest-driven per-widget config, with the Moonlight server surface inlined). It is schema-driven from `WidgetManifests` (no per-widget page code) and replaced the old monolithic `components/WidgetsScreen.qml`.
- **Theme.qml** — singleton (must be `Item`, not `QtObject` — Quickshell can't host Process/Timer children in QtObject) with all colors, fonts, and layout constants. Dark/light/auto mode state is read from `SettingsStore`
- **SettingsStore.qml** — singleton (also `Item`, for the same reason) that owns all QML-side settings I/O for `~/.config/tv-shell/settings.json` and the binding IPC (get/set/capture). Single source of truth for the settings schema
- **components/qmldir** — component registry. New components must be added here or Quickshell won't find them

### File Layout

```
shell/                       # QML shell — Quickshell config root (-c tv-shell)
  shell.qml                  # Entry point, state machine
  components/
    Theme.qml                # Singleton — colors, fonts, layout constants
    SettingsStore.qml        # Singleton — centralized settings I/O + binding IPC
    HomeScreen.qml           # Hero clock, app rows, status icons
    AppCard.qml              # Icon-centric app tile (Freedesktop icons)
    StreamCard.qml           # Moonlight streaming target card
    QuickActions.qml         # Top-right quick actions (notifications, settings, widgets, theme, network, volume, power)
    ShellLayout.qml          # Hosts every surface; owns the ScreenManager router
    ScreenManager.qml        # Minimal Home/Library/Settings/Widgets navigation model
    LibraryScreen.qml        # Secondary browse surface (Moonlight rail + Applications NavigableGrid)
    MoonlightSettings.qml    # Server management — stays here (streaming provider's settingsComponent)
    SettingsButton.qml       # Reusable button atom (also used by lib/)
    SettingsList.qml         # Reusable list-sizing atom
    SettingsEmptyState.qml   # Reusable empty-state card
    MarqueeText.qml          # Scrolling text for long names
    Drawer.qml               # Reusable slide-in drawer (any edge)
    NavigationDrawer.qml     # Left nav drawer: clock header, Resume hero (large running/recent tiles), Home + QuickActions row, Now-Playing mini-strip, bottom status glyph line
    resumeModel.js           # Pure .pragma library: merge/dedup running windows + recent apps → the drawer's Resume list (headless-testable)
    StreamOverlay.qml        # Reconnecting/error overlay
    lib/                     # Shared reusable component library (own qmldir module)
      SettingsDropdown.qml   #   Collapsible single-select dropdown (D-pad)
      SettingsButtonGroup.qml#   Horizontal chip selector (D-pad)
      HintBar.qml            #   Bottom-of-page hint text
      qmldir                 #   lib registry — `module components.lib`
    qmldir                   # Component registry — `module components`
  settings/                  # Settings module (own qmldir — `module shell.settings`)
    SettingsApp.qml          #   Public entry: sidebar + Loader content pane +
                             #   public API (open/openPage/close + `closed`)
    {Audio,Bluetooth,Network,Display,Controllers,KeyBindings,
     AVControl,Accessibility,Power,System}Settings.qml  # the 10 sidebar pages
                             # (Widgets is no longer a settings page — it's the
                             #  top-level WidgetsScreen surface in components/)
    icons/                   #   Sidebar section SVGs (resolved relative to SettingsApp)
    qmldir                   #   `module shell.settings`
    # Pages reach shared singletons/atoms via `import "../components"` and the
    # lib via `import "../components/lib"` (same relative-dir mechanism lib/ uses).
  widgets/                   # Widgets app module (own qmldir — `module shell.widgets`)
    WidgetsApp.qml           #   Public entry: back-stack (list↔config) + public API
                             #   (open/openPage/close + `closed`) — mirrors SettingsApp
    WidgetList.qml           #   L0 — widget list: order model, reorder/toggle/configure
    WidgetConfig.qml         #   L1 — manifest-driven per-widget config (+ inlined Moonlight)
    qmldir                   #   `module shell.widgets`
    lib/                     # Widget framework (own qmldir — `module shell.widgets.lib`)
      Widget.qml             #   Home-widget base (focus/visibility contract)
      WidgetHost.qml         #   Instantiates the registry + builds the generic focus chain
      WidgetRegistry.qml     #   Singleton — ordered home-widget set (id+Component, by persisted order)
      WidgetManifests.qml    #   Singleton — per-widget manifest data (id/name/version/requires/config schema)
      widgetConfig.js        #   Pure migrator: legacy flat widget* keys → widgets.<id>.* subtree
      FilterChips.qml        #   Shared chip-strip filter (used by SegmentedHeader)
      SegmentedHeader.qml    #   Shared segment-pill header (Plex / Moonlight / Apps)
      qmldir                 #   `module shell.widgets.lib`
    moonlight/               # Moonlight widget family (`module shell.widgets.moonlight`)
      MoonlightWidget.qml    #   Home widget: Moonlight servers (small) + the
                             #   shared Steam library view (medium/large),
                             #   imported from steamlib/
      qmldir                 #   `module shell.widgets.moonlight`
    steamlib/                # Shared Steam library view (`module shell.widgets.steamlib`)
      SteamLibraryView.qml   #   Steam library grid — hosted by BOTH MoonlightWidget
                             #   (medium/large) and SteamWidget (below)
      SteamCard.qml          #   Steam game poster card
      qmldir                 #   `module shell.widgets.steamlib`
    nowplaying/              # Now Playing widget family (`module shell.widgets.nowplaying`)
      NowPlayingWidget.qml   #   Home widget: size-switching MPRIS now-playing
      NowPlayingStripView.qml#   Compact transport strip (small size)
      qmldir                 #   `module shell.widgets.nowplaying`
                             #   (NowPlayingCard stays in components/ — shared with MediaWidget/SessionQAM)
    plex/                    # Plex widget family (`module shell.widgets.plex`)
      PlexWidget.qml         #   Home widget: Plex On Deck + Recently Added
      PlexCard.qml           #   Plex poster card
      qmldir                 #   `module shell.widgets.plex`
    apps/                    # Apps widget (`module shell.widgets.apps`)
      AppsWidget.qml         #   Home widget: SegmentedHeader (Recent / All Apps)
                             #   over ONE horizontal NavigableRow rail. id stays
                             #   "recent". (The vertical NavigableGrid of every app
                             #   lives in components/LibraryScreen.qml, not here.)
      qmldir                 #   `module shell.widgets.apps`
    steam/                   # Steam widget (`module shell.widgets.steam`)
      SteamWidget.qml        #   Home widget: hosts the shared steamlib/
                             #   SteamLibraryView; activation launches Steam
                             #   LOCALLY (not a Moonlight stream). Ships
                             #   disabled by default.
      qmldir                 #   `module shell.widgets.steam`
    # WidgetsApp/List/Config + lib/ reach shared singletons/atoms via
    # `import "../components"` and the lib via `import "../components/lib"`.
    # Each per-widget dir (moonlight/steamlib/nowplaying/plex/apps/steam) is one
    # level deeper: `import "../lib"` for the framework, `import "../../components"`
    # for singletons/atoms, `import "../../components/lib"` for lib types; same-dir
    # siblings resolve implicitly.
widgets-index.json            # Machine-readable widget catalog (id/name/version/minFrameworkVersion/requires).
                             # Generated/kept in-sync with WidgetManifests.qml — see SSOT note below.
config/
  hyprland.conf               # Generic monitor default + `source` hook for a per-machine override
  hyprland.conf.example       # Machine-specific display example (LG C2/Denon HDR) → ~/.config/tv-shell/hyprland-local.conf
  palette.md                  # Color palette documentation
  tv-shell.desktop          # Reference Wayland session file (install.sh does NOT read it — write_session_file() generates the installed session .desktop from a heredoc with the resolved Exec)
  tv-shell-input.service    # systemd --user unit for the daemon (install.sh rewrites ExecStart to the prefix; see docs/SYSTEMD_SETUP.md)
  targets.json.example        # Copy-runnable streaming targets → ~/.config/tv-shell/targets.json
  targets.yaml.example        # Annotated streaming-target field reference (docs only)
  config.toml.example         # Per-machine daemon options (typed TOML) → ~/.config/tv-shell/config.toml
daemon/                      # Rust backend daemon (tv-shell-input) — sole backend
  src/                       # input/uinput, ipc, config, apps, bluetooth, network, power, hyprland, health
  README.md                  # daemon architecture + phase notes
packaging/                   # PKGBUILD / install layout (see #147)
scripts/
  install.sh                  # Standalone install: build daemon + lay tree + register session (see docs/INSTALL.md)
  install-deps.sh             # Distro-aware system dependency installer
  tv-shell-session.sh       # Session wrapper launched by SDDM
  super-intent.sh             # Hyprland Super binds -> intents (Super=menu/drawer, +Escape=home, +Backspace=reset, +Right=overlay:session)
  check-widgets-index.py      # Consistency check: asserts widgets-index.json mirrors WidgetManifests.qml (run by CI)
```

### Shared Component Library (`lib/`)

Reusable, page-agnostic UI components live in **`shell/components/lib/`** — a
separate Quickshell module (`module components.lib`) with its own `qmldir`.
**Prefer extracting a shared component over copy-pasting a UI pattern.** If you
hand-roll the same structure on a second page (a dropdown, a chip selector, a
status pill, a modal/overlay shell), promote it to `lib/` instead.

Import convention (verified on-device):

- **A `lib/` component reaches parent singletons** (`Theme`, `Units`,
  `SettingsStore`) and sibling atoms (`SettingsButton`) via an **unnamed relative
  import** at the top of the file: `import "../"` — those names then resolve bare,
  exactly as in the flat `components/` module.
- **A flat `components/` file** (e.g. `HomeScreen.qml`, `NowPlayingCard.qml`,
  `MediaWidget.qml`) accesses **sibling `components` types** (`AppCard`,
  `NavigableRow`, `FocusFrame`, `MarqueeText`, …) **implicitly** — Quickshell
  resolves same-directory types, so **no import is needed** for them. Such a file
  adds **only** `import "lib"`, and that solely for the `components.lib` types it
  uses (e.g. `PointerTrackingArea`, `MprisPlayerBase`). **Never add `import "../"`
  to a flat `components/` file** — from `components/` that points at `shell/`, not
  at `components/`, so it does not provide sibling types and is used by no file here
  (verify: `grep -l 'import "\.\./"' shell/components/*.qml` returns nothing).
- **A per-widget dir** (`widgets/moonlight/`, `widgets/steamlib/`,
  `widgets/nowplaying/`, `widgets/plex/`, `widgets/apps/`, `widgets/steam/`) is two
  levels under `shell/`. Its files reach the widget framework via `import "../lib"`
  (`Widget`, `FilterChips`), the flat `components` singletons/atoms via
  `import "../../components"` (`Theme`, `Units`, `InputMode`, `NavigableRow`,
  `StreamCard`, `FocusFrame`, …), and `components.lib` types via
  `import "../../components/lib"` (`ServiceMonitor`, `ServiceStatusNotice`,
  `PointerTrackingArea`, `MprisPlayerBase`). Types in the **same** widget dir resolve
  implicitly. (`NowPlayingCard` stays in `components/`, so `nowplaying/` reaches it
  via `import "../../components"`.) `MoonlightWidget.qml` and `SteamWidget.qml` each
  reach the shared `SteamLibraryView`/`SteamCard` via `import "../steamlib"`.
- **A `lib/` file reaching a sibling `lib/` type** — a `components.lib` type or
  **singleton** in the same directory (e.g. `WidgetHost.qml` using the
  `WidgetRegistry` singleton, or any lib file using `MprisPlayerBase`) — resolves it
  **implicitly via same-directory resolution; no `import "."` is needed** (and none
  is used — `WidgetHost` references `WidgetRegistry` with no `import "."` and renders
  on-device). Singletons included: a same-directory `pragma Singleton` resolves the
  same way. `import "../"` from a `lib/` file is **only** for reaching PARENT
  (`components`) singletons, never for same-`lib` siblings.
- **A page consumes the library** with `import "lib"` — `lib/` types
  (`SettingsDropdown`, `SettingsButtonGroup`, `HintBar`, …) then resolve bare.
- Every new `lib/` type must be added to `shell/components/lib/qmldir`.

Constraints carried over from the flat module:

- Selectable/interactive `lib/` components must remain **`FocusScope`s** so
  `SettingsApp`'s outer Flickable scroll-follow (which tracks `activeFocusItem`)
  keeps working — never wrap controls in a `fillHeight` self-scrolling `ListView`.
- Pages own their `KeyNavigation` chains; a `lib/` component should expose
  `KeyNavigation.up`/`.down` (or equivalent aliases) so callers keep wiring focus.

Existing reusable atoms still in the flat `components/` dir (`BaseCard`,
`FocusFrame`, `NavigableRow`, `Drawer`, `SettingsButton`/`List`/`EmptyState`,
`DimmedBackdrop`) are migrated into `lib/` opportunistically, not all at once.

**`Widget.qml`** is the base type for home-screen widgets (Now Playing, Moonlight,
Plex, Apps). It bakes in the duck-typed focus contract HomeScreen + NavigableRow
query (`previousRow`/`nextRow`/`firstRow`/`lastRow`, `canFocus`/`regionFocused`,
`focusFirstChild()`, `widgetEnabled`, `size`, `escaped`, `ensureVisibleRequested`)
with single-stop defaults, so a widget extending it satisfies the contract for free
and overrides only what it needs. Up/Down traversal is inherited from the shared
`shell/components/lib/focusChain.js` helper (also used by `NavigableRow`/
`NavigableGrid`/`WakeCard`); `escaped` + `ensureVisibleRequested` are forwarded by
`WidgetHost` so `HomeScreen` wires them once. `MprisPlayerBase` extends it
(overriding `canFocus`/`focusFirstChild`).

**Per-widget config is namespaced (#249 Phase 3).** Each widget owns a
`widgets.<id>.{enabled,order,size,prefs}` subtree in `settings.json`, the QML SSOT
for its config. **`SettingsStore.widget(id)`** reads the (fully-defaulted) subtree;
`setWidget(id,key,value)` / `setWidgetPref(id,prefKey,value)` / `setWidgetOrder(ids)`
write it. There are NO flat `widget*` keys or `Theme.widget*` passthroughs anymore —
read config via `SettingsStore.widget(id)`. **`WidgetManifests.qml`** (singleton,
pure data) is the manifest SSOT: per-widget `id`/`name`/`version`/`requires`
(capability strings)/`config` schema (typed `bool|enum|int|string` controls, plus
the framework-owned `size` enum). **`widgetConfig.js`** (`.pragma library`, pure) is
the one-shot migrator that folds the legacy flat keys into the subtree on first load
(`widgetSpotify*` → `nowplaying`), preserving existing values; old flat keys linger
on disk one release (harmless — daemon shallow-merge preserves them).

**`WidgetRegistry.qml`** (singleton) is the hand-written home-widget set — one entry
per widget pairing its `id` + `Component` with its `enabled`/`size`/`order` bindings
(read from `SettingsStore.widget(id)`). Its `widgets` list is sorted by persisted
`order` (so the home column reflects the Widgets-page reorder); `order` is declared
`int` so a no-op recompute is suppressed and an unrelated enable/size toggle does NOT
rebuild the widget set. It is the single place to add a home widget (no codegen — the
repo forbids QML build tooling). The schema-driven **Widgets app** (`shell/widgets/`,
`module shell.widgets`: `WidgetsApp` + `WidgetList` + `WidgetConfig`) renders the
per-widget enable/size/prefs controls + a controller-navigable reorder list from the
manifests; it replaced the old monolithic `components/WidgetsScreen.qml` (which had
itself replaced `settings/WidgetsSettings.qml`).
**`WidgetHost.qml`** instantiates the registry set (Repeater + Loader)
into a `ColumnLayout` and builds the generic vertical focus chain that replaces
HomeScreen's former hand-wired `previousRow`/`nextRow` web: each widget's UP/DOWN
neighbour resolves to the nearest preceding/following focusable widget's
`lastRow`/`firstRow` (or the widget itself if single-stop), falling back to the
host's `topAnchor`/`bottomAnchor` (HomeScreen wires `topAnchor` to the QuickActions
row — also the never-strand focus fallback — and leaves `bottomAnchor` null: the
standalone All Apps tile was removed, so Down off the last widget is a no-op and the
home→Library jump is the Apps widget's "Open Library" chip). HomeScreen attaches
each widget's behaviour via
`widgetHost.widgetById(id)`. The two now-playing renderers are now ONE
`NowPlayingWidget` whose `size` selects a `NowPlayingCard` (medium) or
`NowPlayingStripView` (small) visual leaf; `MediaWidget` is a thin host of the same
`NowPlayingCard` for standalone use (e.g. SessionQAM).

## Key Data Flows

- **Streaming targets**: Loaded from `~/.config/tv-shell/targets.json` at startup (single-line JSON — see gotchas). The path is resolved client-side by the `Paths` QML singleton (`$TV_SHELL_TARGETS` env → else `${XDG_CONFIG_HOME:-$HOME/.config}/tv-shell/targets.json`); a missing file is a clean no-op (empty target list, no crash). Managed in-UI via MoonlightSettings. Optional `sunshineUser`/`sunshinePass`/`sunshinePort` fields enable pre-flight session detection via the Sunshine API — when present, the shell checks for active sessions before streaming and offers Resume/Quit/Cancel if a different app is running. Credentials should be injected by the deployment system, not committed.
- **Settings persistence**: `~/.config/tv-shell/settings.json` stores `themeMode`, `streamingViewMode`, `controllerDebug`, the per-widget `widgets.<id>.*` subtree (QML-owned) and `keyBindings` (daemon-owned). The **daemon is the sole writer** — `SettingsStore` reads via `get-config` and hands QML-owned keys to `set-config` (read-modify-write), so QML never formats config JSON itself. All QML-side I/O is centralized in the `SettingsStore` singleton — add new settings there (a property + load/save handling), not in Theme.qml. Theme delegates to SettingsStore. Per-widget config is namespaced under `widgets.<id>` and accessed via `SettingsStore.widget(id)` / `setWidget*` (see the Shared Component Library section); on first load a one-shot migrator folds the legacy flat `widget*` keys into that subtree, preserving values.
- **Config locations & paths**: The shell is prefix-agnostic. Per-user config lives under `~/.config/tv-shell/` (`settings.json`, `targets.json`, optional **`config.toml`** — typed per-machine daemon options, read directly by the daemon via `daemon_config.rs`; optional `hyprland-local.conf`); system defaults conventionally under `/etc/tv-shell/`. The install prefix is resolved at runtime — never hardcode `/opt/tv-shell`. Env vars (runtime/session only — **not** per-machine config, which is `config.toml`): `TV_SHELL_DIR` (install root; exported by the session script, also derived from `current_exe`), `TV_SHELL_INPUT_BIN` (override the daemon binary path for the re-exec/dev-override hook; falls back to `$TV_SHELL_DIR/bin/tv-shell-input`), `TV_SHELL_TARGETS` (override the streaming-targets file path), `TV_SHELL_SOCK` (daemon IPC socket). `/opt/tv-shell` survives only as a documented last-ditch fallback in `tv-shell-session.sh` and the daemon's `install_root()`.
- **App discovery & recents**: `AppDiscoveryManager` (apps via `list-apps`) and `RecentsTracker` (`get-recents` / `record-launch`) read JSON straight from the daemon, which owns the `.desktop` scanning (`freedesktop-desktop-entry` crate) and recents file. QML no longer parses `.desktop` files. The QML side talks to the daemon over a native `Quickshell.Io` socket via `SocketClient.qml` — the old per-call `python3 -c` Unix-socket shims were retired.
- **Input daemon IPC**: See [docs/IPC_PROTOCOL.md](docs/IPC_PROTOCOL.md) for the full protocol specification. QML sends commands via Unix socket; the daemon streams events to subscribers.
- **Input & state**: [docs/INPUT_AND_STATE.md](docs/INPUT_AND_STATE.md) is the canonical reference for the shell's state machine (`idle`/`launching`/`streaming`/`reconnecting`/`appRunning`), focus model, the per-context input-semantics matrix, B/Escape back precedence, and the `intent` control surface. Read it before changing any input/focus/nav code.
- **Settings panels**: SettingsApp uses a Loader to swap between section components. Each section manages its own system calls via `Quickshell.Io.Process`.

## System Integration

| Tool | Used For |
|------|----------|
| input daemon (IPC) | Gamepad grab/release, settings I/O (`get/set-config`), app discovery (`list-apps`), recents (`get/record`), Bluetooth (`bt-*`), network reads (`net-*`), suspend/battery (`power-*`), Hyprland reads (`hypr-active`/`hypr-clients` + `hypr:*` events), Sunshine pre-flight (`sunshine-status`) — see [docs/IPC_PROTOCOL.md](docs/IPC_PROTOCOL.md) |
| `wpctl` | Audio volume/mute/sink switching (WirePlumber/PipeWire) |
| `nmcli` | WiFi *join* only (`device wifi connect`) — reads go through the daemon's `net-*` IPC |
| `hyprctl` | Monitor mode/scale changes, app launching, reload, one-shot `dispatch` actions — window/client *reads* go through the daemon's `hypr-*` IPC |
| `systemctl` | Reboot/poweroff one-shots — suspend goes through the daemon's `power-suspend` IPC |
| `cec-client` | _retired_ — HDMI-CEC now owned by the daemon's `cec-*` IPC (`cec-rs`/libcec, #94); built only `--features cec`. The `cec-client`/`cec-ctl`/`living-room-cec` QML fallback chain is removed |
| `moonlight` | Game streaming client (`stream`, `list`, `pair`) |

Besides the owner-only Unix-socket IPC, the daemon can expose a **network-facing
control surface** — an HTTP bridge and an MCP server, both opt-in via
`config.toml` (`[http]`/`[mcp]`), sharing one bearer token (`[http].token_file`),
both thin adapters over `daemon/src/bridge_core.rs`. See
[`docs/CONTROL_SURFACE.md`](docs/CONTROL_SURFACE.md) (and the Agent-Native Dev Loop
under Development). The daemon also emits **observability** signals — structured
journald logs (stdout fallback, auto-detected via `JOURNAL_STREAM`, overridable by
`[observability].log_journal`) and Prometheus metrics via an auth-exempt
`GET /metrics` route plus an optional node_exporter textfile writer
(`[observability].metrics_textfile`); full catalogue in
[`docs/OBSERVABILITY.md`](docs/OBSERVABILITY.md).

The table above reflects the deliberate split: the daemon owns all *reads* of
system state (D-Bus, Hyprland IPC, Sunshine), while shell-outs remain only for
write/action commands (`nmcli` join, `wpctl`, `hyprctl dispatch`, `systemctl`).

**Kiosk window model** (single-app-fullscreen contract — see
[docs/KIOSK_WINDOW_MODEL.md](docs/KIOSK_WINDOW_MODEL.md)): the invariant "exactly
one app window fills the screen; backgrounded apps keep running but never share
it" is held **declaratively** by `config/hyprland.conf` first — a fresh launch
maps fullscreen via the QML `[fullscreen]` exec-rule prefix + the
`windowrule = fullscreen` backstop; a resume swaps fullscreen atomically via
`misc:on_focus_under_fullscreen = 1`; a close promotes the survivor via
`misc:exit_window_retains_fullscreen = true`; and `windowrule = suppress_event
fullscreen maximize` stops apps churning that state themselves. The daemon's
Hyprland actor is the **single idempotent backstop** on top of those rules
(`force_fullscreen` on `openwindow`, `enforce_active_fullscreen` on
`closewindow`/`movewindowv2`/`activewindowv2`, both `fullscreen 0 set`). QML no
longer re-asserts fullscreen on launch/resume — a non-idempotent `fullscreen 0`
toggle there used to race the daemon and toggle a resumed window back OUT of
fullscreen, which produced the two-app split view. The daemon actor also
**self-heals its compositor attachment**: it re-resolves the live Hyprland
instance on each reconnect (`session_env::resolve_hypr_signature` scans
`$XDG_RUNTIME_DIR/hypr/` before trusting an inherited signature) so a killed +
restarted Hyprland doesn't leave it silently deaf.
**HDMI-CEC** lives in the daemon (`cec-*` IPC, deployed and verified on gaming-client
with static-linked libcec — no system `libcec`/`libcec-dev` at build or runtime).
CEC startup/wake focus is gated by `cecFocusOnStartup` (default `false`) and
`cecFocusOnWake` (default `true`), both within the `[cec].lifecycle` master gate
(in `config.toml`).

### Widget sidecars (remote HTTP backends)

A home-screen widget that needs heavy backend logic ships a **sidecar process**.
`tv-shell-host` (the Steam library/launch backend) is the first and only one
today. It runs on a **different machine** (the gaming PC) and the daemon reaches
it **over HTTP on the LAN** with a bearer token (`[steam]` in `config.toml`) — the
daemon is an HTTP *client*, not a process supervisor: it never spawns,
health-restarts, or otherwise manages the sidecar's lifecycle ("health" =
reachability via `GET /status`). The reusable client plumbing lives in
`daemon/src/sidecar.rs`; the daemon↔sidecar JSON contract is single-sourced in the
`tv-shell-protocol` crate (`LibraryResponse`/`StatusResponse`/`LaunchRequest`),
(de)serialized by both sides so the wire shape can't drift. See
[docs/HOST_SETUP.md](docs/HOST_SETUP.md).

## Development

No build step — QML is interpreted by Quickshell at runtime. Deploy by syncing files to the target machine's install directory and restarting Quickshell.

### Deploy Cycle

```bash
# 1. Push changes to git
git push origin <branch>

# 2. Pull on target device (install prefix is resolved from $TV_SHELL_DIR /
#    current_exe — $TV_SHELL_DIR below is just whatever prefix you installed to)
ssh <device> "cd \$TV_SHELL_DIR && git pull"

# 3. Restart Quickshell — single-instance safe (#254). Quickshell runs as a
#    `systemd --user` unit, so ALWAYS restart via systemctl (never `killall
#    quickshell` + `&`, which races and stacks 3-4 instances — the #254 foot-gun).
systemctl --user restart tv-shell-quickshell.service
# Fallback for a non-systemd / bare-process session only — exact-match kill,
# verify zero, THEN relaunch (do NOT background a relaunch before the kill settles):
#   pkill -x quickshell; while pgrep -xc quickshell | grep -qv '^0$'; do sleep 0.2; done
#   SIG=$(ls /run/user/1000/hypr/ | tail -1)
#   WAYLAND_DISPLAY=wayland-1 XDG_RUNTIME_DIR=/run/user/1000 \
#     HYPRLAND_INSTANCE_SIGNATURE=$SIG quickshell -c tv-shell &
# Automated dev loop: prefer the daemon's `/dev/restart-shell` (HTTP) or MCP
# `restart_shell` — both serialized + single-instance (see docs/CONTROL_SURFACE.md).

# 4. Screenshot for verification
WAYLAND_DISPLAY=wayland-1 XDG_RUNTIME_DIR=/run/user/1000 grim /tmp/screenshot.png
```

### Rust Input Daemon

Install the built daemon binary to its resolved location:

```bash
install -m755 daemon/target/release/tv-shell-input "$TV_SHELL_DIR/bin/tv-shell-input"
```

**HDMI-CEC (`--features cec`) and the MCP server (`--features mcp`) are opt-in features.** A plain `cargo build` is C-free; the `cec` feature pulls in `cec-rs`/`libcec-sys` and **static-links a bundled libcec** — a prebuilt static libcec + p8-platform is fetched from ssalonen/libcec-static-builds and linked into the binary, so the daemon needs **no system `libcec`/`libcec-dev`** at build or runtime. The static path needs no bindgen/cmake/clang — only `libudev-dev` + `pkg-config` and network at build time to fetch the archive.

Requires Linux with evdev and uinput access. Auto-discovers gamepad by vendor/product ID (defaults: Xbox controller `045e:028e`, configurable via `GAMEPAD_VENDOR`/`GAMEPAD_PRODUCT` env vars). The Linux-only evdev/uinput/D-Bus modules build only on the target (or CI); the cross-platform subset (`protocol`, `config`, `state`, `device` GUID math, `apps`, `health`, `recents`) builds and tests on any host.

### QA Screenshots

Catalog of views/overlays/states to capture for visual QA (and how to reach each) — [`docs/qa-screenshot-views.md`](docs/qa-screenshot-views.md):

@docs/qa-screenshot-views.md

### Agent-Native Dev Loop

The daemon is built so an LLM agent can drive the **entire** dev/verify loop with
no human in the pixel-path. The daemon exposes its control surface over the network
two ways — a hand-rolled HTTP bridge and an official MCP server — both opt-in, both
documented in [`docs/CONTROL_SURFACE.md`](docs/CONTROL_SURFACE.md). The MCP server
makes the shell **drivable and observable as MCP tools**, so an agent runs a full
**observe → act → verify** loop:

1. `dev_deploy <ref>` → `dev_build` → `dev_restart_daemon` / `restart_shell` — pull a branch and rebuild on-device.
2. `send_intent` / `navigate` / `open_settings` / `launch_app` — drive the UI.
3. `take_screenshot` — **see** the result; `get_status` / `get_logs` — diagnose.
4. Compare against intent, repeat.

Enable on the deploy host in `~/.config/tv-shell/config.toml`: set `[mcp].bind`
(+ `[http].token_file` pointing at a 0600 token file), and `[mcp].dev = true` for
the deploy/build/restart tools. Point an MCP client at `http://<host>/mcp`. Note
the daemon REFUSES to start with a non-loopback bind + dev tools + auth-off
unless `[dev].allow_insecure_lan = true` (the intentional opt-in for a trusted
single-user LAN host). This is the in-repo, distribution-agnostic version of the external
screenshot/deploy automation — no host-management tooling required.

## Design Constraints

- **10-foot UI at 4K**: All font sizes and layout constants in Theme.qml are sized for couch-distance reading. Don't shrink them.
- **Controller-first navigation**: Every interactive element must be reachable via D-pad (arrow keys) and activatable with A (Enter). B (Escape) always goes back. Focus management is critical — use `KeyNavigation` chains and `Keys.on*Pressed` handlers.
- **Palette rules**: See `config/palette.md`. Never use gold for text. Crimson for focus/active states. Ember for secondary interactive elements. All overlay backdrops use `Qt.rgba(0, 0, 0, 0.7-0.85)`.
- **No build tooling**: No bundler, no compiler, no package manager for QML. Files are deployed as-is.
- **Distribution agnostic**: This repo has no knowledge of specific infrastructure, deployment tools, or host management. It's a standalone QML shell that runs on any Linux system with Hyprland + Quickshell.

## Gotchas

- **SplitParser reads line-by-line**: Any JSON loaded via `cat` + `SplitParser` must be single-line. Never pretty-print `targets.json` or `settings.json`.
- **Theme.qml is an Item, not QtObject**: Quickshell 0.3.0 can't host Process/Timer children inside QtObject. The singleton uses Item as its root type.
- **`image://icon/` for Freedesktop icons**: Use `Image { source: "image://icon/" + iconName }` to load icons from the system theme. Falls back to nothing if the icon doesn't exist — provide a letter-initial fallback.
- **qmldir must list new components**: Quickshell won't auto-discover them. Add a line like `MyComponent 1.0 MyComponent.qml`. There are **ten** registries — flat components go in `components/qmldir` (`module components`); shared library components go in `components/lib/qmldir` (`module components.lib`); settings pages go in `settings/qmldir` (`module shell.settings`); the Widgets app's parts go in `widgets/qmldir` (`module shell.widgets`); the widget framework (Widget/WidgetHost/WidgetRegistry/WidgetManifests/FilterChips) goes in `widgets/lib/qmldir` (`module shell.widgets.lib`); the shared Steam library view goes in `widgets/steamlib/qmldir` (`module shell.widgets.steamlib`); and each home-widget family owns its own dir + qmldir — `widgets/moonlight/` (`module shell.widgets.moonlight`), `widgets/nowplaying/` (`module shell.widgets.nowplaying`), `widgets/plex/` (`module shell.widgets.plex`), `widgets/apps/` (`module shell.widgets.apps`). Cross-module reach uses relative-dir imports of the target directory's qmldir: a `lib/` file uses `import "../"` to see parent singletons; a page uses `import "lib"` for library types; a **settings or widgets page** uses `import "../components"` (singletons/atoms) + `import "../components/lib"` (lib types). All are the same mechanism — a relative directory import pulls that dir's qmldir types into bare scope (see [Shared Component Library](#shared-component-library-lib)).
- **WAYLAND_DISPLAY may vary**: Usually `wayland-1` but try `wayland-0` if grim/hyprctl fails.
- **Hyprland instance signature**: Multiple instances may exist in `/run/user/1000/hypr/`; use `tail -1` for the latest.
- **Theme property renames cascade**: `Theme.text` → `Theme.textPrimary` will also hit `Theme.textDim` producing `Theme.textPrimaryDim`. Replace longest matches first.

## CI

- **Orchestrator + single required check (`ci.yml`)**: there is ONE umbrella workflow, `ci.yml`, with no path filter — it runs on every PR (and push to main) so it always reports. A `changes` job (`dorny/paths-filter`) detects which areas changed; each area runs as a **reusable workflow** (`on: workflow_call`) invoked only when its area changed; a final **`ci-gate`** job aggregates results (skipped area = success). **Mark only `CI / ci-gate` as the required status check** — path-filtered workflows can't be required directly (a PR not touching their paths would leave the check "waiting" forever and be unmergeable). To add an area: add a `dorny` filter + a conditional `uses:` job in `ci.yml` and wire it into `ci-gate`'s `needs`.
- **QML formatting** (`lint.yml`, reusable): `qmlformat` (Qt 6.8). On PRs, unformatted files are auto-fixed and pushed (needs `contents: write` + `secrets: inherit`, passed by `ci.yml`). On main, unformatted files fail the check.
- **Rust CI** (`rust.yml` / `host.yml`, reusable): `rust.yml` builds/lints/tests the daemon (`tv-shell-input`) on Linux — a default leg plus a `--features cec` leg (static-libcec, on `rust:1-trixie`). `host.yml` builds/lints/tests the cross-platform host + protocol crates on Linux/macOS/Windows. Both stay `-p`-scoped so the daemon's Linux-only graph never leaks into the host's cross-platform build; `ci.yml`'s change detection (not per-workflow `paths:`) decides whether each runs, so a QML-only PR triggers neither.
- **Headless QML tests** (`qml-test.yml`, reusable): `qmltestrunner` under `QT_QPA_PLATFORM=offscreen` runs the layout/navigation suite in `tests/qml/` (see `tests/qml/README.md`).
- **Releases — three independent tag streams**: releases are triggered by pushing a version tag, not on merge-to-main. There are three independent streams:
  - `host-v<semver>` → `release-host.yml` builds `tv-shell-host` for linux-musl / macOS (arm64+x86_64) / windows and publishes one Release with all binaries + `checksums.txt`. Consumed by the homelab `desktop-common` Ansible role (`install_method: fetch`).
  - `input-v<semver>` → `release-input.yml` builds `tv-shell-input` (`--features cec,mcp`, linux-gnu) and publishes its binary + `checksums.txt`.
  - `widget-<id>-v<semver>` → `release-widget.yml` publishes a notes-only GitHub Release for the named widget. No binary is built — QML is interpreted at runtime. The workflow validates that `<id>` exists in `widgets-index.json` and that `<semver>` matches its recorded version before publishing. Example: `widget-moonlight-v1.1.0`.

  This is a deliberate deviation from the org's PR-label-driven `ai-release.yml` convention: the artifacts are versioned independently within one monorepo, so the tag prefix (not a merged-PR `semver:*` label) selects what to release. Release notes come from `generate_release_notes`.

- **Widget index consistency** (`widgets.yml`, reusable): `scripts/check-widgets-index.py` asserts that `widgets-index.json` is in-sync with `shell/widgets/lib/WidgetManifests.qml`. **`WidgetManifests.qml` is the authoring SSOT** — update it first, then update `widgets-index.json` to match. The check runs in CI whenever either file (or the script itself) changes; a drift fails the check and blocks merge. A PR that doesn't touch either file skips the check (no wedging). To push a widget release tag: bump `version` in both files, merge to main, then push `widget-<id>-v<X.Y.Z>`.
