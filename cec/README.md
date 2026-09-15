# tv-shell-cec

The v2 AV-control daemon: HDMI-CEC over the **kernel** CEC API (`/dev/cecN`,
`pulse8-cec`), with its own config file, socket and unit. Design of record is
[`docs/V2_DESIGN.md`](../docs/V2_DESIGN.md) §8/§9/§11 and the §13 Q7 decision of
2026-09-14, which made CEC the **primary** AV control backend and demoted the IP
leg to the cold path and to recovery. This file is the crate's own map.

> ## What this daemon puts on the bus, and when
>
> **Only when a client asks.** `wake`, `standby`, `input-claim`,
> `input-release`, `input-select`, the `volume` family and `volume-state` are
> the whole transmit surface. Nothing
> fires on a timer, on a session event, or on this daemon's own initiative, and
> **starting it is still silent on the bus** — which is why `kernel/device.rs`
> pins its open sequence's call order by comment rather than leaving it to
> chance (see "The one call that would transmit" below).
>
> The living-room bus carries an Apple TV and a PS5 as well as the television
> and the AVR, so two rules are enforced by construction rather than by
> convention:
>
> - **A `<Standby>` is always ADDRESSED, never broadcast.** A broadcast standby
>   (`0x0F`) powers off every device on the bus. `action::StandbyTarget` has two
>   variants and no broadcast, so the wrong constant is not spellable.
> - **`standby` needs positive proof that this box holds the display**, and a
>   refusal transmits *nothing at all*.
>
> - **A volume action reports what the AVR did, not what left the adapter.** A
>   receiver ignores CEC from a non-selected input *after* ACKing the frame, so
>   success is judged from the AVR's own `<Report Audio Status>` read before and
>   after. An unchanged level is an `error:` naming that cause, never an `ok`.
> - **A key press is inseparable from its release.** `volume::VolumeTx::
>   KeyPressAndRelease` is one intent and `kernel::ops::wire_for` turns it into a
>   `Wire::KeyPair` carrying both messages, so a press cannot be spelled on its
>   own. An unreleased press auto-repeats on the AVR.
>
> The health state machine and the IP recovery leg are later steps of the plan
> for jedwards1230/tv-shell#504. Their verbs are deliberately absent from the
> vocabulary rather than stubbed: an absent verb answers `unknown`, which is a
> client learning the truth. A stub answering `ok` would tell a caller something
> had happened when nothing had.

## Modules

| Module | Owns |
|---|---|
| `config` | `~/.config/tv-shell/cec.toml` — a **third** file, separate from v1's `config.toml` and the core's `core.toml` (below). All-defaults on a missing file, `deny_unknown_fields` everywhere, `validate()` before any value is used. Every key here has a reader in this crate |
| `protocol` | The IPC grammar, carried over from v1 and the core unchanged in contract (§4): newline framing, 4096-byte lines, `ok` / `unknown` / `error:<msg>` / a bare JSON document. Closed vocabulary as an enum with an exhaustive match |
| `ipc` | The Unix-socket server — `LinesCodec`, one task per connection, socket bound 0600 under a tightened umask. Backend work sits behind the `AvBackend` trait so the whole request/reply surface is testable with no adapter |
| `backend` | The seam. `linux-cec` types stop here and never reach `ipc` — which is what makes a swap to `cec_linux` or to hand-rolled ioctls a contained change |
| `state` | The published snapshot: `PhysAddr`, the `Observation` tri-state, and the pure fold from a bus observation to what `av-state` reports. **The rule that `unknown` is never rendered as healthy and never as `false`** lives here |
| `ownership` | **PURE.** The tri-state display-ownership model and the two transmit gates, ported from `daemon/src/display_owner.rs` and `daemon/src/cec.rs` |
| `action` | **PURE.** A verb plus the two observed addresses becomes either a plan of messages or a refusal that transmits nothing. Every gate runs before any message is built |
| `volume` | **PURE.** The volume/mute sequence: system-audio mode first, an inseparable press/release pair, and success judged from the AVR's own report. The bus is a one-method `VolumeBus` trait, so the whole sequence runs in CI with no adapter |
| `kernel` | `/dev/cecN`: open, configure, read the topology back, listen, and transmit a plan. **Linux-only**. `kernel/ops.rs` holds the pure `CecTx` → `linux-cec` `Message` table, so CI covers the whole message set with no adapter |
| `notify` | `sd_notify` — `READY=1` for the unit's `Type=notify`, `WATCHDOG=1` for its `WatchdogSec=`. Transport only; it decides nothing |

`../core/units/tv-shell-v2-cec.service` is this daemon's unit. It lives beside
the other v2 units because the session target that `Wants=` it does, and it is
**shipped but not installed**: `scripts/install-v2.sh`'s `UNITS=()` array does
not carry it yet.

## The rules this code enforces

### `unknown` is a first-class value, and it is never rendered as healthy

Every field of `av-state` is an `Observation`: the value when something observed
it, JSON `null` when nothing has. A daemon reporting `weAreSource: false` because
nothing had told it otherwise would be making a claim it cannot support, and a
consumer cannot tell that apart from a genuine "someone else holds the display".

`daemon/src/display_owner.rs` already articulates why a boolean with a default
cannot work: **the fail-safe direction inverts depending on the consumer.** A
caller deciding whether to send `<Standby>` wants "unknown ⇒ do not"; a caller
deciding whether to claim the display wants "unknown ⇒ go ahead". Neither default
is right for both, so this daemon publishes the tri-state and each consumer picks
its own safe side. That reasoning is ported deliberately; it is the best thing in
the v1 CEC code.

The **whole** `av-state` shape shipped from the first step, with honest `null`s
for what it could not observe. Every field of it is genuinely observable now:
`volume` and `muted` are folded from a `<Report Audio Status>` — whether the
receive loop overhears one or a `volume` action reads one back.

**A volume of `0` and "we do not know the volume" are different wire values.**
CEC's 7-bit audio-volume field defines only `0..=100` as levels and reserves
`0x7F` for *"audio volume status unknown"*, which is exactly what a receiver out
of system-audio mode sends. `volume::level_observation` is the one place that
maps it, and it maps it to `unknown` — never to a clamped number, and never to
`0`. The mute flag is one bit and always means something, so it stays known even
when the level does not.

### The gates: positive proof one way, proof-of-harm the other

Two predicates, deliberately asymmetric, ported from `daemon/src/cec.rs`:

- **`owns_display`** (the standby gate) is true only when the last observed
  claim was **ours**. "Never seen a claim", "someone else claimed it" and "our
  own address is undeterminable" all yield false. Suspending this box must not
  be able to power off a television someone is watching on another input, and
  that needs proof, not the absence of counter-evidence.
- **`may_claim_active_source`** (the wake/claim gate) skips only on **positive
  proof that a different real device holds the screen**. Requiring
  `owner == ours` would make the claim a permanent no-op — if we already owned
  the display there would be nothing to claim.

It is not enough to derive the second from the tri-state. A real *other* owner
must still win when our own address is unknown, and `classify` calls that case
`unknown`, which would permit the claim. The mutation table below has a row for
exactly that, because it is the one difference a plausible refactor erases.

### Every field is "what was observed and when", never an inferred verdict

v1's `cec-health` inferred adapter health from the outcome of *our own
transmits*, and the Ansible watchdog above it then inferred it a **second** time
from IPC reachability. A deliberately stopped daemon therefore read as a wedged
adapter and got "recovered" three times, and the UI copy for the resulting
`adapter_open_failed` sent an operator after a cable for a software conflict.

Nothing here infers. `av-state` reports observations with timestamps; the one
liveness judgement in the crate is the watchdog's, and it is a pure ioctl on our
own file descriptor (below).

### The one call that would transmit, and why the order is pinned

`set_osd_name` **must** be called before `set_logical_addresses`. `linux-cec`
sends a `<Set OSD Name>` message to the TV if a logical address has already been
claimed, and does not if one has not. Before the logical addresses are set we are
`Unregistered`, so the call is pure configuration. The crate's own docs require
the same order for a different reason — the kernel only advertises the name on
query if it was set first — so the two agree, but the transmit is the one that
matters here.

**The one bus interaction that is not ours** is the kernel's:
`CEC_ADAP_S_LOG_ADDRS` makes the *kernel* poll the bus to allocate a logical
address, as every CEC device does when it attaches. No message of this daemon's
is transmitted. Said out loud, because "no transmits" should mean what it says.

### The physical address is explicit, and both values are published

`phys_addr` is a `cec.toml` key and is **not** auto-derived.
`cec-ctl --phys-addr-from-edid` reads the EDID of the connector the adapter sits
on, and this adapter sits on a *different* HDMI input from the video leg —
`card1-HDMI-A-1`'s EDID is readable but belongs to the AVR's video path, so it
is the wrong port's answer.

**`2.5.0.0` is UNVERIFIED against the current rack.** It is the pre-2026-08-07
value and the rack has changed since. A wrong value fails *silently*: a later
`<Active Source>` addresses a port that does not exist and nothing on the bus
complains. So the daemon logs the value it set alongside what
`CEC_ADAP_G_PHYS_ADDR` reads back, warns loudly on a mismatch, and `av-state`
publishes `physAddrConfigured` and `physAddr` side by side. Verify against
`cec-ctl -d /dev/cec0 --show-topology` once the device exists.

### Capabilities are read, never assumed

`get_capabilities()` runs before anything is configured. It gates the two calls
that need a capability (`CEC_CAP_PHYS_ADDR`, `CEC_CAP_LOG_ADDRS`) with an error
naming the capability rather than an opaque ioctl errno, and the full flag set
reaches `av-state` verbatim — read off the bitflags, so a flag this crate has
never heard of is still reported rather than dropped.

**Whether `pulse8-cec` implements `CEC_CAP_MONITOR_PIN` is unverified** and could
not be checked without a device. Nothing here assumes the pin monitor exists.
It matters because the pin monitor is the one signal that separates "the bus is
quiet because everything is off" from "our adapter has stopped hearing";
`av-state.monitorPin` reports what was actually found, and step 6's `av-health`
names which signal is in force.

### The watchdog answers one question, and systemd owns the response

The unit is `Type=notify` with `WatchdogSec=30s`. Both halves are implemented
here, because shipping either directive without its message ships a unit that
never comes up or one that kills itself every 30 s.

`READY=1` is sent **last** — after the socket is bound and the receive loop is
running — so systemd never reports the daemon as serving while a client
connecting on that promise would get `ENOENT`.

`WATCHDOG=1` is fed at half the interval, and only while `CEC_ADAP_G_CAPS`
round-trips. That is a pure ioctl on our own file descriptor: it touches the bus
not at all, so probing liveness has no side effect on anyone's television. When
it stops round-tripping the feed simply **stops** — the daemon does not kill
anything, because systemd's `WatchdogSec=` already owns the response and a second
mechanism with restart authority over the same process is §9's "only one
supervisor" rule being broken.

## Why a new crate, not an evolution of `daemon/`

§13 Q12's precedent, and the same reason the core needed it: v1's `config.toml`
root is `#[serde(deny_unknown_fields)]`, so a v2 table added to it makes the
**v1 daemon abort at startup**, and the symptom presents as "v1 is broken". The
same argument applies to `core.toml`. §11 makes it a rule at every shared layer,
so this daemon has a third config file (`cec.toml`), a third socket
(`tv-shell-v2-cec.sock`) and its own unit name (`tv-shell-v2-cec.service`),
sharing none of them with v1 or the core.

## Why `linux-cec`, and the two caveats

[`linux-cec`](https://gitlab.steamos.cloud/holo/linux-cec/) 0.2.1 is **Valve's**,
on a design that adopts SteamOS shapes everywhere else, and `linux-cec-sys`
declares only `bitflags` + `nix` — **pre-generated bindings, no bindgen and no
libclang at build time**. That preserves the workspace's "a default build needs
no system C libraries" invariant that `daemon/Cargo.toml`'s `cec` feature fought
for, and it is why this crate's CI job needs no apt step at all.
`cec-rs`/`libcec-sys` are deprecated upstream (2026-09-03) and are dropped by
§13 Q7 regardless.

**That invariant is asserted now, not assumed.** It previously held only by
accident: the `cec` job installs no apt packages, so a future `linux-cec` bump
that switched to bindgen would have stayed green on a GitHub runner that happens
to carry libclang, and failed on the deploy box. `scripts/assert-no-system-c.sh`
closes it from both sides — a `cargo tree --invert` ban on the build-time
tooling (`bindgen`, `clang-sys`, `cmake`, `pkg-config`, `libcec-sys`,
`libudev-sys`, …) and an **allowlist** `ldd` over the built binary, which may
link nothing beyond the base C runtime (`libc`, `libm`, `libgcc_s`, the loader,
`linux-vdso`). The allowlist direction is deliberate: the `cec-mcp` job's
`libcec|libp8-platform` grep is a denylist and would not notice a new,
unanticipated system library. `cc` is **not** banned — it is genuinely absent
from this graph, but it is a build-dep of plenty of harmless crates, and a gate
that fires on changes which do not threaten the invariant is a gate people learn
to route around; the script says so in place.

Two caveats, stated rather than buried:

- It declares `nix ^0.31` while the lockfile already carries `nix 0.29`
  transitively, so the tree holds two `nix` copies. Harmless; `cargo tree -d`
  grows an entry.
- Its licence is **LGPL-2.1-or-later**, GPL-3.0-compatible via the "or-later"
  clause — the same class of check as the `cec-rs` GPL-2.0 note in
  jedwards1230/tv-shell#88.

It is 0.2.1 with one author and is not widely exercised. That is exactly what the
`backend::AvBackend` seam is for: keep the crate's types out of `ipc` and a swap
to `cec_linux`, or to a hand-rolled `<linux/cec.h>` transcription, stays
contained.

## Config

`~/.config/tv-shell/cec.toml`, overridable with `$TV_SHELL_CEC_CONFIG`. A missing
file is all-defaults.

```toml
[device]
path      = "/dev/cec0"   # the CEC device node
phys_addr = "2.5.0.0"     # a.b.c.d — explicit, never derived; see above
osd_name  = "tv-shell"    # ≤14 ASCII bytes (the CEC cap), refused if longer
```

The socket is `$TV_SHELL_CEC_SOCK`, else `/run/user/<uid>/tv-shell-v2-cec.sock`,
bound `0600`.

## IPC

**This is the reference for the v2 AV verbs.** `docs/IPC_PROTOCOL.md` documents
v1's `tv-shell-input.sock` and its `cec-*` verbs, which this daemon does not
speak and which do not speak this one's; §11 keeps the two sockets, grammars and
config files separate on purpose, so folding v2's verbs into that document would
suggest a shared surface that does not exist.

| Verb | Reply | Messages |
|---|---|---|
| `ping` | `ok` | — |
| `av-state` | one compact JSON document (below) | — |
| `wake` | `ok` / `refused:` / `error:` | `<Image View On>` → TV, then `<Active Source>` broadcast, then a `<Give Device Power Status>` read-back |
| `standby` | `ok` / `refused:` / `error:` | `<Standby>` → TV, then → Audio System. **Never broadcast.** Gated on `owns_display` |
| `input-claim` | `ok` / `refused:` / `error:` | `<Active Source>` broadcast |
| `input-release` | `ok` / `error:` | `<Inactive Source>` → TV |
| `input-select <phys-addr>` | `ok` / `error:usage:` / `error:` | `<Set Stream Path>` broadcast |
| `volume up\|down` | `ok` / `refused:` / `error:` / `error:usage:` | `<Give System Audio Mode Status>` (+ `<System Audio Mode Request>` if off) → `<Give Audio Status>` → `<User Control Pressed>[Volume Up/Down]` **and** `<User Control Released>` → `<Give Audio Status>` |
| `volume mute\|unmute` | as above | the same sequence, with `<User Control Pressed>[Mute]` — and **no key at all** when the AVR already reports the state asked for |
| `volume-state` | one compact JSON document (below) | `<Give Audio Status>`, falling back to the last one overheard |

Anything else is `unknown`. Every verb but `input-select` and `volume` is a bare
read or bare action, so nothing may follow it: `av-stateX`, `av-state 1`,
`standby now` and `volume-state 1` are all `unknown`. The two that take a body
take exactly one word, and a missing, malformed or extra body is
`error:usage: …` — **never a silent default**, and never `unknown` (the client
knows the verb; it got the call wrong). `input-select` because a
`<Set Stream Path>` naming a port that does not exist fails *silently* on the
bus; `volume` because a typo that read as `ok` would report a change nobody
asked for.

### What `volume` does, and what the bus can actually express

Three things, in this order, and each one is load-bearing:

1. **System audio mode first.** A receiver that has dropped out of it **silently
   ignores** volume UI commands — it ACKs the frame and does nothing. So every
   volume action reads `<Give System Audio Mode Status>` and sends
   `<System Audio Mode Request>` when the answer is no or absent.
2. **The press and its release are one intent.** An unreleased
   `<User Control Pressed>` auto-repeats on the AVR and the volume runs away.
   `Wire::KeyPair` carries both messages in one value, and the release is
   transmitted **even when the press was NAKed** — a spurious NAK would otherwise
   leave a key held down, while an extra release is a no-op at every receiver.
3. **Success is the AVR's `<Report Audio Status>`, read before and after.** An
   unchanged level is reported as an `error:` naming the likely cause ("a
   receiver ignores CEC from a non-selected input…"), except at the ends of the
   0-100 scale, where not moving is correct. If the AVR will not report at all,
   that is an `error:` too: the transmit is reported honestly rather than as
   success.

**`mute` and `unmute` both press CEC's `Mute` UI code (`0x43`), which is a
TOGGLE**, and this daemon does not pretend otherwise. The absolute codes —
`Mute Function` (`0x65`) and `Restore Volume Function` (`0x66`) — are optional in
the specification and widely unimplemented, so an `unmute` built on them would
silently do nothing on the receivers that lack them. **An idempotent unmute is
therefore not expressible as a single message on this bus.** What is expressible,
and what this does, is the read-first shape the rest of the daemon already uses:
`<Give Audio Status>`, then toggle **only if the AVR is not already in the state
asked for**. The verbs are idempotent even though the message is not — repeated
calls converge, and a call that finds the state already correct transmits no key
and answers `ok` on the AVR's own evidence. When the mute state cannot be read at
all, **nothing is transmitted**: a blind toggle could mute a television somebody
asked to unmute.

**`volume` does not claim the display first, deliberately.** `wake` and
`standby` do, because §8's ordering constraint applies to them and they are
lifecycle actions. Yanking the television to this box because somebody nudged the
volume is a bigger side effect than the verb asks for, so `volume` sends the
non-destructive half (`<System Audio Mode Request>`, which routes audio without
touching the video input) and, when the AVR ignores the command anyway, **says
so**. A caller that wants the input as well has `input-claim`, which says what it
does.

```json
{"level":37,"muted":false,"source":"avr-report","observedAt":1757800000000}
```

`volume-state`'s `source` is `avr-report` when the AVR answered a query made just
now and `observed` when the value is the last one the receive loop overheard;
both are `null`, along with `level` and `muted`, when nothing is known. It is the
one read verb that **may touch the bus** — `av-state` answers from the cached
snapshot precisely so it still answers when the device is the thing being
diagnosed, while "what is the AVR's volume" is a question only the AVR can
answer.

### `refused:` — the one addition to the reply grammar

`refused:<why>` means **the daemon deliberately did not act, nothing is broken,
and zero messages reached the bus.** It is not `ok` and it is not `error:`,
because neither is true and both mislead:

- v1 replied `ok` to a skipped transmit, on the reasoning that a skip fails at
  nothing. That makes "we deliberately declined to power off your television"
  read exactly like "we powered off your television", and no later observation
  separates them — on a shared bus the set may well go off for someone else's
  reason.
- `error:` would say a fault occurred and send an operator after a wedged
  adapter. That is the `cec-health` failure shape this design exists to remove,
  one layer down.

The zero-transmit half is not a convention: every gate in `action.rs` runs
before any message is built, and a test asserts over the whole action set that a
refusal carries no plan.

```json
{"backend":"cec","device":"/dev/cec0","physAddr":"2.5.0.0",
 "physAddrConfigured":"2.5.0.0","logAddrs":["playback-device1"],
 "capabilities":["PHYS_ADDR","LOG_ADDRS","TRANSMIT"],"monitorPin":false,
 "tvPower":"on","avrPower":null,"activeSource":"2.5.0.0","weAreSource":true,
 "displayOwnership":{"state":"owned-by-us","owner":"2.5.0.0","ours":"2.5.0.0",
                     "changedAt":1757800000000,"everObserved":true},
 "volume":37,"muted":false,"observedAt":1757800000000,"lostMessages":0}
```

`displayOwnership` is published **beside** `weAreSource`, not instead of it, and
it is the field a consumer whose fail-safe direction is the opposite one needs
(see below). `state` is `owned-by-us` / `owned-by-other` / `unknown`;
`everObserved` distinguishes "we are listening and this bus never announces
ownership" from "we are not listening"; `changedAt` is how long the current
owner has held the display, **not** a staleness measure — CEC ownership is
edge-driven and a claim heard six hours ago is still the current truth.

**Callers must bound their connect and read timeouts.** The unit's isolation from
the session is topological — the only edge is a `Wants=` from the session target,
which carries no ordering and no failure propagation — but the runtime hazard is
a caller blocking on this socket, and that rule lives in the callers (the shell's
Session QAM, the panel), not here.

## Build, test & lint

```bash
cargo fmt --check -p tv-shell-cec
cargo clippy -p tv-shell-cec --all-targets -- -D warnings
cargo build --release -p tv-shell-cec
cargo test -p tv-shell-cec
```

**No device is needed, and none is faked into existence.** Every decision lives
in a pure module beside the I/O — `config` parsing, the wire grammar, the
`PhysAddr` form, the observation fold, the `unknown`-is-never-`false` rule — and
the IPC surface runs end-to-end against a stand-in backend over a real Unix
socket. The `cec` job in `.github/workflows/rust.yml` runs exactly the four
commands above.

There is deliberately **no `#[ignore]`-gated device test yet**. An `#[ignore]`d
test wired into no job defends nothing (jedwards1230/tv-shell#469), and
`continue-on-error` reports green on a skip as readily as on a failure. The
device-backed lane lands with something that needs a device to assert.

### Rule-defending tests, and how to check they still defend anything

A green suite proves nothing until the rule is inverted and the suite goes red.
Every row below was checked that way on 2026-09-14:

| Rule | Mutation | What went red |
|---|---|---|
| A verb is a whole word (`protocol::Command::parse`) | `match` arms → `starts_with` prefix tests | `protocol::word_boundaries_are_enforced`, `ipc::unknown_verbs_are_unknown` |
| `unknown` serializes as `null`, never `false`/`0` (`state::Observation`) | `serialize_none()` → `serialize_bool(false)` | 4 tests across `state` and `ipc` |
| `weAreSource` is `unknown` until the bus says who it is | the fallthrough arm → `Observation::Known(false)` | 6 tests across `state` and `ipc` |
| A `<Report Audio Status>` is only the AVR's if the AVR sent it | drop the initiator gate in `kernel::follower` | `a_third_party_report_is_not_attributed_to_the_tv_or_the_avr` |
| `owns_display` needs positive proof | invert it (`!matches!(…, OwnedByUs)`) | **8 tests** across `ownership`, `action`, `ipc` and `kernel::ops` |
| The claim gate is asymmetric with the standby gate | make it `owns_display(owner, ours)` | **10 tests** across `ownership`, `action`, `ipc` and `kernel::ops` |
| …and asymmetric with the tri-state too | derive it as `classify(..) != OwnedByOther` | `may_claim_active_source_yields_only_to_a_known_other_owner` — the one row where our own address is unknown |
| `standby` is gated on ownership | delete the `owns_display` check from `action::plan` | `standby_refuses_and_transmits_nothing_without_positive_proof` + the two `ipc` refusal tests |
| A `<Standby>` is never broadcast | return `LogicalAddress::Broadcast` from `standby_destination` | `a_standby_is_always_addressed_and_never_broadcast` + 2 more in `kernel::ops` |
| A malformed `input-select` body is a usage error | fall back to a default address | `a_malformed_input_select_body_is_a_usage_error` + the `ipc` twin |
| The ownership timestamp moves only on a real change | drop the equality guard in `store_active_source` | `the_ownership_timestamp_moves_only_on_a_real_change` |
| `input-release` sends `<Inactive Source>`, not `<Active Source>` | swap the message (what `set_active_source(None)` actually does — see below) | `every_intended_transmit_maps_to_its_message_and_destination` |
| A press is always followed by its release | return early from `DeviceVolumeBus::perform` after the press | **3 tests** in `kernel::ops`, incl. the NAKed-press row |
| Success is judged from the read-back, not the transmit | `perform_level` → `Done` without consulting `judge_level` | **4 tests** across `volume`, `kernel::ops` and `ipc` |
| An unreadable volume is `unknown`, never `0` | `level_observation` → `Known(0)` out of range | **3 tests** across `volume`, `kernel::follower` and `kernel::ops` |
| System-audio mode is asked about first | delete the `ensure_system_audio_mode` call from `volume::execute` | **4 tests** across `volume`, `kernel::ops` and `ipc` |
| `mute`/`unmute` converge instead of toggling | `mute_step` → always `Toggle` | **4 tests** across `volume` and `ipc` |
| A missing `volume` argument is a usage error | default it to `up` | `a_missing_or_unknown_volume_argument_is_a_usage_error` + the `ipc` twin |
| An AVR report returning to "volume unknown" clears the old level | keep the previous level when the new one is unknown | `an_avr_that_does_not_know_its_volume_returns_the_level_to_unknown` |

**Every ownership state the gates are tested against is reachable from the real
receive path**, and `the_receive_path_can_produce_every_ownership_verdict` walks
that chain once — `linux-cec` `Message` → `observation_for` → the fold → the
verdict — rather than poking a field. That includes `f.f.f.f`: the payload of an
`<Active Source>` is folded verbatim, so the unaddressable case is something the
bus can produce and not only something a unit test can construct.

### One correction to the plan, found by reading the crate

The plan for jedwards1230/tv-shell#504 lists `set_active_source(None)` as the
release primitive ("→ `InactiveSource`"). **It is not.** `linux-cec` 0.2.1
`device.rs:855` falls back to the device's *own* physical address and sends
`<Active Source>`, i.e. it **claims** the display. Using it for `input-release`
would have done the exact opposite of the verb. `kernel/ops.rs` constructs
`Message::InactiveSource` explicitly, addressed to the TV as the specification
directs, and the mutation row above pins it.

A fifth lives in `core/`: hard-coding `/opt/tv-shell/bin/tv-shell-cec` into the
unit's `ExecStart` fails `the_committed_units_name_no_absolute_install_path`.

### That check is automated now — `cargo-mutants`

Doing it by hand has failed twice. Three tests in jedwards1230/tv-shell#514 were
vacuous until a hand-run mutation caught them, and fixing them exposed a real
bug: `Rx` reset the whole tx-error run, which made the "no rx traffic in the
same window" condition dead code. Separately, inverting a crate's central rule
in jedwards1230/tv-shell#462 left all 88 tests passing.

```bash
cargo install cargo-mutants --locked
./scripts/run-cec-mutants.sh          # from the workspace root
```

Scope is in [`.cargo/mutants.toml`](../.cargo/mutants.toml) and is **glob-based
on purpose**: only `state.rs` and `protocol.rs` exist today, while
`ownership.rs`, `action.rs`, `health.rs` and `failover.rs` arrive with later PRs
in the stack, so the gate's coverage grows as each lands rather than being wrong
now and right later. The I/O modules (`kernel/`, `ipc.rs`, `main.rs`,
`notify.rs`, `backend.rs`, `config.rs`) are excluded and *named* as excluded —
they are exercised through stand-ins, so a mutant surviving there measures the
fake rather than the rule.

Surviving mutants are **reported, not enforced**, until the baseline is agreed.
The job still fails hard when the run examined nothing, generated no mutants, or
produced output that could not be parsed — "we could not measure" must never
look like "we measured and it was fine" (jedwards1230/tv-shell#469).

## Not yet here

Each of these lands with the module that reads it, never ahead of it:

- **Health** — the state machine over the four observed facts, and `av-health`.
- **The IP recovery leg** — the Denon/Marantz telnet client ported from the
  never-merged jedwards1230/tv-shell#191 onto typed config, `Mac::parse` and
  `magic_packet` from `daemon/src/wol.rs`, and the failover decision with
  hysteresis. Zone 2 and a cold TV wake have **no** CEC equivalent, so the IP leg
  is a capability complement on the cold path, not only a fallback.
- **Enabling the unit** — adding it to `scripts/install-v2.sh`'s `UNITS=()`.
