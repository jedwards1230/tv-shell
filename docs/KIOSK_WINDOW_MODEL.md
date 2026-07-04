# Kiosk Window Model

How game-shell guarantees the kiosk contract on Hyprland, why the current
approach keeps failing, and the plan to make it structural.

**The contract.** Exactly one app window is visible and fills the screen; the
shell (Quickshell) sits deterministically above or below it; backgrounded apps
(Plex HTPC, Steam) keep running but never share the screen. Two app windows must
never be visible at once.

**Assumptions.** Hyprland ~0.55 (config targets 0.55; see `config/hyprland.conf`).
The Quickshell shell is a **layer-shell** surface (`WlrLayershell`, holds
`WlrKeyboardFocus.Exclusive` while mapped) — it is *not* a tiled toplevel, so it
is unaffected by window rules and workspace switches and is not a party to the
tiling layout. App windows are ordinary xdg toplevels the tiler owns.

## How it works today (reactive enforcement)

Everything runs on **one workspace**. Nothing in the repo assigns windows to
distinct workspaces (verified: no `workspace` dispatch/rule anywhere in
`shell/`, `daemon/src/`, `config/`, `scripts/`). "One app fills the screen" is
maintained by *reacting* to Hyprland's event stream after the tiler has already
laid windows out, from **two independent enforcers**:

1. **Daemon** (`daemon/src/hyprland.rs`) — reads `.socket2.sock` events. On
   `openwindow` → `force_fullscreen` (`focuswindow` + `fullscreen 0 set`, idempotent).
   On `closewindow`/`movewindowv2`/`activewindowv2` → `enforce_active_fullscreen`
   (`fullscreen 0 set` if the active window is windowed). #293/#294/#303.
2. **QML** (`shell/components/AppLifecycleManager.qml`) — on launch/resume,
   `focuswindow` the target then re-assert fullscreen: the class path
   (`focusWindow`) *reads* `hypr-active` and toggles only if windowed; the
   **address path** (`focusByAddress` → `focusWindowAddr`, used by the per-window
   home cards — i.e. the Steam/Plex resume path) fires `hyprctl dispatch
   fullscreen 0` **unconditionally** (a bare toggle, not `... set`).

Declarative backstops in `config/hyprland.conf`: `windowrule = fullscreen on,
match:class .+` (only lands if the new window also wins initial keyboard focus on
the same map — the shell's exclusive grab usually loses that race, so this is
best-effort), and `misc:on_focus_under_fullscreen = 1` (on focusing a window
*under* a fullscreen one, atomically swap fullscreen to it — correct, but only
fires when the outgoing window is genuinely fullscreen).

## Why reactive enforcement structurally cannot win

The bad state — two app windows tiled side-by-side — is **representable**: the
single shared workspace lets the tiler split whenever two toplevels coexist and
neither is fullscreen. Enforcement is a control loop chasing a setpoint the tiler
keeps moving, and it has three structural defects:

- **Two enforcers, no serialization, conflicting semantics.** The daemon's
  idempotent `fullscreen 0 set` and QML's non-idempotent `fullscreen 0` *toggle*
  act on the same global layout with no ordering. They race and can undo each
  other.
- **A toggle is state-dependent; the truth is shared and mutable.** `fullscreen
  0` flips whatever the current state is. Read-then-toggle (QML class path) races
  any concurrent writer; the address path doesn't even read.
- **Recovery has a blind spot.** `on_focus_under_fullscreen` only helps while a
  window is *still fullscreen*. Once both windows are tiled, no declarative rule
  re-fullscreens on focus — only an actor can, and that's the racy actor above.

### Today's incidents, mapped to the defects

1. **Split view on Steam resume (Plex backgrounded).** Resuming an
   already-fullscreen app via the address path runs `focuswindow` (Hyprland, via
   `on_focus_under_fullscreen=1`, correctly swaps fullscreen to Steam) → then
   QML's **unconditional `fullscreen 0` toggle flips Steam back OUT** → Steam and
   Plex are both tiled → the tiler splits them. The daemon's `activewindowv2`
   enforcement races to re-fullscreen and sometimes wins, sometimes not — hence
   "repeatedly." Killing Plex "fixed" it because `closewindow` enforcement then
   ran with a single window and no racing partner. This reproduces even with a
   perfectly healthy daemon; it is a QML-intrinsic bug amplified by the two-enforcer race.
2. **Daemon deaf to a restarted Hyprland (>1h).** `resolve_hypr_signature()`
   returned an inherited `HYPRLAND_INSTANCE_SIGNATURE` without checking liveness.
   A long-lived `systemd --user` daemon that inherited the var (imported into the
   user manager by a prior Hyprland) stays pinned to the *dead* instance after a
   render-hang kill+restart; every socket path resolves to a dead socket
   ("Connection refused"), and reconnect backoff re-resolved to the same stale
   signature forever. Silent — everything else looked healthy; trapped two
   investigators.
3. **Presenter flapped Shell↔Game.** The gamepad presenter follows
   `activewindow`. During the incident-1 focus churn the event stream itself
   flaps, so the presenter flaps — a symptom of reacting to a racing stream.
4. **HDMI/CEC flap wedges Hyprland's render loop** (hyprctl still answers, frames
   frozen); kill+restart is the only recovery, which then triggers incident 2.

## Options evaluated

**(a) Declarative windowrules + per-app workspace isolation.** `windowrulev2 =
fullscreen` at map time is already present but best-effort (focus-race gated).
The strong form is **one app window per workspace**: the tiler can then *never*
place two app windows together — the bad state becomes unrepresentable, and
"focus an app" becomes "switch to its workspace" (a deterministic, idempotent op)
instead of "re-assert fullscreen" (a racy toggle). The shell is layer-shell, so
it is unaffected and stays visible across workspace switches. This obsoletes most
reactive enforcement. Caveat grounded in Hyprland behavior: `dispatch
focuswindow` does **not** reliably follow to a window on another workspace
([hyprwm/Hyprland#1611](https://github.com/hyprwm/Hyprland/issues/1611)), so the
resume path must switch workspace explicitly — a QML change — and multi-window
apps (Steam splash + main) must be grouped onto one workspace by class.

**(b) Daemon hardening as backstop.** Instance-signature liveness re-resolution +
reconnect (fixes the incident-2 class outright); enforcement stays only as a gap
filler. Cheap, high value, no behavior risk. **Today's socket loss already
reconnects with backoff** — the bug was purely that re-resolution returned the
stale signature; fixed here.

**(c) Deeper decoupling — gamescope / cage / custom wlroots kiosk.** gamescope's
model *is* one fullscreen app at a time, which is attractive, but it is built for
a single game under a compositor, not for hosting a persistent Quickshell
**layer-shell** UI plus arbitrary backgrounded apps — layer-shell/multi-toplevel
support is not its use case, and Moonlight/CEC/HDR/VRR paths would need
re-proving. cage is single-app kiosk (no persistent overlay shell). A custom
wlroots kiosk is a rewrite. **Honest assessment: this is a large rewrite that
option (a) makes unnecessary** — Hyprland already has the isolation primitive
(workspaces); we simply aren't using it. Reject (c).

**(d) Hyprland-native patterns not yet used.** Numbered-workspace-per-window via
`movetoworkspacesilent` on `openwindow` (daemon-side, class-grouped) + `workspace
N` on resume (QML-side); `special` workspaces are an alternative but muddy the
"exactly one visible" model. This is the mechanism behind (a).

## Recommendation

Adopt **(a) + (b)**, reject **(c)**. Make isolation structural (one app window per
workspace) and keep a single, idempotent daemon enforcer as the only backstop;
harden the daemon's compositor attachment so a restarted Hyprland self-heals.
Delete the QML fullscreen toggles — they are the incident-1 root cause and are
redundant once isolation holds.

### Migration plan

**Phase 1 — daemon hardening + declarative correctness (this PR; verifiable
off-device, zero behavior risk).**
- `resolve_hypr_signature()` now **scans `$XDG_RUNTIME_DIR/hypr/` for the live
  socket dir first** and only falls back to the inherited env var when none
  exists — so a reconnect self-heals onto a restarted Hyprland. Kills incident 2.
  Unit-tested (`daemon/src/session_env.rs`).
- Connect-time signature logging in `hyprland.rs` (`event listener attached to
  <dir>`) so a future stale attach is visible in the journal at a glance.
- Correct the `on_focus_under_fullscreen` documentation in `config/hyprland.conf`
  to the real Hyprland semantics (it is a *resume/focus-under* knob, not an
  *open* knob; value 1 = atomic fullscreen swap).

**Phase 2 — single enforcer (small, QML lane; needs on-device validation).**
Remove QML fullscreen re-assertion entirely (`ensureFullscreen`,
`ensureFullscreenQuery`, and the `fullscreen 0` dispatch on both resume paths in
`AppLifecycleManager.qml`). The daemon's idempotent `fullscreen 0 set` becomes
the **sole** enforcer. This alone kills incident 1 (no toggle to fight). Validate:
Steam resume with Plex backgrounded no longer splits.

**Phase 3 — workspace isolation (the structural guarantee; needs on-device
validation).** On `openwindow`, the daemon moves each new app window to its own
workspace, **grouped by class** so a multi-window app (Steam splash + main) stays
together: query `j/clients`, reuse the workspace of an existing window of that
class else allocate the lowest unoccupied number, `dispatch
movetoworkspacesilent N,address:<addr>` then `workspace N`. The resume path
becomes an explicit `dispatch workspace N` (QML), since `focuswindow` does not
follow across workspaces. With one app per workspace the tiler cannot split, so
fullscreen enforcement degrades to a belt-and-suspenders no-op (kept only to
cover a layer-shell exclusive-zone edge). Land behind a default-off daemon config
flag (`[kiosk] isolate_workspaces`) so it merges safely and is flipped on-device
to validate. Open questions to settle on-device: does the shell layer surface
render correctly on the switched-to workspace; Steam's exact multi-window
sequence; return-to-home workspace selection.

### Claims needing on-device validation before merging Phases 2–3
- `focuswindow` cross-workspace non-follow behavior on *this* Hyprland build
  (documented upstream; confirm on 0.55).
- A lone tiled window with `gaps_out=0`/`border_size=0` fully covers the screen
  including any layer-shell exclusive zone (else keep the fullscreen backstop).
- Steam's window-open sequence groups correctly by class under isolation.
- Removing the QML toggle (Phase 2) does not regress the single-app fullscreen-
  on-resume case where the daemon is the only enforcer.
