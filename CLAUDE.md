# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

A 10-foot couch gaming UI built with [Quickshell](https://quickshell.org/) (QML) on Hyprland. Controller-navigable home screen for Moonlight game streaming, local app launching, and system settings. Designed for 4K@120Hz HDR on OLED displays.

## Architecture

```
SDDM → game-shell-session.sh → Hyprland (kiosk) → Quickshell (shell.qml)
       │                                  │  ▲ intent:* control-surface stream + Keys
       │  Super* → super-intent.sh ───────┼──┘ (keyboard → intents: menu/home/reset)
                               └── game-shell-input (Rust daemon: EVIOCGRAB the
                                   gamepad fleet → per-player uinput; backend IPC)
```

- **shell.qml** — entry point: state machine (`idle` → `launching` → `streaming` → `reconnecting`) and process management
- **game-shell-input** (Rust daemon, `daemon/`) — the sole backend. It owns the **gamepad fleet only**: grabs every connected pad exclusively via evdev (`EVIOCGRAB`, tracked by fd with a DB-match-or-reject discovery gate), manages hot-join/leave with stable per-player slots, and re-presents each pad as a clean per-player virtual gamepad in the game presenter. It emits nav keys + a first-class **`intent` control surface** (`intent <name>` command → `intent:*` broadcast — the closed vocabulary keyboard-escape and automation also ride), plus fleet outputs (rumble/battery/LED), and serves the full Unix-socket IPC (settings, app discovery, Bluetooth/network/power, Hyprland reads, Sunshine). **It does NOT read the keyboard** — the keyboard (K400) belongs to the compositor + QML (Wayland focus / `Keys`); Hyprland binds inject intents via `scripts/super-intent.sh`: bare **`Super` → `intent menu`** (toggle the nav drawer), **`Super+Escape` → `intent home`** (return-to-shell escape), **`Super+Backspace` → `intent home-hold`** (reset), **`Super+Right` → `intent overlay:session`** (open Session QAM). Build with `scripts/build-daemon.sh` (canonical; uses `--features cec,mcp`) or `cargo build --release --features cec,mcp` and install to `$GAME_SHELL_DIR/bin/game-shell-input`; the session script starts it as the `game-shell-input.service` `systemd --user` unit (bare-process fallback when no user manager / under a `GAME_SHELL_INPUT_BIN` dev override) — see [docs/SYSTEMD_SETUP.md](docs/SYSTEMD_SETUP.md)
- **ShellLayout.qml** — hosts every top-level surface (Home, Library, Settings, overlays, drawers) and owns the **ScreenManager** router. shell.qml reaches the shell only through `ShellLayout`'s API (`openSettings`/`closeSettings`, `toggleMenu`, `focusHome`, …), never into a surface's internals.
- **ScreenManager.qml** — minimal navigation model for the secondary-screen layer (Home is the base; Library/Settings open over it). `push("settings", {page})` / `push("library")` / `popToHome()` centralize the imperative show/hide + focus handoff. It does NOT own modal/overlay back-handling or the Settings-internal B-stack — it reacts to each surface's `closed` signal and never intercepts Escape. Visibility/focus **bindings** stay declarative on the surfaces.
- **settings/SettingsApp.qml** — the Settings "app": its own `shell.settings` module (the 11 pages + sidebar). Public API `open()` / `openPage(id)` / `close()` + `closed` signal; deep-link slugs and the moonlight/streaming reroute live in `openSectionById` behind `openPage`.
- **Theme.qml** — singleton (must be `Item`, not `QtObject` — Quickshell can't host Process/Timer children in QtObject) with all colors, fonts, and layout constants. Dark/light/auto mode state is read from `SettingsStore`
- **SettingsStore.qml** — singleton (also `Item`, for the same reason) that owns all QML-side settings I/O for `~/.config/game-shell/settings.json` and the binding IPC (get/set/capture). Single source of truth for the settings schema
- **components/qmldir** — component registry. New components must be added here or Quickshell won't find them

### File Layout

```
shell/                       # QML shell — Quickshell config root (-c game-shell)
  shell.qml                  # Entry point, state machine
  components/
    Theme.qml                # Singleton — colors, fonts, layout constants
    SettingsStore.qml        # Singleton — centralized settings I/O + binding IPC
    HomeScreen.qml           # Hero clock, app rows, status icons
    AppCard.qml              # Icon-centric app tile (Freedesktop icons)
    StreamCard.qml           # Moonlight streaming target card
    QuickActions.qml         # Top-right quick actions (volume, network, theme, power)
    ShellLayout.qml          # Hosts every surface; owns the ScreenManager router
    ScreenManager.qml        # Minimal Home/Library/Settings navigation model
    LibraryScreen.qml        # Secondary browse surface (Moonlight + Applications)
    MoonlightSettings.qml    # Server management — stays here (streaming provider's settingsComponent)
    SettingsButton.qml       # Reusable button atom (also used by lib/)
    SettingsList.qml         # Reusable list-sizing atom
    SettingsEmptyState.qml   # Reusable empty-state card
    MarqueeText.qml          # Scrolling text for long names
    Drawer.qml               # Reusable slide-in drawer (any edge)
    NavigationDrawer.qml     # Left nav drawer (Home, Settings)
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
     AVControl,Widgets,Accessibility,Power,System}Settings.qml  # the 11 pages
    icons/                   #   Sidebar section SVGs (resolved relative to SettingsApp)
    qmldir                   #   `module shell.settings`
    # Pages reach shared singletons/atoms via `import "../components"` and the
    # lib via `import "../components/lib"` (same relative-dir mechanism lib/ uses).
config/
  hyprland.conf               # Generic monitor default + `source` hook for a per-machine override
  hyprland.conf.example       # Machine-specific display example (LG C2/Denon HDR) → ~/.config/game-shell/hyprland-local.conf
  palette.md                  # Color palette documentation
  game-shell.desktop          # Wayland session file (install.sh rewrites Exec to the prefix)
  game-shell-input.service    # systemd --user unit for the daemon (install.sh rewrites ExecStart to the prefix; see docs/SYSTEMD_SETUP.md)
  targets.json.example        # Copy-runnable streaming targets → ~/.config/game-shell/targets.json
  targets.yaml.example        # Annotated streaming-target field reference (docs only)
  config.toml.example         # Per-machine daemon options (typed TOML) → ~/.config/game-shell/config.toml
daemon/                      # Rust backend daemon (game-shell-input) — sole backend
  src/                       # input/uinput, ipc, config, apps, bluetooth, network, power, hyprland, health
  README.md                  # daemon architecture + phase notes
packaging/                   # PKGBUILD / install layout (see #147)
scripts/
  install.sh                  # Standalone install: build daemon + lay tree + register session (see docs/INSTALL.md)
  install-deps.sh             # Distro-aware system dependency installer
  game-shell-session.sh       # Session wrapper launched by SDDM
  super-intent.sh             # Hyprland Super binds -> intents (Super=menu/drawer, +Escape=home, +Backspace=reset, +Right=overlay:session)
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

## Key Data Flows

- **Streaming targets**: Loaded from `~/.config/game-shell/targets.json` at startup (single-line JSON — see gotchas). The path is resolved client-side by the `Paths` QML singleton (`$GAME_SHELL_TARGETS` env → else `${XDG_CONFIG_HOME:-$HOME/.config}/game-shell/targets.json`); a missing file is a clean no-op (empty target list, no crash). Managed in-UI via MoonlightSettings. Optional `sunshineUser`/`sunshinePass`/`sunshinePort` fields enable pre-flight session detection via the Sunshine API — when present, the shell checks for active sessions before streaming and offers Resume/Quit/Cancel if a different app is running. Credentials should be injected by the deployment system, not committed.
- **Settings persistence**: `~/.config/game-shell/settings.json` stores `themeMode`, `streamingViewMode`, `controllerDebug` (QML-owned) and `keyBindings` (daemon-owned). The **daemon is the sole writer** — `SettingsStore` reads via `get-config` and hands QML-owned keys to `set-config` (read-modify-write), so QML never formats config JSON itself. All QML-side I/O is centralized in the `SettingsStore` singleton — add new settings there (a property + load/save handling), not in Theme.qml. Theme delegates to SettingsStore.
- **Config locations & paths**: The shell is prefix-agnostic. Per-user config lives under `~/.config/game-shell/` (`settings.json`, `targets.json`, optional **`config.toml`** — typed per-machine daemon options, read directly by the daemon via `daemon_config.rs`; optional `hyprland-local.conf`); system defaults conventionally under `/etc/game-shell/`. The install prefix is resolved at runtime — never hardcode `/opt/game-shell`. Env vars (runtime/session only — **not** per-machine config, which is `config.toml`): `GAME_SHELL_DIR` (install root; exported by the session script, also derived from `current_exe`), `GAME_SHELL_INPUT_BIN` (override the daemon binary path for the re-exec/dev-override hook; falls back to `$GAME_SHELL_DIR/bin/game-shell-input`), `GAME_SHELL_TARGETS` (override the streaming-targets file path), `GAME_SHELL_SOCK` (daemon IPC socket). `/opt/game-shell` survives only as a documented last-ditch fallback in `game-shell-session.sh` and the daemon's `install_root()`.
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
**HDMI-CEC** lives in the daemon (`cec-*` IPC, deployed and verified on gaming-client
with static-linked libcec — no system `libcec`/`libcec-dev` at build or runtime).
CEC startup/wake focus is gated by `cecFocusOnStartup` (default `false`) and
`cecFocusOnWake` (default `true`), both within the `[cec].lifecycle` master gate
(in `config.toml`).

## Development

No build step — QML is interpreted by Quickshell at runtime. Deploy by syncing files to the target machine's install directory and restarting Quickshell.

### Deploy Cycle

```bash
# 1. Push changes to git
git push origin <branch>

# 2. Pull on target device (install prefix is resolved from $GAME_SHELL_DIR /
#    current_exe — $GAME_SHELL_DIR below is just whatever prefix you installed to)
ssh <device> "cd \$GAME_SHELL_DIR && git pull"

# 3. Restart Quickshell (find Hyprland signature first)
SIG=$(ls /run/user/1000/hypr/ | tail -1)
WAYLAND_DISPLAY=wayland-1 XDG_RUNTIME_DIR=/run/user/1000 \
  HYPRLAND_INSTANCE_SIGNATURE=$SIG hyprctl dispatch exec 'killall quickshell'
sleep 1
WAYLAND_DISPLAY=wayland-1 XDG_RUNTIME_DIR=/run/user/1000 \
  HYPRLAND_INSTANCE_SIGNATURE=$SIG quickshell -c game-shell &

# 4. Screenshot for verification
WAYLAND_DISPLAY=wayland-1 XDG_RUNTIME_DIR=/run/user/1000 grim /tmp/screenshot.png
```

### Rust Input Daemon

For on-device / deploy builds, use the canonical script which sets the right feature flags:

```bash
./scripts/build-daemon.sh   # equivalent to: cd daemon && cargo build --release --features cec,mcp
# Install to the daemon binary's resolved location ($GAME_SHELL_DIR/bin by
# default; $GAME_SHELL_INPUT_BIN overrides the exact path).
install -m755 daemon/target/release/game-shell-input "$GAME_SHELL_DIR/bin/game-shell-input"
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

Enable on the deploy host in `~/.config/game-shell/config.toml`: set `[mcp].bind`
(+ `[http].token_file` pointing at a 0600 token file), and `[mcp].dev = true` for
the deploy/build/restart tools. Point an MCP client at `http://<host>/mcp`. Note
the daemon REFUSES to start with a non-loopback bind + dev tools + auth-off
unless `[dev].allow_insecure_lan = true` (the intentional opt-in game-client-1
uses). This is the in-repo, distribution-agnostic version of the external
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
- **qmldir must list new components**: Quickshell won't auto-discover them. Add a line like `MyComponent 1.0 MyComponent.qml`. There are **three** registries — flat components go in `components/qmldir` (`module components`); shared library components go in `components/lib/qmldir` (`module components.lib`); settings pages go in `settings/qmldir` (`module shell.settings`). Cross-module reach uses relative-dir imports of the target directory's qmldir: a `lib/` file uses `import "../"` to see parent singletons; a page uses `import "lib"` for library types; a **settings page** uses `import "../components"` (singletons/atoms) + `import "../components/lib"` (lib types). All four are the same mechanism — a relative directory import pulls that dir's qmldir types into bare scope (see [Shared Component Library](#shared-component-library-lib)).
- **WAYLAND_DISPLAY may vary**: Usually `wayland-1` but try `wayland-0` if grim/hyprctl fails.
- **Hyprland instance signature**: Multiple instances may exist in `/run/user/1000/hypr/`; use `tail -1` for the latest.
- **Theme property renames cascade**: `Theme.text` → `Theme.textPrimary` will also hit `Theme.textDim` producing `Theme.textPrimaryDim`. Replace longest matches first.

## CI

- **QML formatting**: `qmlformat` (Qt 6.8) enforced via GitHub Actions. On PRs, unformatted files are auto-fixed and pushed. On main, unformatted files fail the check.
- **Rust CI**: `rust.yml` builds/lints/tests the daemon (`game-shell-input`) on Linux — a default leg plus a `--features cec` leg (static-libcec, on `rust:1-trixie`). `host.yml` builds/lints/tests the cross-platform host + protocol crates on Linux/macOS/Windows. Both are path-filtered and `-p`-scoped so a QML-only PR triggers neither and the daemon's Linux-only graph never leaks into the host's cross-platform build.
- **Releases — per-binary tags**: artifacts are released by pushing a per-binary tag, not on merge-to-main:
  - `host-v<semver>` → `release-host.yml` builds `game-shell-host` for linux-musl / macOS (arm64+x86_64) / windows and publishes one Release with all binaries + `checksums.txt`. Consumed by the homelab `desktop-common` Ansible role (`install_method: fetch`).
  - `input-v<semver>` → `release-input.yml` builds `game-shell-input` (`--features cec,mcp`, linux-gnu) and publishes its binary + `checksums.txt`.

  This is a deliberate deviation from the org's PR-label-driven `ai-release.yml` convention: the two binaries are versioned independently within one monorepo, so the tag prefix (not a merged-PR `semver:*` label) selects what to build. Release notes come from `generate_release_notes`.
