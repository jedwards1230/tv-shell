# Input Architecture Unification

Closes #98, #96, #97, #45, #99, #100, #101. Lays the substrate for #75
(streaming home overlay). Designed and executed from
[`docs/game-shell-input-architecture-plan.md`](game-shell-input-architecture-plan.md).

## Summary

The shell's input handling had a "two ways to do one thing" smell that would
worsen as local multiplayer lands:

- **Keyboard was split across two owners** — most keys reached the shell via
  compositor → QML, but `Super` detoured through a Rust **evdev keyboard snoop**
  → `home-press`. (That snoop is why `wtype` couldn't open the nav drawer.)
- **The daemon both produced and consumed evdev devices in one namespace**,
  forcing the brittle `is_synthetic` name-match hack to skip its own uinput pads.
- **`home` was overloaded** — global return-to-shell escape *and* in-shell drawer
  toggle, disambiguated only by shell state.
- **Two daemon implementations** (Rust primary + Python rollback) = a second
  "two ways."

This PR resolves all four:

- **Rust owns the gamepad fleet only.** It stops reading the keyboard entirely;
  the keyboard (HTPC K400) is first-class on the compositor + QML via Wayland
  focus and `Keys`.
- **A first-class `intent` control surface** (`intent <name>` command →
  `intent:*` broadcast) is the single, documented, headless path for shell
  intents. Keyboard global-escape (Hyprland `Super` bind), automation, and the
  daemon's own gamepad logic all ride the identical path, so QML consumes **one**
  closed vocabulary regardless of source.
- **Device identity by fd + a DB-match-or-reject discovery gate** retires
  `is_synthetic` and stops the daemon from ever grabbing a foreign injector
  (e.g. `ydotoold`, which advertises `BTN_SOUTH`).
- **Multi-pad fleet management** with hot-join/leave and stable per-player slots,
  a presenter split (shell vs game), and fleet outputs (rumble/battery/LED).
- **Full Rust cutover** — the Python rollback daemon is deleted; QML talks to the
  daemon over a **native `Quickshell.Io` socket** (`SocketClient.qml`), replacing
  the ~29 `python3 -c` socket shims.

## The single rule

> The compositor is the focus router. Rust exists only because the gamepad isn't
> a first-class compositor input.

Every ownership question resolves by asking *"can Wayland already route this by
focus?"* — yes → compositor + QML; no (a gamepad, or a global intent that must
bypass focus) → Rust daemon + the control surface.

## Closed intent vocabulary (final wire protocol)

`home`, `home-tap`, `home-hold`, `menu`, `nav-up`, `nav-down`, `nav-left`,
`nav-right`, `select`, `back`, `settings`, `power`.

- `intent:home` — **global** return-to-shell escape (keyboard `Super`,
  automation). Always leaves the running app. QML counts **three** `intent:home`
  within 1500 ms as the reset multi-stroke (the same action gamepad Home-hold
  triggers).
- `intent:home-tap` / `intent:home-hold` — **neutral** gamepad Home signals; QML
  (which owns focus) decides what each means.
- `menu` is the focus-scoped drawer toggle (keyboard Tab / on-screen button).

Full protocol — including `get-pads`, the `pad:connected/disconnected/index/battery`
events, and the `rumble <id> <ms>` command — is in
[`docs/IPC_PROTOCOL.md`](IPC_PROTOCOL.md).

## Phases & commits

| Phase | Commit | Description |
|-------|--------|-------------|
| 0 | `64931af` | Baseline + regression checklist (`docs/INPUT_REGRESSION_CHECKLIST.md`) |
| 1 | `2f9458c` | Control surface — `intent` command + `intent:*` events (#98) |
| 2 | `8a86d8a` | QML consumes `intent:*`; de-overload `home`; rewire keyboard escape (Super press-only + QML triple-tap reset); delete keyboard snoop (#98) |
| 3 | `6ef198c` | Device identity by fd + DB-match-or-reject gate; retire `is_synthetic` (#98) |
| 4 | `59c4c90` | Multi-pad fleet refactor + hot-join/leave + stable slots (#98, #101) |
| 5 | `9f55043` | Presenter split (shell vs game) + legacy bridge deletion (#6, #75 substrate) |
| 5.5 | `90b6972` | Fleet outputs — rumble/battery/LED (#99, #100, #101) |
| 6 | `73a394a` | Mouse-mode ownership moves to QML (#45) |
| 7 | `72c12e9` | Python cutover — drop the Python rollback daemon (#96) |
| 8 | `075282a` | Native Quickshell socket — drop `python3 -c` shims (#97) |
| 9 | _(this)_ | Consolidation, smell-check, docs, PR description |

## Issues closed

| Issue | What it covers |
|-------|----------------|
| **#98** | Multi-pad fleet management — control surface, fd identity, fleet refactor, stable slots |
| **#96** | Drop the Python rollback daemon; full Rust cutover |
| **#97** | Native Quickshell `Quickshell.Io` socket; retire `python3 -c` shims |
| **#45** | Physical mouse triggers mouse-mode — QML now owns the `Theme.mouseMode` flag |
| **#99** | Gamepad rumble (`FF_RUMBLE`, cap-gated) |
| **#100** | Gamepad battery reporting (`pad:battery:*`) |
| **#101** | Stable player slots + player-indicator LED (`pad:index:*`) |
| **#75** (substrate) | Streaming home overlay — the game presenter + always-intercepted Home is the substrate; the overlay UI ships separately |

## Deviations from the original plan

The plan left four open questions for the human; all were resolved and applied,
overriding the plan's open-questions section:

1. **Keyboard escape = Super press-only (no hold).** Hyprland's `bindr` release
   leg is unreliable for bare modifiers (the very reason the snoop existed), so
   instead of a Super press/release → tap/hold split, the bare `Super` press maps
   to a single `intent home` (`scripts/super-intent.sh`, with early-boot retry).
   The **reset** action is a QML-side **multi-stroke**: three rapid `intent:home`
   within 1500 ms invokes the same reset path as gamepad Home-hold. The triple-tap
   is counted in QML off the global intent stream, **not** in the daemon.
2. **A dedicated `home` wire intent was added** to the closed vocabulary, meaning
   "global return-to-shell escape" — **distinct** from the gamepad neutrals
   `home-tap` / `home-hold`. QML maps `intent:home` → `returnToShell()`
   unconditionally and counts the triple-tap reset off it.
3. **Keyboard debug pane reimplemented in QML.** Rather than keep a daemon
   `Event::Keys` path after deleting its producer (which would re-create the
   smell), the controller-debug overlay's keyboard half now reads held keys
   directly from Wayland `Keys` in `ShellLayout.qml`. The daemon emits **no**
   `keys:` event and has **no** `kbd-log` command.
4. **Ride-alongs #99/#100/#101 and #97 are IN this PR** (not deferred to a
   fast-follow).

### Intentionally retained `python3 -c` (documented, not a regression)

`components/ControllerSettings.qml` keeps one `python3 -c` **Process** — it is an
`evdev` / `/proc/bus/input/devices` *enumerator* (lists every controller-like
device the system sees, grabbed or not, with vendor/product/path/phys), **not** a
daemon socket shim. The daemon's `get-pads` only reports the grabbed fleet, so
converting this diagnostic page to a socket call would change what it shows. Phase
8's scope was the socket shims; this enumerator is a deliberate exception. A
follow-up could add a richer daemon "enumerate all input devices" command to
retire it.

## MUST verify on game-client-1 before merge (hardware checklist)

The cross-platform Rust subset is green in CI, but the evdev/uinput/D-Bus/Hyprland
modules only compile and run on Linux. **Deploy the branch to game-client-1,
rebuild the daemon (`cargo build --release` → install), restart Quickshell, then
walk the full [`docs/INPUT_REGRESSION_CHECKLIST.md`](INPUT_REGRESSION_CHECKLIST.md).**
Consolidated hardware gates:

**Single-pad parity (the plan invariant — `Fleet`-of-one is a strict superset):**
- [ ] Gamepad D-pad + left-stick navigation moves focus (item 1, 2)
- [ ] Home-tap in shell (idle) toggles the nav drawer (item 3)
- [ ] Home-tap over a running app returns to shell (item 4)
- [ ] Home-hold triggers reset (item 5)
- [ ] Safety combos fire: force-quit (Back+Home+LB+RB), end-session (Home+B 3 s),
      suspend-stream (Start+LB+RB) — and force-quit emits the Moonlight quit chord
      in the game presenter only (items 6, 7, 8)
- [ ] App-launch grab handoff: grabbed in shell, virtual pad to the app on launch,
      regrab on return (item 9)
- [ ] Right-stick drives the cursor and flips to mouse mode (item 10)
- [ ] Single-pad reconnect preserves the player slot (item 12)
- [ ] Controller debug overlay shows held buttons (daemon) **and** held keyboard
      keys (QML/Wayland) (item 13)

**Keyboard ownership (the core change):**
- [ ] Bare `Super` press returns from a running app to the shell (item 11)
- [ ] `Super` does **not** leak to focused apps — 1Password / the streamed app
      never sees a bare Super
- [ ] Three rapid `Super` presses within 1.5 s trigger reset (the same as Home-hold)
- [ ] `Tab` toggles the nav drawer
- [ ] `wtype`/`ydotool` can drive nav **and** the daemon opens **no** keyboard
      evdev fds (`ls -l /proc/<game-shell-input pid>/fd` — confirm the real process
      name first)

**Device identity / multi-pad (#98, #101):**
- [ ] With `ydotoold` running, the daemon does **not** grab it as a bogus pad
- [ ] The daemon finds the real pad and skips its own per-player virtual pads (by fd)
- [ ] Two-pad: P2 joins as index 1, both share one menu focus, simultaneous
      Home-hold fires `intent:home-hold` exactly once
- [ ] Unplug/replug preserves stable slots (P1 stays P1 across a P2 reconnect)
- [ ] `get-pads` returns the whole fleet
- [ ] In game, each player reads its own `game-shell-virtual-pad-<slot>` (`evtest`),
      the physical pad stays grabbed, and in-game Home reaches the shell (not the game)

**Fleet outputs (#99/#100/#101) — degrade to no-op without the capability:**
- [ ] LED lights the player-indicator on slot assignment (`pad:index:*`), if the
      pad has `EV_LED`
- [ ] Battery level surfaces (`pad:battery:*`) for wireless pads
- [ ] Rumble fires on connect / via `rumble <id> <ms>`, gated by `rumbleEnabled`

**Mouse mode (#45):**
- [ ] A physical mouse wiggle flips mouse-mode **without** a daemon event
- [ ] A key / gamepad-nav reverts to controller mode; right-stick→cursor still works

**Cutover (#96/#97):**
- [ ] A fresh SDDM boot runs **only** the Rust daemon (`pgrep -af gamepad-input.py`
      empty); the full shell works
- [ ] `pgrep -af python3` is empty during normal shell use (the
      `ControllerSettings` enumerator runs only while that page is open)
- [ ] All settings pages function and subscribe streams deliver over the native socket
