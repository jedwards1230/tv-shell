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
2. **Whether the shell can *see* a key is a compositor question**, and it is answered:
   §2.1 shows a synthesised key reaches the shell just as a physical one does — but that
   gamescope silently drops `KEY_MENU`, which is the drawer's only binding.

---

## 2. Measurements — phase 0

All four have now been run on htpc-1 (2026-09-09/10). Two settle open questions; one
found a sharp, unexplained contrast that phase 1 depends on.

Method notes: the shell runs as an ordinary client beside Moonlight, launched by hand;
the session unit is never touched (Moonlight is the boot app with
`boot_relaunch = "always"`, and an empty compositor is a black television recoverable
only over SSH). Frames come from gamescope's own `GAMESCOPECTRL_REQUEST_SCREENSHOT`,
waiting on the **file**, since the request property clears at ~32 ms and the PNG lands
at ~712 ms.

### 2.1 M0 — does a real key reach the shell? **Yes. The drawer key was the problem.**

**Resolved 2026-09-10.** A uinput keyboard drives the v2 shell exactly as a physical one
does. The long-running negative was never about routing, devices, or seats — it was the
one key being probed.

| Probe | Device | Result |
|---|---|---|
| `KEY_ENTER` | uinput, Moonlight suspended | **reacts** (9001 → 9003) |
| `KEY_ENTER` | uinput, Moonlight running | **reacts** |
| `Return` | XTEST, Moonlight running | reacts |
| `Menu` | XTEST | **opens the drawer** |
| **`KEY_MENU`** | **uinput** | **nothing** |

Enter and the arrows arrive from a synthesised device under every condition tested.
`KEY_MENU` does not arrive at all, while XTEST `Menu` does — and XTEST is injected
*inside* Xwayland, bypassing gamescope entirely. So **gamescope does not deliver
`KEY_MENU` to the client.**

**This is a live bug, not a curiosity.** `Keys.onMenuPressed` (`Main.qml:174`) is the
*only* binding that opens the drawer, so **the drawer is unreachable from any real input
device** — keyboard today, and gamepad once phase 1 exists. Everything that ever appeared
to work on it was XTEST driven from an SSH session.

**Required fix:** bind the drawer to a key gamescope actually routes. v1 used **Tab** for
the same surface, which is the natural candidate and should be verified the same way
before being relied on. `Menu` may remain as an additional binding; it simply cannot be
the only one.

**Consequences for the plan:**

- **Q2 resolves to option (a).** The core can drive the shell with a uinput keyboard, and
  no seat work is needed — a udev rule tagging the device `uaccess` + `ID_SEAT=seat0` was
  tried and changed nothing in either direction, because nothing was wrong with the seat.
- Phase 1 is unblocked, with one addition: **the shell's drawer binding must change**, and
  the keymap must not emit a code gamescope drops.
- Any key the core intends to emit should be **verified to arrive**, not assumed.
  `KEY_MENU` looked entirely reasonable on paper.

#### 2.1.1 A real trap found on the way, worth keeping

The first virtual keyboard advertised `KEY_ESC`, the arrows, `KEY_MENU`, A–Z and 0–9 —
and udev did **not** tag it `ID_INPUT_KEYBOARD`, so libinput never treated it as a
keyboard. systemd's `input_id` sets that property only when key codes **1..31 are all
advertised** — which includes `KEY_MINUS`, `KEY_EQUAL`, `KEY_LEFTBRACE`,
`KEY_RIGHTBRACE` and `KEY_LEFTCTRL`, none of which the shell will ever send.

```
K400  : ID_INPUT=1 ID_INPUT_KEY=1 ID_INPUT_KEYBOARD=1   Handlers=sysrq kbd event1
first : ID_INPUT=1 ID_INPUT_KEY=1                        Handlers=kbd event14
fixed : ID_INPUT=1 ID_INPUT_KEY=1 ID_INPUT_KEYBOARD=1   Handlers=sysrq kbd event14
```

**Whatever creates the core's uinput keyboard must advertise the full 1..31 block**, or
it is silently not a keyboard as far as libinput is concerned. Advertising only the keys
you intend to send is the intuitive thing to do and it is wrong.

#### 2.1.2 How this measurement kept going wrong, and what fixed it

Three consecutive runs produced negatives that were **not evidence**, and the failures
are worth recording because they generalise:

- **The probe key did not exist.** The only binding that opens the drawer is
  `Keys.onMenuPressed` (`Main.qml:174`), and **a Logitech K400 Plus has no Menu key**.
  Every "the shell did not react" result was asking for an impossible press. The probe
  became **Enter**, which activates the focused card.
- **The home screen has one card**, so Left/Right are legitimate no-ops — the first run
  used them as the probe *and as the control*, so both failed and the run could not
  distinguish "keys do not work" from "this key does nothing here".
- **The success signal was a whole-frame hash**, and the home screen has a live clock.
  The frame differs after 30 s regardless of input, which is a guaranteed false positive.
  The signal became `GAMESCOPE_FOCUSED_APP` changing.
- **Zero input events were indistinguishable from ignored input events.** The device
  node is now read on a separate channel, so "nobody pressed anything" and "the shell
  ignored it" are different findings.

### 2.2 M2 — what re-points X focus after an overlay is destroyed

**Not measured.** The M0 runs did read X input focus after a close, but only in a run
where the drawer had never opened, so that reading is about a compositor state no
overlay ever entered. Carried unchanged on jedwards1230/tv-shell#485, which still needs
its own attended measurement: which *in-process* call re-points focus at the base
surface once the overlay window is gone.

### 2.3 M1 — does an idle presenter show up as a second controller? **Yes.**

**Measured 2026-09-10.** With Moonlight running and the physical pad **ungrabbed**, a
permanent virtual pad was created. Moonlight opened its `/dev/input/event15` within
**2 seconds** and held it alongside the real pad's `event3` for the rest of the probe.

**The idle presenter is enumerated, so the app sees two controllers.** This settles §4
Q1 in favour of **option A (grab always)**, and means **`V2_DESIGN.md` §7's ungrab
bullet does not survive** — under it, every game would see a phantom second pad.

### 2.4 M3 — does `EVIOCGRAB` take the pad from a running app? **Yes.**

**Measured 2026-09-10.** Established read-only first: Moonlight reads
`/dev/input/event3` (**evdev**, not `js0`) and is its only holder. Then, with the user
driving the pad continuously:

| Phase | Events we received |
|---|---|
| reading ungrabbed (10 s) | 2899 |
| **reading grabbed (12 s)** | **4015** |

The grab succeeds and every event in the second row is one Moonlight did not get. **The
premise of the entire routing design holds.** The two-phase shape is deliberate: without
the ungrabbed phase, a zero count could not be told apart from nobody touching the pad —
which is exactly how the first two attempts wasted a cycle.

The controller itself is a **Vader 4 Pro whose dongle presents as `045e:028e`
"Microsoft X-Box 360 pad"** (XInput mode). Three consequences: it is already the core's
canonical known device, so the DB-match-or-reject discovery gate accepts it with no
work; its identity resolves on the stable `phys:` tier (empty `Uniq`, real `Phys`), so
player slots survive a replug into the same port; and in XInput mode it exposes **no
companion touchpad or motion nodes**, so §7's inhibition concern does not arise here —
at the cost of the gyro being unreachable by this path.

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

**Unblocked** by §2.1, with one addition: the shell's drawer binding must move off
`KEY_MENU`, which gamescope does not deliver, and the keymap must emit only codes
verified to arrive.

**Shipped, default off.** `core/src/input/keymap.rs`, the `create_keyboard` /
`emit_key` backend seam, `[input].shell_keys`, and `owner` / `route` /
`masked_keys` / `masked_axes` on `InputReport` are in. Nothing changes on a box
until that flag is set, and it has not been exercised on hardware yet — the
acceptance for that is a person at the television, not a green suite.

**The honest cost, stated loudly:** with routing forced to `Shell` the pad is grabbed
unconditionally, so **Moonlight loses the pad the whole time the flag is on**. That is
why it is off by default and why enabling it is an attended act, never a deploy.

### Phase 2 — the owner decision, computed and asserted

**Shipped, still under the default-off `[input].enabled`.**

Why it had to follow phase 1 immediately, measured rather than argued: with
`shell_keys = false`, holding Guide correctly returned the screen to the shell
(jedwards1230/tv-shell#498) and **the home screen was then completely inert**, because
the pad was still forwarding to the app's presenter. Routing was static — `shell_keys`
pinned it for the life of the session — so you could have a drivable shell or a working
app, never both.

What landed:

- `core/src/input/routing.rs`, pure, no syscalls: an `InputOwner` of `Shell |
  ShellOverlay | App { id } | Unknown`, the `route()` it implies, and the transition
  plan (`from`, `to`, what to quiesce, what to route) as data. The per-app `contract`
  is **not** here — that is phase 4's `[[app]]` change, and adding a field nothing can
  populate would have been a decision input no config could reach.
- `core/src/input/watcher.rs`: a thread that reads the screen, computes the owner, and
  **pushes** it into the input runtime. The input thread never does an X round trip —
  that would put compositor latency, and a hung X server, on the pad path.
- A **write path into the input thread**: `InputHandle::control()` hands out an
  `mpsc` sender carrying `Control::SetOwner`, beside the read-only `watch` receiver
  `InputReports` already was.
- v1's **`FOCUS_SETTLE_MS = 300`**, ported as `routing::SETTLE`, over a ~250 ms poll.
  Observations settle; the core's own writes (`show`/`launch`/`home`, and the Guide
  escape's `home`) are **asserted** and apply at once. That split is v1's too.
- `input-focus take|release`, the shell's overlay declaration. A **declaration of the
  shell's own state, never a command about routing**: the core folds it into a decision
  it makes itself, and it is ignored outright unless the shell is what is on screen —
  so a shell that dies without sending `release` self-heals with no timeout and no
  liveness check. v1's `set_overlay_focus` with the failure mode removed.
- **The safe default, in one arm of one function: `Unknown` routes to the app, never to
  the shell.** An unreadable screen folds into the same answer, via the
  `SCREEN_UNREADABLE` sentinel `Compositor::on_screen_app` already fails closed with.

Two consequences worth stating out loud:

- **`shell_keys` changed meaning.** It now PINS the owner to the shell and disables
  arbitration, rather than being the only way to reach the shell route. The key name
  and its default are unchanged, so nothing on a box moves.
- **The keyboard is now created unconditionally** (whenever `enabled` is on), not only
  under `shell_keys`. It has to be: any session can be handed the shell route at any
  moment, and creating the device *at* that moment is the hotplug event
  jedwards1230/tv-shell#402 forbids.

**What phase 2 deliberately does NOT do: masking.** Each transition *quiesces* the
target it leaves, so nothing is left holding a button — but the physical release that
arrives afterwards still crosses to the new target, which never saw the press. That is
#295's shape and it is phase 3's job; `masked_keys` / `masked_axes` stay empty and the
gap is reported rather than papered over.

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

Also ships the force-quit combo, gated on v1's `presenter_owns_app` rule.

**The Guide hold shipped early, out of this phase** (jedwards1230/tv-shell#496,
`core/src/input/escape.rs`). Using phase 1 on hardware showed the ordering was wrong:
launching an app worked and there was then **no way back with the controller**, which is
the failure that makes the shell unusable, so the escape had to land before any further
navigation work. It is what §5 says it must be — the core performs the base-layer write
itself, with the shell dead, hung or never started — and it is active on **both** routes,
because the route it is most needed on is the app one. A tap still reaches whatever is on
screen (v1's behaviour, and its 500 ms threshold, ported as `[input].guide_hold_ms`);
only a hold escapes. `input-state` carries an `escape` block: `armed`, `fires`,
`failures`, `last_fire_unix_ms`.

One thing it does NOT do, reported rather than worked around: it fills neither
`masked_keys` nor `masked_axes`. Routing is still pinned, so the escape changes what is
*on screen* without changing where pad events *go* — Guide itself is buffered and never
crosses, and every other button keeps forwarding to the same target, so its real release
still arrives. There is nothing held across a change to mask. Those fields become live
when phase 2 makes the route a decision.

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

**Settled: A.** M1 (§2.3) measured the second controller directly — Moonlight opened the
idle presenter within 2 s. Option B would hand every game a phantom pad, so §7's ungrab
bullet must be amended and the reversal recorded there rather than left to diverge
quietly.

### Q2 — how do keys reach QML?

Qt 6 has no gamepad input (QtGamepad was removed), so either the core synthesises keys
or the shell grows a private pad path.

- **(a) uinput keyboard, routed by gamescope.** Matches §7, one mechanism, and is
  **required regardless** for the `keyboard` contract — Plex and a browser read no
  gamepad. Depends on gamescope routing keys correctly.
- **(b) core streams pad events over IPC; the shell injects in-process.** Immune to
  gamescope routing and to #485; makes drawer-vs-home routing an offscreen-testable
  pure decision. But it only works for our own shell and adds a second nav mechanism.

**Resolved to (a)** by §2.1: a uinput keyboard drives the shell under every condition
tested, with Moonlight running or suspended, and needs no seat tagging. The one caveat is
per-key, not per-device — `KEY_MENU` never arrives, so emitted codes must be verified.

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
| Per-pad-complete combo detection | **Ported** | Stops two pads each holding half a combo from firing it. In `escape.rs`: the hold state is per pad, and one pad cannot satisfy or cancel another one's |
| `check_grab_invariant` | **Port as assert-and-log** | Cheap; catches routing drift the report cannot |
| Stable player slots, DB-match discovery, hot join/leave | **Already in v2** | `identity.rs` / `fleet.rs` / `discovery.rs` |
| Presenter switching by create/destroy | **Redesign** | Forbidden by #402 / §7 — a hotplug event Moonlight forwards to the host. Route, never rebuild |
| `shell_focus` + `overlay_focus`, both shell-declared | **Redesign** | The core derives shell-on-screen from the base layer; the shell declares only its overlay |
| Shell-delivered escape (`intent home-hold`) | **Dropped; replaced** | Failed exactly when it was needed. The core now performs the base-layer write itself — shipped in jedwards1230/tv-shell#496, `core/src/input/escape.rs` |
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
