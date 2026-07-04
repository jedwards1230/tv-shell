# Kiosk Window Model

How game-shell guarantees the kiosk contract on Hyprland, why the previous
reactive approach kept failing, and how the current implementation makes it
structural.

**The contract.** Exactly one app window is visible and fills the screen; the
shell (Quickshell) sits deterministically above/below it; backgrounded apps
(Plex HTPC, Steam) keep running but never share the screen. Two app windows must
never be visible at once.

**Assumptions.** Hyprland ~0.55 (config targets 0.55). The Quickshell shell is a
**layer-shell** surface (`WlrLayershell`, holds `WlrKeyboardFocus.Exclusive`
while mapped) — not a tiled toplevel, so it is unaffected by window rules and
workspace switches and is never a party to the tiling layout. App windows are
ordinary xdg toplevels the tiler owns. Every option below is verified against the
Hyprland source (file references inline); anything that did not survive that
check is listed under *Rejected*.

## Why the old reactive model structurally could not win

Everything ran on **one workspace**, and "one app fills the screen" was
maintained by *reacting* to Hyprland's event stream after the tiler had already
laid windows out, from **two independent enforcers**:

1. **Daemon** (`daemon/src/hyprland.rs`): `force_fullscreen` on `openwindow`;
   `enforce_active_fullscreen` (idempotent `fullscreen 0 set`) on
   `closewindow`/`movewindowv2`/`activewindowv2`.
2. **QML** (`AppLifecycleManager.qml`): on launch/resume, `focuswindow` then
   re-assert fullscreen — the resume address-path fired `hyprctl dispatch
   fullscreen 0`, a **non-idempotent toggle**, unconditionally.

The bad state (two app windows tiled side by side) was **representable** because
the single shared workspace lets the tiler split whenever two toplevels coexist
and neither is fullscreen. That gives three structural defects: two enforcers
with no serialization and conflicting semantics (idempotent `set` vs
non-idempotent toggle); a toggle is state-dependent while the layout is shared
and mutable; and once both windows are tiled, no declarative rule re-fullscreens
on focus — only the racy actor could.

**Today's incidents, mapped:**
1. *Split view on Steam resume (Plex backgrounded).* Resuming an already-
   fullscreen app ran `focuswindow` (Hyprland, via `on_focus_under_fullscreen=1`,
   correctly kept it fullscreen) → then QML's unconditional `fullscreen 0` toggle
   flipped it back **out** → both apps tiled → tiler split. Reproduces even with
   a healthy daemon; killing Plex "fixed" it because close-path enforcement then
   ran with a single window and no racing partner.
2. *Daemon deaf to a restarted Hyprland (>1h).* `resolve_hypr_signature()`
   trusted an inherited `HYPRLAND_INSTANCE_SIGNATURE` without a liveness check, so
   a long-lived `systemd --user` daemon stayed pinned to the dead instance after a
   render-hang kill+restart; reconnect backoff re-resolved the same stale
   signature forever. Silent — nothing surfaced it.
3. *Presenter flapped Shell↔Game* during the incident-1 focus churn — a symptom
   of reacting to a racing event stream.
4. *HDMI/CEC flap wedges Hyprland's render loop* (hyprctl still answers, frames
   frozen); kill+restart is the only recovery, which then triggers incident 2.

## The fix: declarative-first, single idempotent backstop, self-healing daemon

Remove the tiler's freedom to reach the bad state, collapse enforcement to one
idempotent authority, and make the daemon's compositor attachment self-heal.

**Declarative kiosk rules** (`config/hyprland.conf`, all verified against source):
- `windowrule = fullscreen on, match:class .+` — force every app window
  fullscreen at map (best-effort; gated on winning initial focus).
- `windowrule = suppress_event fullscreen maximize, match:class .+` — the kiosk
  owns fullscreen; ignore a window's own fullscreen/maximize *requests* so a game
  toggling fullscreen can't churn compositor state. `suppress_event` blocks only
  the window's requests, not `hyprctl dispatch`, so the daemon/keybinds keep
  control (`src/desktop/view/Window.cpp`).
- `misc:on_focus_under_fullscreen = 1` — focusing a window under a fullscreen one
  atomically un-fullscreens the old and fullscreens the new; the correct resume
  swap (`src/desktop/state/FocusState.cpp`).
- `misc:exit_window_retains_fullscreen = true` — closing a fullscreen app
  promotes the survivor to fullscreen natively; the declarative form of the
  daemon's old close-path enforcement (`src/config/values/ConfigValues.cpp`).

**Launch-time atomic placement** (`AppLifecycleManager.qml`): the app launch
dispatches `hyprctl dispatch exec [fullscreen] <cmd>`, so the app's first window
maps fullscreen from the start with nothing to correct post-hoc (exec-rule
syntax: `src/config/supplementary/executor/Executor.cpp`).

**Single enforcer** (`AppLifecycleManager.qml`): the QML `fullscreen 0` toggle
and its `hypr-active` read (`ensureFullscreen`/`ensureFullscreenQuery`) are
**removed**. `on_focus_under_fullscreen=1` + the daemon's idempotent `fullscreen
0 set` are now the only things that ever change fullscreen — the incident-1 root
cause is gone.

**Self-healing daemon** (`daemon/src/session_env.rs`, `hyprland.rs`): signature
resolution scans `$XDG_RUNTIME_DIR/hypr/` for the live socket dir *before*
trusting an inherited env var, so a reconnect re-attaches to a restarted Hyprland
(kills incident 2). Connect-time logging names the attached instance; five
consecutive failed reconnects escalate to a loud `error!` naming the deaf-daemon
condition.

Together these make the contract hold **by construction on a single workspace**:
every window is fullscreen at map and can't un-fullscreen itself; focus swaps are
atomic; closes promote the survivor; and the one actor that broke it is gone. No
frame ever shows two tiled app windows.

## Evaluation of the proposed lockdown ideas

| Idea | Verdict | Notes |
|---|---|---|
| Launch-time atomic `[fullscreen]` placement | **Adopted** | Highest-leverage; app is fullscreen at map. Launching is QML (`hyprctl dispatch exec`), not the daemon — `intent app:` just routes to QML. |
| `suppress_event fullscreen maximize` | **Adopted** | Effect name is `suppress_event` (underscored) in 0.55.2's `WindowRuleEffectContainer.cpp` EFFECT_STRINGS; the legacy `windowrulev2` spelling `suppressevent` fails config parse on-device. Tokens (`fullscreen`/`maximize`) verified in `Window.cpp`. `activate` deliberately **not** suppressed — `focus_on_activate`/launch-focus rely on it; focus-steal is prevented structurally by the fullscreen invariant instead. |
| `new_window_takes_over_fullscreen = 2` | **Rejected** | **Does not exist** in Hyprland 0.55 (`ConfigValues.cpp` has no such key) — setting it errors in `hyprctl configerrors`. Its intent is covered by the fullscreen windowrule + `on_focus_under_fullscreen`; adopted `exit_window_retains_fullscreen` (a real option) instead. |
| Dynamic per-class `windowrulev2` from the daemon | **Rejected (obviated)** | The wildcard `match:class .+` already applies to every class; runtime per-class registration adds churn for no coverage gain. |
| Strip default Hyprland binds | **N/A (already satisfied)** | Hyprland ships **no** default keybinds; the kiosk config already declares only the super-intent set + `SUPER,Q`. Nothing to strip. |
| `idleinhibit fullscreen` windowrule | **Rejected** | Every app is always fullscreen, so this would inhibit idle for *any* running app and defeat the shell's configurable sleep timer (Power page). Media players already send the Wayland idle-inhibit protocol when actually playing — the nuance a blanket rule loses. |
| Compositor watchdog in the daemon | **Partial (Phase 1) / Phase 2** | The event-socket-dead case (incident 2) is now detected + escalated. The render-wedge (incident 4 — frozen frames while `hyprctl` still answers) is **not** IPC-observable from the daemon; detecting it needs a render-side heartbeat (Phase 2). Auto-heal (kill Hyprland → restart plasmalogin → restart daemon) is Phase 2. |
| "Running apps" list in the NavigationDrawer | **Phase 2 (UI, not implemented)** | The daemon window model already exposes everything it needs — each running window's `address`, `class`, `workspace`, `focusHistoryId` via `hypr-clients`. So `A` → `focusByAddress` (today) or `dispatch workspace N` (under Phase-2 isolation) is a race-free switch. Phase 1 does not paint it into a corner. |

## Interaction with the Steam widget (PR #306)

#306's resume path (`SteamCard`/`MoonlightWidget` → `focusByAddress` →
`appLaunched`) is **unchanged in shape** and **improves** under this work: the
`fullscreen 0` toggle that `focusByAddress` used to trigger is gone, so resuming
Steam while Plex is backgrounded swaps fullscreen atomically instead of splitting.
No API change — `focusByAddress(address)` still focuses the window and emits
`appLaunched`. **Under a future Phase 2 (per-app workspaces)** resume becomes
`dispatch workspace N` (because `dispatch focuswindow` does not reliably follow
across workspaces — [hyprwm/Hyprland#1611](https://github.com/hyprwm/Hyprland/issues/1611));
#306 would then read the target window's `workspace` (already in the
`hypr-clients` model) rather than call `focuswindow`. Nothing in #306 needs to
change for the current phase.

## Phase 2 (deferred — the strongest form, needs on-device iteration)

**Per-app-workspace isolation.** Assign each app window its own workspace
(class-grouped, so Steam's splash + main share one) so two app windows can never
occupy the same workspace — the split state becomes *unrepresentable* rather than
merely *prevented*. Launch: `exec [workspace N silent; fullscreen]`. Resume:
`dispatch workspace N` (not `focuswindow`, per #1611). This is a bigger change
(workspace allocation, resume-path rework, Steam multi-window grouping) that
needs on-device iteration, which is why it is deferred rather than shipped blind.
The single-workspace model above already satisfies the contract; isolation is
strictly-stronger insurance. Also Phase 2: the render-wedge heartbeat + auto-heal
watchdog, and the NavigationDrawer running-apps list.

## On-device validation checklist (before merge)

Deploy to htpc-1 and confirm:
- [ ] **Two apps backgrounded, switch between them, never a split view.** Launch
  Plex HTPC, launch Steam (Plex backgrounds), resume Plex from a home card, resume
  Steam — each switch shows exactly one fullscreen app, never a side-by-side tile.
- [ ] **App-initiated fullscreen churn is absorbed.** In a game/player, toggle its
  own fullscreen/menu repeatedly — the kiosk stays fullscreen (suppress_event).
- [ ] **Close promotes the survivor.** With two apps running, quit the foreground
  one — the backgrounded app comes forward fullscreen, no split
  (`exit_window_retains_fullscreen`).
- [ ] **Fresh launch is fullscreen immediately** — no visible tiled flash before
  fullscreen (the `[fullscreen]` exec-rule).
- [ ] **Kiosk survives a compositor restart.** Kill + restart Hyprland; confirm the
  daemon re-attaches (journal: `event listener attached to …`), fullscreen
  enforcement + presenter follow-focus resume, and no manual daemon restart is
  needed.
- [ ] **Config parses clean:** `hyprctl configerrors` is empty after reload.
- [ ] Single-app launch/resume still fullscreens correctly with the daemon as the
  only enforcer (QML toggle removed).
