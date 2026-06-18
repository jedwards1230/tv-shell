# QA Screenshot Views

A living catalog of game-shell views/overlays/states worth capturing in a visual-QA
screenshot batch. Keep this updated as views are added or changed.

## How to capture

See the `game-shell-dev` skill ("Driving the UI for Screenshots"). In short there
are **two CLI channels** (see [IPC_PROTOCOL.md](IPC_PROTOCOL.md)):
- **Directional nav / select / back = real key events.** Either `wtype -k <Left|Right|Up|Down|Return|Escape>` (Wayland virtual keyboard) **or** the daemon's `key <name>` IPC (`key up|down|left|right|select|back` over the socket). Both reach the focused surface's `KeyNavigation`. **`Tab` toggles the nav drawer** when the shell window holds Wayland focus (K400 on the couch, `ShellLayout.Keys.onTabPressed → toggleMenu()`, idle only) — but `wtype -k Tab` from an external session may not reach the shell window if focus is elsewhere; prefer `intent menu` over the socket for reliable automation.
- **Drawer / settings / power / home = the `intent` control surface** (socket): `echo "intent menu" | nc -U /run/user/1000/game-shell-input.sock` toggles the **left nav drawer**; `intent settings` / `intent power` / `intent home` open those. (At the TV the drawer also opens via gamepad **Home** tap or a bare **Super** press — Hyprland bind → `super-intent.sh` → `intent menu`. Super+Escape = escape; Super+Backspace = reset.) Deep-link targets also use this surface: `intent settings:<page>` opens a specific settings page in one command (e.g. `intent settings:bluetooth`); `intent overlay:volume` / `intent overlay:network` open the respective QAM popover; `intent app:<wmClass>` launches a local app by its StartupWMClass.
- Screenshots are 4K (~2000 tokens each) — shoot in **tiers** (below), not all at once.

## Home screen index map (QuickActions, top-right)

`0=Notifications, 1=Settings, 2=Theme toggle, 3=Network, 4=Volume, 5=Power`.
Left/Right move; Return activates; Down drops focus into the content regions below.
**Focus does not always start on this row** — with Continue/New content present it
starts on a card, so press **Up** first to reach the QuickActions row before
Left/Right.

**B (Back / Escape) on the home screen** resets focus to the default landing
position (top content row, first card). If already at the default position it is a
quiet no-op. B does **not** open Settings — use QuickActions idx 1 (→ Return) or
`intent settings` (socket) to reach Settings.

---

## A. Home screen — states & rows
| # | View | How to reach | Notes |
|---|------|--------------|-------|
| A1 | Home, full (idle) | default after restart | hero clock/date + QuickActions, then the enabled standardized widgets: Now Playing, Plex (On Deck + Recently Added), Recent apps, All Apps entry |
| A2 | Now Playing — medium | MPRIS active, size = medium (default) | `MediaWidget` card: cover art + progress bar + transport row |
| A3 | Now Playing — small | MPRIS active, size = small (Settings ▸ Widgets) | `NowPlayingStrip` slim strip; both sizes collapse when nothing plays |
| A4 | Plex Recently Added — dynamic chips | Plex healthy, ≥2 media categories present | `FilterChips` show All + only categories present (no Music pill on a music-less library); re-filter live on item `kind` |
| A4b | Widgets settings — list + drill-in | Settings ▸ Widgets | list of widget rows (Moonlight/Now Playing/Plex/Recent), each one focus stop: **A** toggles enable in place, **X**/**Right** drills into that widget's config sub-page (Size, Hide-from-Recent, and — Moonlight only — Manage servers ▸ the embedded server list). **B** steps back one level (sub-page → list → sidebar). |
| A5 | All Apps entry | always present | single tile; A → opens the Library surface (A12) |
| A6 | Empty states | no running/recents, Plex empty/off | Continue + New rails collapse; B still lands on the All Apps entry (never strands) |
| A9 | Long-name marquee | card with long title | `MarqueeText` scroll |
| A10 | Controller battery glyph | wireless pad connected reporting charge | 🔋+% beside QuickActions; ⚡ when charging; crimson ≤15%; hidden when only wired pads or none (#100) |
| A11 | Plex widget — On Deck + Recently Added | `GAME_SHELL_PLEX_*` env set and Settings ▸ Widgets ▸ Plex on | two poster rows (`PlexWidget`), size-scaled (small/medium); On Deck shows a resume bar; Recently Added carries the dynamic chips (A4) |
| A11b | Plex server-down notice | as A11 but Plex unreachable (down / 5xx) | inline `ServiceStatusNotice`: "Plex unavailable" — both Plex rails collapse, focus chain still walks (service-health bus) |

## A12. Library — secondary browse surface
| # | View | How to reach | Notes |
|---|------|--------------|-------|
| A12 | Library — full | home "All Apps" entry → A | "Library" header + Moonlight section + Applications grid; B returns to Home with focus restored |
| A12a | Moonlight — servers | Library | server cards (`StreamCard`); servers-only (the apps-view toggle was removed) |
| A12c | Applications row | local launchers present | full `AppDiscoveryManager.applications` list |

## B. Context menus / popovers
| # | View | How to reach |
|---|------|--------------|
| B10 | App card context menu (`PopoverMenu`) | focus an app card → context key (Focus / Close) |
| B11 | Stream card context menu | focus a stream card → context key (Resume / Quit) |

## C. Overlays & dialogs
| # | View | How to reach | Capturability |
|---|------|--------------|---------------|
| C12 | Left nav drawer (`NavigationDrawer`) | `intent menu` (socket) — or gamepad Home / bare Super at the TV — or `Tab` when the shell window holds Wayland keyboard focus | socket-reachable; `wtype -k Tab` works only when shell window has focus (unreliable in automation) |
| C13 | Notification center | QuickActions idx 0 → Return | wtype |
| C14 | Notification center — empty | as above, no notifications | wtype |
| C15 | Notification toast (`NotificationToast`) | trigger a notification | transient; timing-sensitive |
| C16 | Power overlay (`PowerOverlay`) | QuickActions idx 5 → Return | wtype |
| C17 | Session conflict dialog (`SessionDialog`) | real stream conflict | needs live conflict / mock |
| C18 | Stream overlay (`StreamOverlay`) | launching / reconnecting / error | needs active/failing stream |
| C19 | Error log viewer (`ErrorLogViewer`) | notification center → error log | wtype |
| C20 | Volume QAM popover (`VolumeOverlay`) | home QuickActions idx 4 → Return; also reachable from the nav drawer; also `intent overlay:volume` (socket) | wtype |
| C21 | Network QAM popover (`NetworkOverlay`) | home QuickActions idx 3 → Return; also reachable from the nav drawer; also `intent overlay:network` (socket) | wtype |

## D. Settings panel + pages + substates
| # | View | How to reach / notes |
|---|------|----------------------|
| D20 | Settings sidebar (panel open) | QuickActions idx 1 → Return |
| D21–31 | Pages: Audio, Bluetooth, Network, Display (+Appearance/theme), Controllers, Key Bindings, AV Control, Widgets, Accessibility, Power, System (+Storage). **Moonlight server management is no longer a sidebar page** — it's demoted under Widgets ▸ Moonlight ▸ Manage servers; `intent settings:moonlight` / `settings:streaming` reroute there. | Down/Up move the sidebar **cursor only** — the content pane does **not** follow it. Press **Return** to load the focused page (focus stays on the sidebar). `Right` then enters the *loaded* page's controls; it does **not** switch pages. So per page: Down/Up → **Return** → screenshot. Each page is also directly reachable via `intent settings:<id>` (socket) — id slugs: `audio`, `bluetooth`, `network`, `display`, `controllers`, `keybindings`, `avcontrol`, `accessibility`, `power`, `system`. `streaming`/`moonlight` reroute to Widgets ▸ Moonlight ▸ Manage servers (no longer a sidebar page). Theme mode selector (auto/light/dark) is part of the **Display** page. Free-space storage readout is part of the **System** page. Display page (#127): now reads monitors via daemon `hypr-monitors` IPC (replaces `hyprctl monitors -j` shell-out); shows live HDR status (read-only, driven by daemon `hdr` field), HDR toggle (persists + applies via `hyprctl keyword monitor` with/without `bitdepth,10,cm,hdr` suffix), separate Refresh Rate dropdown (filters `availableModes` to current resolution), Night Light toggle + color-temperature dropdown (applies via `hyprsunset`, requires hyprsunset), Overscan stepper (persists safe-area pct). |
| — | Bluetooth — scanning + device list | substate |
| — | Network — Wi-Fi list / connect | substate |
| — | Network — gateway/DNS card + test-connection result | substate (net-status now carries `gateway`, `dns`, and per-connection `speed`; page shows a Gateway/DNS read-only card and a Test-connection action with OK/Failed inline result) |
| — | Controllers — pad connected vs none | substate |
| — | Key Bindings — capture mode ("press a button") | substate |
| — | AV Control — CEC device info populated | substate — reads `cec-scan` JSON from daemon + subscribes to `cec:device:*`/`cec:power:*` events (#16) |
| — | AV Control — Focus preference toggles | always-visible "Focus Preferences" section: "Focus TV on startup" (default Off) and "Focus TV on wake from sleep" (default On); render correctly even when CEC is unavailable |
| — | Moonlight — add/edit server form | substate, reached via Widgets ▸ Moonlight ▸ Manage servers ▸ Add Server |
| — | Display — each theme mode selected (auto/light/dark) | substate in Display page Appearance section |
| — | Display — live external reload | QA: edit `~/.config/game-shell/settings.json` over SSH (e.g. flip `themeMode` `dark`→`light`) while the shell is open; confirm the theme switches without a Quickshell restart. The daemon broadcasts `config:changed` and `SettingsStore` re-fetches via `get-config`. No new screenshot view — the existing theme substates cover the visual. |
| — | Accessibility — Reduce Motion on/off; Text Size Default/Large/Larger | substate |
| — | Audio — default-sink persistence (by node.name, re-applied on boot), 5.1 speaker-test buttons (FL/FR/Center/LFE/RL/RR + All channels), sample-rate/format read-out | substate |
| — | Power — sleep-timer cycle (Off/5/10/15/30/60 min), wake-on-controller toggle (On/Off), End session button reachable via `intent settings:power` | substate — the auto-suspend idle timer lives at the shell root and fires regardless of which settings page is open |

> **#141**: All list-bearing settings pages (Network ×2, Bluetooth ×2, Moonlight, Display, Controllers) now share `SettingsList` for row-count sizing — the floating-gap regression class (#123/#139) is centralized. QA: verify lists pack directly under their headers with no gap in both dark and light mode. The Display page still uses `SettingsList` for the monitor list (#127 did not change that).

## E. Theme variants (multiplier)
Capture at least **home, a settings page, notification center, power overlay** in
both **light** and **dark** mode (toggle via QuickActions idx 2). Full rigor = every
view ×2.

## F. Input-mode variants
Same view in **controller mode** (crimson focus borders) and **mouse mode** (hover
highlights + cursor). Relevant to the #45 mouse-mode work.

## G. Transient / condition-dependent (flag, don't block a batch)
Launching state, streaming (LIVE badge), `appRunning` overlay drawer — only
capturable with a live stream/app.

---

## Capturability summary
- **Key-driven (nav/select/back via `wtype -k` or `key <name>`):** A1–A9, B10–B11, C13–C14, C16, C19, E, F; D20 and (once open) the D21–D31 page controls.
- **Socket deep-link (`intent` command):** D21–D31 settings pages (`intent settings:<page>`) and C20/C21 overlays (`intent overlay:volume` / `intent overlay:network`) are directly socket-reachable in one command without navigating through the sidebar or QuickActions.
- **Drawer (C12):** `intent menu` over the socket (or gamepad Home / bare Super at the TV) is the reliable path; `Tab` also works when the shell window holds Wayland keyboard focus, but is unreliable from an external automation session.
- **Needs a real condition:** C15 (toast timing), C17 (stream conflict), C18 (stream overlay), G (live stream/app).

## Suggested tiered batch
1. **Tier 1 — static views, dark mode:** A1–A9, B10–B11, D20–D31 + settings substates, C13/C14/C16/C19.
2. **Tier 2 — light mode** re-shoot of the core set (E).
3. **Tier 3 — input-mode** variants (F) where visually distinct.
4. **Tier 4 — manual/condition:** drawer (C12, `intent menu` over the socket or a TV press; or `Tab` with direct keyboard focus), then condition-dependent (C15/C17/C18/G).
