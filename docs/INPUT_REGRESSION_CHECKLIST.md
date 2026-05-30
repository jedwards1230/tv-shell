# Input Regression Checklist (input-unification)

> **Purpose:** A concrete, manual checklist of the **current single-pad behavior**
> that EVERY phase of the input-unification PR must preserve. Run it on
> game-client-1 after each phase's deploy, before moving on.
>
> This is the "Phase 0 baseline" referenced throughout
> `docs/game-shell-input-architecture-plan.md`. Each item cites the exact current
> code path (`file:line`) so a later phase can self-check that it moved/renamed
> the seam without changing the observable behavior. Line numbers are from the
> `feat/input-unification` worktree at Phase 0 (branched from main) and will
> drift as phases land — trust the code, use the lines as a guide.

## How to run

1. Deploy the branch to game-client-1 and restart Quickshell + the Rust daemon
   (see `game-shell-dev` skill / CLAUDE.md deploy cycle).
2. Have a single Xbox-style pad connected and a K400 keyboard.
3. Walk each numbered item; tick PASS/FAIL. A single FAIL blocks the phase.
4. For the daemon-fd checks, find the real process first:
   `pgrep -af game-shell-input` (NOT `gamepad-input.py` once cut over).

## Baseline green gate (run from `rust/`)

These must stay green every phase (cross-platform subset only — the Linux-only
modules build in CI / on game-client-1):

```
cargo fmt --check
cargo test
cargo clippy --all-targets -- -D warnings
```

Phase 0 result (dev mac, 2026-05-30): `fmt --check` clean; `cargo test` =
**74 passed, 0 failed**; `clippy --all-targets -D warnings` clean.

---

## 1. Gamepad navigation — D-pad

- **Expect:** D-pad up/down/left/right move menu focus in the shell home screen;
  holding a direction auto-repeats after a short delay.
- **Code path:** `input.rs:851-888` (`handle_abs` `ABS_HAT0X`/`ABS_HAT0Y` →
  emit `KEY_LEFT/RIGHT/UP/DOWN` via `emit_key`). Daemon must be **grabbed**
  (`handle_gamepad:738` routes to `handle_event` only when `self.grabbed`).
- **Wire:** virtual-kb arrow keys → QML `Keys`/`KeyNavigation` focus chains.

## 2. Gamepad navigation — left stick

- **Expect:** Left stick outside the deadzone moves focus the same as the D-pad;
  held deflection auto-repeats (300 ms initial delay, 150 ms interval).
- **Code path:** `input.rs:889-890` (`ABS_X`/`ABS_Y` → `handle_stick_axis`),
  `handle_stick_axis:911-938`, repeat timer `start_stick_repeat:990-1019`
  (`STICK_INITIAL_DELAY_MS` / `STICK_REPEAT_INTERVAL_MS`). Deadzone math
  `state::left_stick_target`. Calibration from `absinfo` on connect
  (`calibrate:591-628`).

## 3. Home-tap in shell (idle) = drawer toggle

- **Expect:** A short press+release of the gamepad Home (Guide) button while in
  the shell toggles the navigation drawer open/closed (and wakes AV).
- **Code path:**
  - Daemon: `BTN_MODE` release before the 2 s hold → `Event::HomePress`
    (`input.rs:778-790`), wire `home-press`.
  - QML: `InputManager.qml:70-71` `home-press` → `homePressed()`.
  - `shell.qml:76-83` `onHomePressed`: in `idle` calls
    `avController.wake()` + `root._layout.handleHomeTap()`.
  - `ShellLayout.qml:43-57` `handleHomeTap()` toggles `navDrawer.opened`
    (or closes powerOverlay/notificationCenter first).
- **Note for later phases:** the plan renames `home-press` → `intent:home-tap`
  and routes tap→`menu` in QML; the *observable* drawer-toggle must be identical.

## 4. Home-tap over a running app = return to shell

- **Expect:** While a **local app** is running (`appRunning` state), a Home-tap
  toggles the overlay nav drawer over the app (NOT a direct return). From that
  overlay drawer, selecting Home returns to the shell.
- **Code path:** `shell.qml:76-79` `onHomePressed` in `appRunning` →
  `root.overlayDrawerOpen = !root.overlayDrawerOpen`. Overlay drawer UI:
  `ShellLayout.qml:184-225`; `onHomeSelected → returnToShellRequested`.
- **Subtlety:** In `appRunning` the daemon **stays grabbed** (local app launch
  does not release — see item 9), so Home-tap still reaches QML via `home-press`.
- **Distinct from streaming:** in `streaming`/`reconnecting` the daemon is
  **released**, so Home is not intercepted by the daemon at all; return paths in
  those states come from combos / stream lifecycle, not Home-tap.

## 5. Home-hold = reset

- **Expect:** Holding the gamepad Home button for ~2 s triggers the "reset"
  action:
  - In `appRunning`: returns to shell (`root.returnToShell()`).
  - In `idle`: closes all overlays/drawers/panels and refocuses the home screen.
- **Code path:**
  - Daemon: `start_home_hold:1091-1102` arms a `HOME_HOLD_SECS` (2.0) timer;
    `HomeHoldFired:693-700` → `Event::ComboHomeHold` (wire `combo:home-hold`).
    Release before the timer aborts it and emits `home-press` instead
    (`input.rs:782-788`).
  - QML: `InputManager.qml:72-73` `combo:home-hold` → `homeHeld()`.
  - `shell.qml:84-94` `onHomeHeld`: `appRunning`→`returnToShell()`;
    `idle`→ close navDrawer/settings/notificationCenter/powerOverlay + `focusHome()`.
- **Note for later phases (resolved open question):** keyboard reset is a
  **3× Super press within 1500 ms**, counted in QML off the `intent:home` stream
  — it must map to this same idle/appRunning reset path.

## 6. Safety combo — force-quit

- **Expect:** Holding **Back + Home + LB + RB** (`BTN_SELECT + BTN_MODE +
  BTN_TL + BTN_TR`) simultaneously fires an instant force-quit (no hold timer).
  When the daemon is **ungrabbed** (i.e. streaming), it ALSO emits
  `Ctrl+Alt+Shift+Q` via uinput to quit Moonlight.
- **Code path:** `check_quit_combo:1072-1080` (`QUIT_COMBO_KEYS` =
  `[BTN_SELECT, BTN_MODE, BTN_TL, BTN_TR]`, `config.rs:93`); emits
  `send_moonlight_quit:326-340` only when `!self.grabbed`; always publishes
  `Event::ComboForceQuit` (wire `combo:force-quit`).
- **QML:** `InputManager.qml:52-53` → `forceQuitRequested()` →
  `shell.qml:66` `root.forceQuit()`.
- **Note:** combos are gamepad-button based (`held_keys`), NOT keyboard chords —
  they survive the keyboard-snoop deletion (Phase 2) unchanged.

## 7. Safety combo — end-session

- **Expect:** Holding **Home + B** (`BTN_MODE + BTN_EAST`) for 3 s fires
  end-session.
- **Code path:** `check_combo_start:1044-1054` (`COMBO_KEYS = [BTN_MODE,
  BTN_EAST]`, `COMBO_HOLD_SECS = 3.0`, `config.rs:89-90`);
  `ComboEndSessionFired:701-710` re-checks the subset is still held →
  `Event::ComboEndSession` (wire `combo:end-session`).
- **QML:** `InputManager.qml:54-55` → `endSessionRequested()` →
  `shell.qml:67` `inputManager.endSession()` (runs `/usr/local/bin/end-game-session`).

## 8. Safety combo — suspend-stream

- **Expect:** Holding **LB + RB + Start** (`BTN_TL + BTN_TR + BTN_START`) fires
  suspend-stream — but NOT while the force-quit combo is also satisfied.
- **Code path:** `check_suspend_combo:1082-1089` (`SUSPEND_COMBO_KEYS =
  [BTN_START, BTN_TL, BTN_TR]`, `config.rs:96`), guarded by
  `!subset_held(QUIT_COMBO_KEYS,…)` → `Event::ComboSuspendStream`
  (wire `combo:suspend-stream`).
- **QML:** `InputManager.qml:56-57` → `suspendStreamRequested()` →
  `shell.qml:68-71` calls `streamManager.suspend()` only in
  `streaming`/`reconnecting`.

## 9. App-launch grab handoff (grab on shell, release on app)

- **Expect (two distinct cases):**
  - **Streaming launch:** the daemon **releases** the grab so the streamed
    session receives the raw pad. Returning to the shell **re-grabs**.
  - **Local app launch:** the daemon **stays grabbed** (so shell nav / Home-tap
    overlay keep working over the app); return-to-shell keeps/re-asserts the grab.
- **Code path:**
  - Streaming: `StreamManager.qml:168` `requestInputRelease()` →
    `shell.qml:144` `inputManager.release()` → daemon `Control::Release` →
    `do_ungrab:541-558`. Re-grab on return: `shell.qml:134/145/155/167/181`
    `inputManager.grab()` → `do_grab:521-539`.
  - Local app: `AppLifecycleManager.qml:31/47` `hyprctl dispatch exec` +
    `appLaunched()` → `shell.qml:104-106` sets `state="appRunning"` with **no**
    release call.
  - Auto-grab on (re)connect: `try_connect:560-580` calls `do_grab` +
    `Event::ControllerWake`.
- **Verify directionally:** `status` IPC returns `connected:grabbed` in shell,
  `connected:released` while streaming, and `connected:grabbed` again after
  return.

## 10. Right-stick = mouse mode

- **Expect:** Right stick outside the deadzone moves the mouse cursor (quadratic
  velocity, ~60 Hz) and flips the UI into mouse mode. LB/RB emit left/right
  mouse clicks. Right-stick cursor + clicks work in **both** grabbed and
  ungrabbed states.
- **Code path:**
  - Cursor: `handle_rstick_axis:940-984` (sets `input-mode:mouse` on first
    deflection), 60 Hz `MouseTick:672-692` → `emit_mouse_move`, velocity
    `state::compute_mouse_velocity`. Ungrabbed path still active
    (`handle_event_ungrabbed:813-819`).
  - Clicks: `input.rs:793-797` (grabbed) and `:827-831` (ungrabbed) →
    `emit_mouse_button(BTN_LEFT/RIGHT)`.
  - Mode flag: `Event::InputMode(Mouse)` (wire `input-mode:mouse`) →
    `InputManager.qml:58-63` sets `Theme.mouseMode = true`; controller input
    reverts it to `false` (`input-mode:controller`).
- **Note for later phases (#45):** Phase 6 moves `Theme.mouseMode` ownership to
  QML (real Wayland pointer sets it). The right-stick→cursor daemon path here
  must keep working as ONE of several mouse-mode sources.

## 11. Super-tap (keyboard) = return-from-app / drawer

- **Expect:** A bare **Super (Meta/Guide)** keyboard tap behaves like a Home-tap
  (drawer toggle in shell, overlay-drawer / return over an app). A focused app
  (e.g. 1Password) must NOT see the bare Super press.
- **Code path (current, pre-cutover):**
  - Hyprland: `config/hyprland.conf:76` `bind = , Super_L, exec, true` consumes
    the press so apps don't see it.
  - Daemon snoop: `keyboard_supervisor:1192-1217` reads keyboards **without
    grab**; `handle_kbd:1106-1145` routes `KEY_LEFTMETA/RIGHTMETA` through a
    tap-vs-hold flow (`start_routed_hold`/`resolve_routed_release:1147-1179`)
    that emits `Event::HomePress` (tap) / `Event::ComboHomeHold` (hold), reusing
    the same wire events as gamepad Home.
- **Note for later phases (resolved open question):** Phase 2 **deletes the
  keyboard snoop entirely**. Super becomes a Hyprland **press-only** bind →
  `intent home` (the dedicated global return-to-shell intent), and
  **3 rapid Super presses within 1500 ms** → reset. QML maps `intent:home` →
  `returnToShell()` unconditionally. After cutover, verify the daemon opens
  **no keyboard evdev fds** (`ls -l /proc/<pid>/fd` on the real
  `game-shell-input` process) and 1Password never sees a bare Super.

## 12. Single-pad reconnect

- **Expect:** Unplugging the pad emits a disconnect (UI shows "Controller
  Disconnected"); replugging re-discovers it, re-grabs, and emits a wake (UI
  shows "Controller Connected"). Stick state is reset on disconnect so no key
  is left stuck down.
- **Code path:**
  - Disconnect: `next_event` error → `on_disconnect:582-589`
    (`Event::ControllerDisconnected`, `reset_stick_state`, clears
    `connected`/`grabbed`).
  - Reconnect poll: `run` loop `:236-238` every 2 s while `gamepad.is_none()`
    → `try_connect:560-580` (`find_gamepad` → calibrate → grab →
    `Event::ControllerWake`).
  - QML: `InputManager.qml:64-69` `controller-wake` / `controller-disconnected`
    → notifications + `controllerWake()` / `controllerDisconnected()`.
- **Note for later phases:** Phase 4 replaces the single
  `gamepad: Option<EventStream>` with a `Fleet`; a fleet-of-one MUST reproduce
  this exact connect/disconnect/regrab/stick-reset behavior.

## 13. Controller debug overlay (buttons + keys)

- **Expect:** With `controllerDebug` enabled, an on-screen pane shows currently
  held controller inputs (`buttons:`) and held keyboard keys (`keys:`).
- **Code path:** `ShellLayout.qml:259-381`; subscribes via `debugSubscribe`
  (sends `kbd-log on` + `subscribe`), parses `buttons:`/`keys:` lines.
  Daemon: `notify_held_buttons:356-373` (`buttons:`),
  `notify_held_keys:375-387` (`keys:`, from the keyboard snoop).
- **Note for later phases (resolved open question):** the keyboard half
  (`keys:` + `kbd-log`) goes away with the snoop in Phase 2. The keyboard pane
  is to be **reimplemented in QML** reading Wayland `Keys` directly (no daemon
  `Event::Keys` path). The controller `buttons:` half stays daemon-driven.

---

## Cross-phase invariants (the plan's single rule)

- The compositor is the focus router; Rust exists only for the gamepad + global
  intents that must bypass focus.
- Every phase keeps the **cross-platform Rust tests green** and leaves the
  **single-pad rig logically intact** — `Fleet`-of-one is a strict superset of
  the single-pad daemon, so items 1-13 must all still pass.
- Safety combos (6/7/8) are gamepad-button based and independent of the keyboard
  ownership change.
