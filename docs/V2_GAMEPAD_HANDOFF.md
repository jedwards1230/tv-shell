# v2 gamepad handoff — plan

> Goal, in the words it was set in: **"solid gamepad handoff between apps."** The basic
> UI is done (jedwards1230/tv-shell#483, #484). This is the second of the two v2 goals.

This document is the plan for getting a controller to drive the v2 shell and to hand
off cleanly to and from apps. It is **in-progress work**, not a reference page: phases
go stale by design, and §2 records measurements that are expected to change what the
later phases look like.

Companion documents: [`V2_DESIGN.md`](V2_DESIGN.md) (the core's contract),
[`V2_SHELL.md`](V2_SHELL.md) (the shell's), and v1's
[`INPUT_AND_STATE.md`](INPUT_AND_STATE.md), which is the behaviour being replaced.

---

## 1. The asymmetry everything follows from

Pad events and key events reach clients by **completely different routes**, and the
design is a consequence of that fact rather than a preference.

- A **pad** is read by the app directly from `/dev/input/eventN` (SDL / libinput's
  joystick path). gamescope never opens joystick nodes and never routes them. So *who
  has focus is irrelevant to pad delivery* — if the pad is to stop driving an app, it
  must be stopped **at the source**, in the core.
- A **key** goes libinput → gamescope → the focused client. A synthesised key is
  therefore routed *by the compositor*, and lands wherever gamescope thinks focus is.

Two consequences, and they are the spine of the plan:

1. **Routing is a core-side decision about a stream the core already owns.** The core
   holds `EVIOCGRAB`, owns the base layer, owns `launch`, and is the only process still
   alive when the shell is wedged. v1 put the decision partly in the shell, and its
   escape hatch consequently failed in exactly the situation it existed for.
2. **Whether the shell can *see* a key is a compositor question**, and §2 shows it is
   not yet answered on this hardware.

---

## 2. Measurements — phase 0

The plan deliberately refuses to design past these. §2.1 and §2.2 have been run;
§2.3 and §2.4 have not, and cannot be until a controller is physically connected.

Method notes that apply throughout: the shell runs as an ordinary client beside
Moonlight, launched by hand; the session unit is never touched (Moonlight is the boot
app with `boot_relaunch = "always"`, and an empty compositor is a black television
recoverable only over SSH). Frames come from gamescope's own
`GAMESCOPECTRL_REQUEST_SCREENSHOT`, waiting on the **file**, since the request property
clears at ~32 ms and the PNG lands at ~712 ms.

### 2.1 M0 — does a real evdev key reach the shell? **Measured 2026-09-09: no.**

This is the riskiest assumption in the whole plan, and it does not hold as hoped.
**Every keypress ever tested on this box was `xdotool`, i.e. XTEST**, which is injected
*inside* Xwayland and never passes through gamescope at all. `V2_DESIGN.md` §13 Q1
warned that a real evdev event may take a different route. It does.

| Probe | Result |
|---|---|
| XTEST `Menu` (the known-good path) | **drawer opens** — control passes |
| Real evdev `KEY_MENU` from a uinput keyboard | **nothing** — no drawer window, no tag lines in the shell log |

The negative was then narrowed by elimination, because a bare "it did not work" would
have been unattributable:

| Hypothesis | Verdict | Evidence |
|---|---|---|
| The probe key does nothing here | **rejected** | XTEST `Menu` opens the drawer in the same run |
| Device not classified as a keyboard | **rejected** | see below — fixed, still no delivery |
| Device not visible to gamescope | **rejected** | gamescope holds an open fd on the node (`fuser` names pid 900 `gamescope-wl`) |
| gamescope focuses something else | **rejected** | `GAMESCOPE_FOCUSED_WINDOW`, `GAMESCOPE_FOCUSED_APP` (9001), `GAMESCOPECTRL_BASELAYER_APPID` and X input focus **all** name the shell |
| The shell being an Xwayland client is the problem | **untested** | see §2.1.2 |

#### 2.1.1 A real trap found on the way, worth keeping

The first virtual keyboard advertised `KEY_ESC`, the arrows, `KEY_MENU`, A–Z and 0–9 —
and udev did **not** tag it `ID_INPUT_KEYBOARD`, so libinput never treated it as a
keyboard. systemd's `input_id` sets that property only when key codes **1..31 are all
advertised** — which includes `KEY_MINUS`, `KEY_EQUAL`, `KEY_LEFTBRACE`,
`KEY_RIGHTBRACE` and `KEY_LEFTCTRL`, none of which the shell will ever send.

Compared against the K400 (`event1`), a device known to be accepted:

```
K400  : ID_INPUT=1 ID_INPUT_KEY=1 ID_INPUT_KEYBOARD=1   Handlers=sysrq kbd event1
first : ID_INPUT=1 ID_INPUT_KEY=1                        Handlers=kbd event14
fixed : ID_INPUT=1 ID_INPUT_KEY=1 ID_INPUT_KEYBOARD=1   Handlers=sysrq kbd event14
```

**Whatever creates the core's uinput keyboard must advertise the full 1..31 block**, or
it is silently not a keyboard as far as libinput is concerned. Advertising only the
keys you intend to send is the intuitive thing to do and it is wrong. This is a
prerequisite for M0 succeeding by any route, and it cost a full measurement cycle to
find.

#### 2.1.2 What is still unknown, and it is the important part

Everything measured so far used a **virtual** keyboard. The open question is whether a
**physical** keyboard drives the v2 shell — and there is no evidence either way,
because every key ever tested on this box was XTEST.

- If the K400 **does** drive the shell, the problem is specific to uinput devices under
  this gamescope (a seat/`ID_SEAT` or libinput-acceptance question), and Q4 option (a)
  is still viable once that is understood.
- If the K400 **does not**, then real keyboard input has never worked in the v2 session
  at all. That is a larger finding than this plan, and it forces Q4 to option (b).

**This needs one keypress from a person at the television.** It is the single highest-
value unmeasured fact in the document.

An attempted shortcut — running the shell as a native Wayland client instead of an
Xwayland one — does **not** answer it: without X tagging the shell logs
*"not a gamescope focus candidate"* and is never eligible for focus, so a negative
result there means nothing.

### 2.2 M2 — what re-points X focus after an overlay is destroyed

**Not measured.** The M0 runs did read X input focus after a close, but only in a run
where the drawer had never opened in the first place, so that reading is about a
compositor state no overlay ever entered — it says nothing about focus after a destroy.
Carried unchanged on jedwards1230/tv-shell#485, which still needs its own attended
measurement: which *in-process* call re-points focus at the base surface once the
overlay window is gone.

### 2.3 M1 — does an ungrabbed pad plus a permanent presenter show as two controllers?

**Not measured — blocked.** No controller and no dongle is connected to htpc-1: no
`/dev/input/js*`, nothing in `/proc/bus/input/devices` beyond the K400, the CEC
adapter and a POROSVOC receiver, and `lsusb` shows no pad dongle. Decides §4 Q3.

### 2.4 M3 — does `EVIOCGRAB` actually stop Moonlight reading the pad here?

**Not measured — blocked**, same reason. This is the premise of all routing and is
currently verified by nobody; `core/tests/input_uinput.rs` says as much explicitly.

---

## 3. Phases

Phase 1 is deliberately ordered so a controller drives the shell **before** handoff
exists — the goal is visible early rather than only at the end.

### Phase 1 — the pad drives the shell

Ships `core/src/input/keymap.rs` (pure: pad code → key code; stick-to-dpad with
auto-repeat, porting v1's calibrated `StickRepeat` timing; Guide handled separately);
`create_keyboard()` / `emit_key()` on the backend seam, implemented in
`evdev_backend.rs`, with the keyboard created **once in `InputSession::start`** beside
the pads under the same permanence rule; routing hard-wired to `Shell` behind a new
`[input].shell_keys` sub-flag under the still-default-false `[input].enabled`; and
`owner` / `route` / `masked_keys` / `masked_axes` added to `InputReport` from the
start, so a hardware session reads what the core decided instead of inferring it.

**Gated on §2.1.2.** Under option (b) this phase instead grows an event subscription on
the core and an in-process injector in the shell.

**The honest cost, stated loudly:** with routing forced to `Shell` the pad is grabbed
unconditionally, so **Moonlight loses the pad the whole time the flag is on**. That is
why it is off by default and why enabling it is an attended act, never a deploy.

### Phase 2 — the owner decision, computed and asserted

`core/src/input/routing.rs`, pure: an `InputOwner` of `Shell | ShellOverlay | App { id,
contract } | Unknown`, and the transition plan (what to quiesce, what to mask, what to
route). No syscalls.

A **screen watcher** in the core recomputes on a ~250 ms poll *and* immediately after
the core's own `show` / `launch` / `home`, porting v1's `FOCUS_SETTLE_MS = 300`
debounce — launch flaps focus several times in a fraction of a second, which v1 learned
the hard way.

One new verb, `input-focus take|release`, sent by the shell when it opens or closes an
input-taking overlay. It is a **declaration of the shell's own state, never a command
about routing**: the core folds it into a decision it makes itself, so a shell that
dies without sending `release` self-heals when the watcher sees the window is gone.
This is v1's `set_overlay_focus` with the failure mode removed.

Routing also needs a **write path into the input thread** — `InputReports` is a
read-only `watch` receiver today. The input thread must never do an X round trip; the
watcher lives on the core side and pushes `SetOwner` messages over an `mpsc`.

**Safe default, stated explicitly: `Unknown` routes to the app, never to the shell.**
Trapping the pad in an invisible shell is the worse failure — the user sees a game and
a dead controller.

### Phase 3 — masking and the escapes

The concrete failure being prevented: you are on a card with A held down; the shell
sends `launch`; the app maps and the pad starts forwarding; the app never saw an A
*press* but receives the autorepeat or the lone A *release*, and Steam Big Picture
reads that as an activation. That is jedwards1230/tv-shell#295, observed on this
hardware. The axis sibling: you were holding Right to reach the card, so the fresh
presenter latches Right and Big Picture scrolls away forever.

Port `mask_forward_decision`, `mask_axis_forward_decision` and `abs_in_neutral_zone`
from `daemon/src/input/grab.rs` **verbatim, with their tests** — they are pure, already
tested, and hard-won on this exact hardware. Triggers (`ABS_Z` / `ABS_RZ`) are never
masked, so analog trigger use is untouched.

Masking runs in **both directions**: a button held when the drawer opens must not
instantly activate a drawer item. And leaving a target must **quiesce** it — release
every key or button the target still believes is held. `presenter.rs::quiesce` does
this for pads; the keyboard needs the same, or the shell (or Plex) is left with a stuck
key nothing will ever release.

Also ships the Guide hold → a core-side `home` performed as a base-layer write directly,
with the shell dead or alive; and the force-quit combo, gated on v1's
`presenter_owns_app` rule.

### Phase 4 — contracts, then "proven", then default-on

`contract = "gamepad" | "keyboard"` on `[[app]]` in `core.toml` — **an Ansible change**,
since that file is Ansible-managed and must not be hand-edited on the box. Plex and a
browser take the keyboard route; everything else takes the pad. `handoff` collapses
into `gamepad`.

**The acceptance list that gates flipping `[input].enabled = true`:**

1. Thirty minutes of couch use with no stuck button and no dead pad.
2. Held-button and held-axis launch verified to leak nothing.
3. The pad survives: app exits on its own; app crashes; core restarts; pad unplugged
   and replugged *while an app is on screen*.
4. Moonlight streams with **exactly one** controller visible to the host.
5. `input-state` shows `drops` and `emit_failures` at zero, or each one explained.
6. No SSH rescue was needed at any point.

The Ansible flip then happens on its own, as a change with nothing else in it.

### Phase 5 — parallel, not blocking

jedwards1230/tv-shell#473: publish the phase-2 watcher's snapshots as the event stream
(full snapshots, not deltas). jedwards1230/tv-shell#485: build the shell-side decision
module `V2_SHELL.md` §11.9c sketches, once §2.2 has said what asserting means.

---

## 4. Decisions

### Q1 — grab always, or ungrab while an app owns the screen?

`V2_DESIGN.md` §7 currently says the physical node is **ungrabbed** while the app is
the base window, so the game sees the real pad and no virtual twin double-fires.

- **Option A — grab always, the app reads the presenter.** One device ever moves.
  Masking is possible, because we own the stream. Guide can be intercepted. Costs a hop
  of latency, and rumble/battery/LED must eventually be proxied back through the core.
- **Option B — §7 as written.** Real rumble and gyro for free. But the idle presenter is
  still enumerated, so the app likely sees **two controllers** — and decisively,
  **masking becomes impossible**: the button held at launch reaches the real device and
  nothing can swallow it. That is #295 reintroduced by construction.

**Recommendation: A**, pending M1 (§2.3). Choosing A means amending §7 and recording
the reversal there rather than diverging from it quietly.

### Q2 — how do keys reach QML?

Qt 6 has no gamepad input (QtGamepad was removed), so either the core synthesises keys
or the shell grows a private pad path.

- **(a) uinput keyboard, routed by gamescope.** Matches §7, one mechanism, and is
  **required regardless** for the `keyboard` contract — Plex and a browser read no
  gamepad. Depends on gamescope routing keys correctly.
- **(b) core streams pad events over IPC; the shell injects in-process.** Immune to
  gamescope routing and to #485; makes drawer-vs-home routing an offscreen-testable
  pure decision. But it only works for our own shell and adds a second nav mechanism.

**Decided by §2.1.2, not by argument.** As measured, (a) does not currently work on
this hardware for a virtual keyboard.

### Q3 — Guide tap: pass through to the game, or always swallow?

v1 accepted pass-through (hold for home, tap reaches the game). Proposed: match v1.

### Q4 — when does `[input].enabled` flip in Ansible?

Proposed: after §3 phase 4's acceptance list, as a change with nothing else in it.

---

## 5. What of v1 to port, redesign, or drop

| v1 behaviour | Verdict | Why |
|---|---|---|
| `mask_forward_decision` / `mask_axis_forward_decision` / `abs_in_neutral_zone` | **Port verbatim** | Hard-won from #295 on this hardware; already pure and tested |
| `FOCUS_SETTLE_MS = 300` debounce | **Port** | Launch flaps focus; v2 will flap identically |
| Per-pad-complete combo detection | **Port** | Stops two pads each holding half a combo from firing it |
| `check_grab_invariant` | **Port as assert-and-log** | Cheap; catches routing drift the report cannot |
| Stable player slots, DB-match discovery, hot join/leave | **Already in v2** | `identity.rs` / `fleet.rs` / `discovery.rs` |
| Presenter switching by create/destroy | **Redesign** | Forbidden by #402 / §7 — a hotplug event Moonlight forwards to the host. Route, never rebuild |
| `shell_focus` + `overlay_focus`, both shell-declared | **Redesign** | The core derives shell-on-screen from the base layer; the shell declares only its overlay |
| Shell-delivered escape (`intent home-hold`) | **Drop** | Failed exactly when it was needed. The core performs the base-layer write itself |
| Rumble / battery / LED | **Defer** | Not on the handoff path. Under option A it becomes a real follow-up; name it, don't build it here |
| Mouse emulation, capture mode, remap table | **Drop for now** | None is on the handoff path |

---

## 6. Testing — what is provable where

| Layer | What it proves |
|---|---|
| Pure Rust, no seat | the owner truth table; mask key/axis decisions; the keymap; the transition *plan* (quiesce-then-switch, masks seeded) |
| Recording backend double | the **call sequence**: a transition quiesces before it switches; masks are seeded from held state; no presenter is created or destroyed outside `start` |
| `/dev/uinput` | the kernel accepts the keyboard profile and its devnode reads back |
| Offscreen QML | the shell sends exactly one `input-focus take` per drawer open and one `release` per close, and none on a reconnect |
| Real gamescope | §2's measurements; app-id resolution across a launch |
| The sofa, irreducibly | "no leaked presses, no stuck buttons, no dead pad" over 30 minutes |

Against the three recorded ways a green suite can be empty:

- **The rule is untested** → every new pure rule ships with a mutation note in its doc
  comment, the discipline already used in `mod.rs`.
- **The test runs nowhere** → already closed, not outstanding.
  `core/tests/input_uinput.rs` was wired into `rust.yml` by
  jedwards1230/tv-shell#469 as the `core-uinput` job, which runs on the bare runner VM
  and has since gone green on every execution; it is now a blocking leg. The gates that
  make that meaningful are in place: `modprobe uinput` failing is an explicit `::error::`
  rather than a silent skip, and the suite PANICS instead of skipping when
  `TV_SHELL_TEST_UINPUT` is unset under `--ignored`, so the step cannot pass by running
  nothing. Phase 1 inherits this rather than having to build it — keep the panic-not-skip
  gate.
- **The state is unreachable** → the transition states must be reachable from the
  session double, and owner/route/mask state must be in `InputReport` from phase 1, so a
  hardware session reads the decision instead of inferring it.

Note also that `justin` is **not** in the `input` group on htpc-1, so `/dev/uinput` can
be written but the resulting `/dev/input/eventN` cannot be read back there. Creation
works; readback does not. `CONTRIBUTING.md` already documents this as the two-permission
trap.
