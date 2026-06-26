# Widget Framework — Design / PRD

> **Status:** Draft for review · **Owner:** @jedwards1230 · **Scope:** turn game-shell's
> informal "widget" convention into a real, extensible framework on top of QML + the Rust
> daemon, so widgets (Moonlight, Now Playing, Plex, …) become self-contained, independently
> versioned units that a *flavor* (config profile) composes. Goal: make the repo public and
> let others add/select widgets without forking the shell core.

## 1. Why

Today a "widget" is a **convention, not a contract**. The exploration of the current code found:

- **No base class.** Widgets conform to a duck-typed "home-tile focus contract"
  (`widgetEnabled`, `size`, `previousRow`/`nextRow`, `regionFocused`, `canFocus`,
  `focusFirstChild()`) documented in `HomeScreen.qml` (#249) but enforced nowhere.
  `lib/MprisPlayerBase.qml` is the *only* real shared base (Now Playing only).
- **Hand-wired instantiation.** `HomeScreen` statically declares the four widgets in a
  `ColumnLayout` and builds the focus chain from a hardcoded `_contentRegions()` list. The
  code comment admits the duck-typed walk is "fragile by design."
- **Config is flat + hardcoded.** `SettingsStore` carries `widgetSpotifyEnabled`,
  `widgetPlexSize`, `widgetMoonlightEnabled`, … as individual properties; `Theme`
  re-exposes each as a readonly passthrough; `WidgetsSettings.qml` hand-codes a row per
  widget. Adding a widget is a 4+ file edit across `HomeScreen`, `SettingsStore`, `Theme`,
  and `WidgetsSettings`.
- **Knowledge is duplicated across the Rust↔QML seam.** The MCP `SettingsPage` enum
  (`daemon/src/mcp.rs`, 12 slugs) mirrors QML's settings sidebar; the `_widgets` catalog in
  `HomeScreen` re-lists the same widgets a third time.

The boundary that *is* clean — the daemon as a capability host talking to QML over a
stringly-typed intent socket — is the part to preserve. The mess is entirely on the QML
side, plus the small amount of widget knowledge that leaked into Rust.

## 2. Goals / Non-goals

**Goals**
- A formal `Widget` base class that owns the focus/visibility/contract plumbing.
- A **per-widget manifest** (`widget.json`) that is the single source of truth: identity,
  version, entry component, required capabilities, and config schema.
- Registry-driven instantiation + generic focus chain (a `WidgetHost`), so adding a widget
  is *one directory*, not edits scattered across the shell core.
- **Namespaced, schema-driven config**: each widget owns a `widgets.<id>.*` subtree; the
  daemon keeps treating `settings.json` as opaque JSON.
- A dedicated, **top-level Widgets page** generated from manifests — not a hand-coded
  afterthought inside Settings.
- **Per-widget versioning + changelog** in the one monorepo, extending the existing
  `input-v*` / `host-v*` independent-tag scheme to `widget-<id>-v*`.
- **Flavors**: a config profile selects which widgets are enabled, their order/size, and
  per-widget prefs. `game-client-1` ships one flavor; the public default is another.

**Non-goals (for v1)**
- Runtime / out-of-tree widget discovery. Widgets are **in-tree**; contribution is via
  PR/fork. The manifest format is designed so runtime discovery can be added later without
  redesign.
- A sandbox. QML widgets run in the shell process with full context access — they are
  **trusted code** (see §10). No isolation is promised.
- Third-party *compiled-in* Rust modules. The third-party backend surface is a **sidecar
  process** (§8), not a daemon recompile.
- Rebranding / extracting the framework into a separate repo. game-shell *is* the
  framework; flavors are config.

## 3. Architecture overview

```
┌───────────────────────────────────────────────────────────────┐
│ QML shell (Quickshell)                                        │
│                                                               │
│  WidgetRegistry  ◄── (build-time codegen from widget.json)    │
│       │  enabled + ordered list, per-widget config schema     │
│       ▼                                                       │
│  WidgetHost ── instantiates ── Widget (base) ◄── MoonlightWidget │
│       │  builds focus chain generically       ◄── PlexWidget    │
│       │                                       ◄── NowPlayingWidget│
│  WidgetsPage ── renders config form from each manifest schema │
│       │                                                       │
│  Config (namespaced: widgets.<id>.{enabled,size,order,prefs}) │
└───────────────────────────────────────────────────────────────┘
        │ get-config / set-config / subscribe (Unix socket, text)
        ▼
┌───────────────────────────────────────────────────────────────┐
│ Rust daemon = capability host (UI-agnostic)                   │
│  • settings.json sole writer, opaque JSON                     │
│  • intents / event bus / MPRIS / network / app launch         │
│  • optional per-widget *core* backend module (plex.rs, …)     │
└───────────────────────────────────────────────────────────────┘
        │ game-shell-protocol (typed serde, IPC)
        ▼
┌───────────────────────────────────────────────────────────────┐
│ Widget sidecars (separate processes) — game-shell-host today  │
│  Steam library/launch; future: any heavy widget backend       │
└───────────────────────────────────────────────────────────────┘
```

**Principle: the daemon stays widget-agnostic.** It deals in generic capabilities and
opaque config. A widget's *meaning* lives in its directory (QML + manifest), not in Rust.
The one piece of Rust that knows widget identities today (the MCP `SettingsPage` enum)
is replaced by manifest-driven generation (§9).

## 4. The widget manifest — the keystone

Every widget is a directory under `shell/widgets/<id>/` with a `widget.json`:

```jsonc
// shell/widgets/moonlight/widget.json
{
  "id": "moonlight",                 // stable, kebab; config namespace + tag prefix
  "name": "Moonlight",
  "description": "Game streaming (Sunshine/Moonlight) + Steam library.",
  "version": "1.2.0",                // semver; drives widget-moonlight-v* releases
  "author": "jedwards1230",
  "entry": "MoonlightWidget.qml",    // root component must extend the Widget base
  "minFrameworkVersion": "1.0.0",

  // Capabilities the host/daemon must provide; a missing one greys the widget
  // and surfaces a reason on the Widgets page rather than crashing.
  "requires": ["network", "moonlight-backend"],

  // Config schema → drives the Widgets page form AND the namespaced defaults.
  // `enabled` and `order` are implicit framework keys; `size` is declared when
  // the widget supports multiple sizes.
  "config": {
    "size":        { "type": "enum", "values": ["small", "medium", "large"], "default": "large" },
    "preferSteam": { "type": "bool", "default": false, "label": "Show Steam library first" }
  }
}
```

Field notes:
- `id` is the single identity used for the config namespace (`widgets.moonlight.*`), the
  registry key, and the release tag (`widget-moonlight-v1.2.0`).
- `requires` is a **capability gate**, not an import. Capabilities are a curated vocabulary
  the daemon advertises (e.g. `network`, `mpris`, `plex-backend`, `moonlight-backend`,
  `steam-sidecar`). The host disables a widget whose capability is unavailable and shows
  why — this is how a public user without a Plex server simply doesn't see the Plex widget.
- `config` is a small typed schema (`bool` | `enum` | `int` | `string`). The framework owns
  `enabled` (bool) and `order` (int) for every widget; the manifest only adds widget-specific
  prefs. The schema is the contract the Widgets page renders against.

The manifest is read by **both sides**:
- **QML** (via codegen): registry, Widgets-page form, focus order.
- **Rust** (via codegen): the capability set it must honor and the MCP page list — replacing
  the hand-maintained `SettingsPage` enum. Values stay opaque; only the *key set* is generated.

## 5. The `Widget` base class

`shell/widgets/lib/Widget.qml` — a `FocusScope` that bakes in the contract plumbing so an
author overrides only content + a couple of hooks. Strawman:

```qml
// shell/widgets/lib/Widget.qml  (module shell.widgets.lib)
import QtQuick
import "../../components"        // Theme, InputMode singletons

FocusScope {
    id: root

    // ---- Injected by the host from the registry + config ----
    required property string widgetId
    property bool   widgetEnabled: true
    property string size: "medium"
    property var    config: ({})              // widgets.<id>.prefs, typed by manifest
    property Item   previousRow: null         // host-wired neighbors
    property Item   nextRow: null

    // ---- Authoring surface (widget overrides) ----
    property bool shouldShow: true            // content-readiness gate
    default property alias content: body.data
    function focusFirstChild() {              // default: focus first focusable in body
        return _focusFirst(body)
    }

    // ---- Contract plumbing (host reads; author does not touch) ----
    readonly property bool regionFocused: activeFocus
    readonly property bool canFocus: widgetEnabled && shouldShow
    visible: widgetEnabled && shouldShow

    // standard accent bar, input-mode (mouse vs controller) handling, and
    // up/down neighbor navigation live here, once, for every widget.
    Keys.onUpPressed:   if (previousRow) previousRow.forceActiveFocus()
    Keys.onDownPressed: if (nextRow)     nextRow.forceActiveFocus()

    Item { id: body; anchors.fill: parent }
}
```

A concrete widget shrinks to its content:

```qml
// shell/widgets/plex/PlexWidget.qml
import "../lib"
Widget {
    widgetId: "plex"
    shouldShow: hasOnDeck || hasRecent
    // … PlexCard rows as content …
}
```

`MprisPlayerBase` is refactored to extend `Widget` (it already implements the contract by
hand), so Now Playing keeps working and the dedup stays.

## 6. `WidgetHost` + the registry

`WidgetHost.qml` replaces the hand-wired portion of `HomeScreen`. It reads the
**enabled, ordered** list from `WidgetRegistry` and builds the focus chain generically:

```qml
// shell/widgets/lib/WidgetHost.qml  (sketch)
ColumnLayout {
    Repeater {
        model: WidgetRegistry.enabledOrdered          // [{id, component, schema}, …]
        delegate: Loader {
            sourceComponent: modelData.component       // codegen-mapped Component
            onLoaded: {
                item.widgetId      = modelData.id
                item.widgetEnabled = Config.widget(modelData.id).enabled
                item.size          = Config.widget(modelData.id).size
                item.config        = Config.widget(modelData.id).prefs
            }
        }
    }
    // post-pass after all Loaders settle: walk visible items in order and set
    // previousRow/nextRow programmatically — the chain HomeScreen hardcodes today.
}
```

**Registry generation (build-time).** A script (`scripts/gen-widget-registry.*`) scans
`shell/widgets/*/widget.json` and emits:
- `shell/widgets/lib/WidgetRegistry.qml` — a singleton listing `{id, component, schema,
  requires, version}` with `import`-resolved `Component`s (compile-time safe, matches
  Quickshell's static module model).
- `daemon/src/widgets_generated.rs` — the capability set + MCP page list.

The generator runs in CI (and a pre-commit / `just gen` locally); a check job fails if the
generated files are stale. This keeps "one manifest, two consumers" honest without a runtime
parse. Runtime discovery (scan a dir, build the registry live) is a drop-in replacement for
the generated `WidgetRegistry` later — same shape, different source.

## 7. Config & flavors

**Namespaced config.** The flat `widget<Name>Enabled/Size` keys in `SettingsStore` migrate
to a per-widget subtree the widget owns:

```jsonc
// settings.json (daemon-owned, still opaque to the daemon)
"widgets": {
  "moonlight":  { "enabled": true,  "order": 0, "size": "large",  "prefs": { "preferSteam": false } },
  "nowplaying": { "enabled": true,  "order": 1, "size": "medium", "prefs": {} },
  "plex":       { "enabled": true,  "order": 2, "size": "medium", "prefs": {} },
  "recent":     { "enabled": true,  "order": 3, "size": "medium", "prefs": {} }
}
```

`SettingsStore` exposes generic accessors — `Config.widget(id)` for reads,
`Config.setWidget(id, key, value)` for writes — instead of one setter per widget. `Theme`'s
per-widget passthroughs are deleted; widgets read their own injected `config`. The daemon's
`get-config` / `set-config` / `subscribe` / `config:changed` IPC is unchanged (it already
treats the document as opaque).

**Flavors are not a subsystem.** A "flavor" is just **which widgets a user enables** — the
`widgets.<id>.enabled` config namespace *is* the flavor. One user runs Plex + Spotify;
another runs Jellyfin + Spotify. That's basic per-service config, nothing more — no profile
resolution layer, no `flavors/*.toml`, no named-profile machinery. The framework registers
every available widget; config turns the ones you want on. The only "seed" needed is a
default `settings.json` (or absent-key defaults from each manifest) so a fresh install isn't
blank. `game-client-1`'s setup is simply its `settings.json`.

## 8. Sidecar backend contract (QML + sidecar)

Per the chosen extension surface, a widget needing real backend logic ships a **sidecar
process**, not a daemon recompile. The precedent already exists: `game-shell-host` is a
sidecar the daemon talks to over the typed `game-shell-protocol` crate (`protocol/src/lib.rs`:
`LibraryEntry` / `LibraryResponse` / `LaunchRequest`) for the Steam library.

Generalize that into a **widget-backend contract**:
- A widget declares `"requires": ["<id>-sidecar"]` and ships a backend binary +
  `backend.toml` (how to launch it, health check).
- The daemon supervises the sidecar (spawn, health, restart) and exposes its responses to
  QML through the existing capability/event channels — QML never speaks to the sidecar
  directly, only through the daemon, preserving the clean seam.
- The protocol is typed serde over the daemon↔sidecar socket (reuse the `game-shell-protocol`
  pattern; each sidecar gets its own message types or a shared envelope).
- **Core** widgets (Plex, Moonlight) may keep their in-daemon module (`plex.rs`,
  `moonlight.rs`) — blessed/first-party only. Third-party widgets use the sidecar path.

**Decision: one shared supervisor + a common envelope.** The daemon owns a single
sidecar-lifecycle path (spawn / health / restart) and a shared message envelope; each
sidecar declares its own typed payloads inside it. New backends reuse the plumbing instead of
re-solving lifecycle + protocol the way `game-shell-host` does bespoke today. `game-shell-host`
is migrated onto this shared supervisor as the reference implementation.

## 9. The Widgets page (top-level, schema-driven)

- **Promote Widgets to a top-level destination** (peer of Home / Library / Settings), not a
  page buried in the Settings sidebar. The current `WidgetsSettings.qml` afterthought is
  removed.
- The page is **generated from manifests**: it lists every registered widget, shows
  enabled/disabled + order + size + the manifest's `config` schema rendered as generic
  controls (toggle, dropdown, etc.), plus capability status ("Plex backend unavailable").
  No per-widget page code.
- **A plasma-bigscreen-style reorder list is the order UI.** The page includes an in-shell,
  fully controller-navigable "arrange widgets" list (move up / move down on a focused row),
  modeled on Plasma Bigscreen's reordering — so the order is set on the HTPC itself, not by
  hand-editing config. Reordering writes `widgets.<id>.order`; the manifest's `defaultOrder`
  only seeds first run. Per-widget version + changelog link surfaces here too (ties to §10).
- The MCP `SettingsPage`/`open_settings` surface adds a generated `widgets` entry from the
  registry instead of the hand-maintained enum.

## 10. Versioning & release

**Extend the existing independent-tag scheme.** The repo already ships
`release-input.yml` (`input-v*`, the daemon) and `release-host.yml` (`host-v*`, the sidecar),
explicitly "independently versioned within the one monorepo." Add a third axis:

- Each `widget.json` carries `version`. A `release-widget.yml` workflow detects which
  `shell/widgets/<id>/` directories changed in a merged PR, and for each cuts an immutable
  `widget-<id>-vX.Y.Z` tag with AI-generated, **per-widget** release notes (reuse
  `jedwards1230/release-workflows` `ai-release.yml@v1`, diff scoped to that widget's dir).
- A top-level `widgets-index.json` (marketplace-shaped, mirroring the `claude-plugins`
  `marketplace.json` pattern) aggregates `{id, version, minFrameworkVersion}` — the public
  "what widgets exist + at what version" index, and what a future runtime loader would read.
- Per the repo's release conventions: the git tag is authoritative; if a manifest version
  must be bumped, the bump rides the tagged commit, not pushed back to `main`. Opt-in via
  `semver:*` labels stays the trigger.

This gives independent **version tracking + changelogs** in one repo (the chosen model)
without standing up separate distribution machinery.

## 11. Security posture (public repo)

State this plainly in the README so expectations are set:
- QML widgets run **in the shell process** with full access to singletons and the daemon
  socket. There is **no sandbox**. Widgets are trusted code, exactly as a Quickshell config
  is trusted.
- Distribution is **source + review**, not a sandboxed runtime. First-party widgets are
  "core"; community widgets are reviewed PRs. The `requires`/capability gate limits what a
  widget can *reach*, but a malicious widget could still misbehave — so trust is by review.
- Sidecars are separate processes the daemon supervises; they inherit the user's privileges.
  Same trust model. The token-by-reference rule (`config.toml` points at mode-0600 token
  files) already in place for HTTP/Plex/Steam extends to widget sidecars.

## 12. Migration phasing

Each phase is independently shippable and leaves the shell working:

1. **`Widget` base class.** Introduce `shell/widgets/lib/Widget.qml`; refactor the four
   existing widgets (and `MprisPlayerBase`) to extend it. No behavior change, no config
   change. Pure consolidation of the duck-typed contract.
2. **`WidgetHost` + registry.** Add the codegen + `WidgetRegistry`; move `HomeScreen`'s
   static instantiation + hardcoded `_contentRegions()` to the host's generic focus chain.
   Still the same four widgets, same flat config — just no longer hand-wired.
3. **Manifest + namespaced config + Widgets page.** Add `widget.json` per widget; migrate
   `settings.json` to the `widgets.<id>.*` subtree (one-time migrator in `config.rs`); delete
   `Theme` passthroughs and `WidgetsSettings.qml`; ship the top-level schema-driven Widgets
   page. Generate the MCP page list from manifests.
4. **Per-widget versioning.** Add `release-widget.yml` + `widgets-index.json`; backfill
   initial `version`s and per-widget changelogs.
5. **Sidecar contract generalization.** Factor the `game-shell-host` pattern into a reusable
   widget-backend supervisor + protocol so a new widget can ship a backend.
6. *(Later, optional)* Runtime out-of-tree discovery; community widget distribution.

Phases 1–2 are the high-value, low-risk core (they kill the "fragile hand-wired focus chain"
and the 4-file-edit problem). Phase 3 is the visible payoff (real Widgets page). 4–5 unlock
the public-framework story.

## 13. Resolved decisions

| # | Question | Decision |
|---|---|---|
| 1 | Config schema breadth | **Minimal** — `bool \| enum \| int \| string` only. Extend the vocabulary on demand when a real widget needs more. |
| 2 | Capability discovery | **Daemon advertises a `capabilities` query.** The host gates each widget's manifest `requires` against it at runtime and greys + explains an unavailable widget (e.g. no Plex server). Supports per-host/flavor differences cleanly. |
| 3 | Sidecar supervision | **One shared supervisor + common envelope** (§8); typed per-sidecar payloads. `game-shell-host` migrates onto it as the reference. |
| 4 | "Flavor" machinery | **None.** A flavor is just the set of widgets a user enables in config (`widgets.<id>.enabled`). Basic per-service config, not a subsystem (§7). |
| 5 | Widget order | **In-shell plasma-bigscreen-style reorder list** on the Widgets page (controller-navigable, set on the HTPC). Persists `widgets.<id>.order`; manifest `defaultOrder` seeds first run only (§9). |
| 6 | Project name | **`qs-bigscreen`** (Quickshell-anchored, mirrors `plasma-bigscreen`). Keep this repo as *the framework*; `game-shell` is retired. GitHub name confirmed free. Rename is its own scoped change — see §14. |

### Still genuinely open (smaller, decide during implementation)

- **Capability vocabulary contents** — the canonical list of capability strings
  (`network`, `mpris`, `plex-backend`, `moonlight-backend`, `steam-sidecar`, …) and exact
  `capabilities`-query wire shape. Settled when phase 5 lands.
- **`order` seeding** — whether `defaultOrder` is a required manifest field or optional with
  registration-order fallback.

## 14. Naming — rename to `qs-bigscreen`

**Decided: `qs-bigscreen`.** Quickshell-anchored (the widgets are Quickshell QML; Hyprland is
just the compositor), and a deliberate echo of `plasma-bigscreen` — it reads instantly as
"the Quickshell equivalent of Plasma Bigscreen." `game-shell` is retired. The repo stays *the
framework* (not extracted into a separate repo, not demoted to a flavor). GitHub
`jedwards1230/qs-bigscreen` confirmed free.

The rename is mechanical but **wide**, so scope it as its own change *after* (or alongside,
but separate from) the framework build. Surfaces to touch:

- **This repo:** rename `jedwards1230/game-shell` → `qs-bigscreen`; Quickshell config dir +
  symlink (`~/.config/quickshell/game-shell` → `qs-bigscreen`); install root and
  `GAME_SHELL_*` env vars; the `game-shell-session` script + Wayland session files.
- **Daemon/binary:** the binary is `game-shell-input` today; decide whether it becomes
  `qs-bigscreen-input` (and whether release tags stay `input-v*` or move to a namespaced
  prefix). Tags `host-v*` / `widget-<id>-v*` likewise.
- **Homelab orchestration:** ansible `game_client_*` vars + `game_client_common` role tags,
  `scripts/repos.conf` path, the `game-shell-dev` plugin + skill + predefined MCP server id
  (`:8090/mcp`), and `services-inventory` / wiki references.
- **Docs:** in-repo docs, the wiki project page (`home/projects/game-client-shell`).

> Note: `game-client-1` (the *host*) keeps its name — only the *shell/framework* is renamed.
> The HTPC is a machine; `qs-bigscreen` is the software it runs.

## 15. File inventory — what moves where

| Concern | Today | After |
|---|---|---|
| Widget contract | duck-typed comment in `HomeScreen.qml` | `shell/widgets/lib/Widget.qml` |
| Instantiation + focus | hardcoded in `HomeScreen._contentRegions()` | `shell/widgets/lib/WidgetHost.qml` + generated registry |
| Widget identity/version/schema | `_widgets` catalog + `SettingsStore` props + `mcp.rs` enum | `shell/widgets/<id>/widget.json` (SSOT) |
| Config | flat `widget<Name>*` in `SettingsStore`, `Theme` passthroughs | `widgets.<id>.*` subtree + generic `Config.widget(id)` |
| Settings UI | `shell/settings/WidgetsSettings.qml` (afterthought) | top-level schema-driven Widgets page |
| Backend (heavy) | in-daemon module (`plex.rs`) / `game-shell-host` sidecar | core: keep module; community: sidecar contract |
| Release | `input-v*`, `host-v*` | + `widget-<id>-v*` + `widgets-index.json` |
```
