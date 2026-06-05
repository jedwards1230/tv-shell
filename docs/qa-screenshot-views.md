# QA Screenshot Views

A living catalog of game-shell views/overlays/states worth capturing in a visual-QA
screenshot batch. Keep this updated as views are added or changed.

## How to capture

See the `game-shell-dev` skill ("Driving the UI for Screenshots"). In short there
are **two CLI channels** (see [IPC_PROTOCOL.md](IPC_PROTOCOL.md)):
- **Directional nav / select / back = real key events.** Either `wtype -k <Left|Right|Up|Down|Return|Escape>` (Wayland virtual keyboard) **or** the daemon's `key <name>` IPC (`key up|down|left|right|select|back` over the socket). Both reach the focused surface's `KeyNavigation`. There is **no `Tab` drawer key** — `wtype -k Tab` does nothing useful.
- **Drawer / settings / power / home = the `intent` control surface** (socket): `echo "intent menu" | nc -U /run/user/1000/game-shell-input.sock` toggles the **left nav drawer**; `intent settings` / `intent power` / `intent home` open those. (At the TV the drawer also opens via gamepad **Home** tap or a bare **Super** press — Hyprland bind → `super-intent.sh` → `intent menu`. Super+Escape = escape; Super+Backspace = reset.) Deep-link targets also use this surface: `intent settings:<page>` opens a specific settings page in one command (e.g. `intent settings:bluetooth`); `intent overlay:volume` / `intent overlay:network` open the respective QAM popover; `intent app:<wmClass>` launches a local app by its StartupWMClass.
- Screenshots are 4K (~2000 tokens each) — shoot in **tiers** (below), not all at once.

## Home screen index map (QuickActions, top-right)

`0=Notifications, 1=Settings, 2=Theme toggle, 3=Network, 4=Volume, 5=Power`.
Left/Right move; Return activates; Down drops focus into the app rows. **Focus does
not always start on this row** — with running/recent apps present it starts on an
app card, so press **Up** first to reach the QuickActions row before Left/Right.
`streamingViewMode` ∈ `"servers"` | `"apps"` toggles two different home layouts.

---

## A. Home screen — states & rows
| # | View | How to reach | Notes |
|---|------|--------------|-------|
| A1 | Home, full (idle) | default after restart | hero clock/date + QuickActions + rows |
| A2 | Running row | apps running | `runningWindows.length > 0` |
| A3 | Recent row | after launching apps | `RecentsTracker.recentApps` |
| A4 | Moonlight row — "servers" view | `streamingViewMode = servers` | server cards |
| A5 | App rows — "apps" view | `streamingViewMode = apps` | per-host "Moonlight — \<host\>" rows |
| A6 | Applications row | local launchers present | `appsRow` |
| A7 | Empty states | no running / recents / targets | verify layout holds when rows hide |
| A8 | App-view substates | apps view, host discovering/offline | "Discovering apps…" / "Offline or no apps found" |
| A9 | Long-name marquee | card with long title | `MarqueeText` scroll |

## B. Context menus / popovers
| # | View | How to reach |
|---|------|--------------|
| B10 | App card context menu (`PopoverMenu`) | focus an app card → context key (Focus / Close) |
| B11 | Stream card context menu | focus a stream card → context key (Resume / Quit) |

## C. Overlays & dialogs
| # | View | How to reach | Capturability |
|---|------|--------------|---------------|
| C12 | Left nav drawer (`NavigationDrawer`) | `intent menu` (socket) — or gamepad Home / bare Super at the TV | socket-reachable (NOT `wtype` — no Tab handler); drawer now shows an HDR/display status glance (HDR on/off + resolution/refresh) near the bottom, sourced via `hypr-monitors` IPC |
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
| D21–31 | Pages: Audio, Bluetooth, Network, Display, Controllers, Key Bindings, AV Control, Moonlight, Appearance, Accessibility, Power | Down/Up move the sidebar **cursor only** — the content pane does **not** follow it. Press **Return** to load the focused page (focus stays on the sidebar). `Right` then enters the *loaded* page's controls; it does **not** switch pages. So per page: Down/Up → **Return** → screenshot. Each page is also directly reachable via `intent settings:<id>` (socket) — id slugs: `audio`, `bluetooth`, `network`, `display`, `controllers`, `keybindings`, `avcontrol`, `appearance`, `accessibility`, `power` (plus `streaming` or provider id when active). Display page (#127): now reads monitors via daemon `hypr-monitors` IPC (replaces `hyprctl monitors -j` shell-out); shows live HDR status (read-only, driven by daemon `hdr` field), HDR toggle (persists + applies via `hyprctl keyword monitor` with/without `bitdepth,10,cm,hdr` suffix), separate Refresh Rate dropdown (filters `availableModes` to current resolution), Night Light toggle + color-temperature dropdown (applies via `hyprsunset`, requires hyprsunset), Overscan stepper (persists safe-area pct). |
| — | Bluetooth — scanning + device list | substate |
| — | Network — Wi-Fi list / connect | substate |
| — | Controllers — pad connected vs none | substate |
| — | Key Bindings — capture mode ("press a button") | substate |
| — | AV Control — CEC device info populated | substate — reads `cec-scan` JSON from daemon + subscribes to `cec:device:*`/`cec:power:*` events (#16) |
| — | Moonlight — add/edit server form + servers/apps toggle | substate |
| — | Appearance — each theme mode selected (auto/light/dark) | substate |
| — | Appearance — live external reload | QA: edit `~/.config/game-shell/settings.json` over SSH (e.g. flip `themeMode` `dark`→`light`) while the shell is open; confirm the theme switches without a Quickshell restart. The daemon broadcasts `config:changed` and `SettingsStore` re-fetches via `get-config`. No new screenshot view — the existing Appearance substates cover the visual. |
| — | Accessibility — Reduce Motion on/off; Text Size Default/Large/Larger | substate |
| — | Power — sleep-timer cycle (Off/5/10/15/30/60 min), wake-on-controller toggle (On/Off), End session button reachable via `intent settings:power` | substate |

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
- **Drawer (C12):** `intent menu` over the socket (or gamepad Home / bare Super at the TV) — **not** `wtype` (no Tab handler).
- **Needs a real condition:** C15 (toast timing), C17 (stream conflict), C18 (stream overlay), G (live stream/app).

## Suggested tiered batch
1. **Tier 1 — static views, dark mode:** A1–A9, B10–B11, D20–D30 + settings substates, C13/C14/C16/C19.
2. **Tier 2 — light mode** re-shoot of the core set (E).
3. **Tier 3 — input-mode** variants (F) where visually distinct.
4. **Tier 4 — manual/condition:** drawer (C12, `intent menu` over the socket or a TV press), then condition-dependent (C15/C17/C18/G).
