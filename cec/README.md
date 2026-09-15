# tv-shell-cec

The v2 AV-control daemon: HDMI-CEC over the **kernel** CEC API (`/dev/cecN`,
`pulse8-cec`), with its own config file, socket and unit. Design of record is
[`docs/V2_DESIGN.md`](../docs/V2_DESIGN.md) §8/§9/§11 and the §13 Q7 decision of
2026-09-14, which made CEC the **primary** AV control backend and demoted the IP
leg to the cold path and to recovery. This file is the crate's own map.

> ## This step is READ-ONLY. **This daemon performs no CEC transmits.**
>
> It opens the adapter, sets the physical and logical addresses, reads the
> capability set, and runs a follower receive loop. That is all of it. The
> living-room bus carries an Apple TV and a PS5 as well as the television and
> the AVR, so a stray transmit is a real-world side effect on someone's evening
> — which is why `kernel/device.rs` pins its call order by comment rather than
> leaving it to chance (see "The one call that would transmit" below).
>
> Power, input switching, volume, the health state machine and the IP recovery
> leg are later steps of the plan for jedwards1230/tv-shell#504. Their verbs are
> deliberately absent from the vocabulary rather than stubbed: an absent verb
> answers `unknown`, which is a client learning the truth. A stub answering `ok`
> would tell a caller the television had been woken when nothing happened.

## Modules

| Module | Owns |
|---|---|
| `config` | `~/.config/tv-shell/cec.toml` — a **third** file, separate from v1's `config.toml` and the core's `core.toml` (below). All-defaults on a missing file, `deny_unknown_fields` everywhere, `validate()` before any value is used. Every key here has a reader in this crate |
| `protocol` | The IPC grammar, carried over from v1 and the core unchanged in contract (§4): newline framing, 4096-byte lines, `ok` / `unknown` / `error:<msg>` / a bare JSON document. Closed vocabulary as an enum with an exhaustive match |
| `ipc` | The Unix-socket server — `LinesCodec`, one task per connection, socket bound 0600 under a tightened umask. Backend work sits behind the `AvBackend` trait so the whole request/reply surface is testable with no adapter |
| `backend` | The seam. `linux-cec` types stop here and never reach `ipc` — which is what makes a swap to `cec_linux` or to hand-rolled ioctls a contained change |
| `state` | The published snapshot: `PhysAddr`, the `Observation` tri-state, and the pure fold from a bus observation to what `av-state` reports. **The rule that `unknown` is never rendered as healthy and never as `false`** lives here |
| `kernel` | `/dev/cecN`: open, configure, read the topology back, and listen. **Linux-only** |
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

The **whole** `av-state` shape ships from this first step, with honest `null`s
for what this step cannot observe, rather than a smaller payload that changes
shape in step 5.

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

| Verb | Reply |
|---|---|
| `ping` | `ok` |
| `av-state` | one compact JSON document (below) |

Anything else is `unknown`. Both verbs are bare reads, so nothing may follow
them: `av-stateX` and `av-state 1` are different words and answer `unknown`.

```json
{"backend":"cec","device":"/dev/cec0","physAddr":"2.5.0.0",
 "physAddrConfigured":"2.5.0.0","logAddrs":["playback-device1"],
 "capabilities":["PHYS_ADDR","LOG_ADDRS","TRANSMIT"],"monitorPin":false,
 "tvPower":null,"avrPower":null,"activeSource":null,"weAreSource":null,
 "volume":null,"muted":null,"observedAt":null,"lostMessages":0}
```

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
These four were checked that way on 2026-09-14:

| Rule | Mutation | What went red |
|---|---|---|
| A verb is a whole word (`protocol::Command::parse`) | `match` arms → `starts_with` prefix tests | `protocol::word_boundaries_are_enforced`, `ipc::unknown_verbs_are_unknown` |
| `unknown` serializes as `null`, never `false`/`0` (`state::Observation`) | `serialize_none()` → `serialize_bool(false)` | 4 tests across `state` and `ipc` |
| `weAreSource` is `unknown` until the bus says who it is | the fallthrough arm → `Observation::Known(false)` | 6 tests across `state` and `ipc` |
| A `<Report Audio Status>` is only the AVR's if the AVR sent it | drop the initiator gate in `kernel::follower` | `a_third_party_report_is_not_attributed_to_the_tv_or_the_avr` |

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

- **Power and input switching** — `wake`, `standby`, `input-claim`,
  `input-release`, `input-select`, and the pure `owns_display` /
  `may_claim_active_source` gates ported from `daemon/src/cec.rs`, which must
  precede them. `standby` is gated on positive proof of ownership: a broadcast
  standby powers off every device on a shared bus.
- **Volume** — `volume up|down|mute|unmute`, `volume-state`, and system-audio-mode
  handling. A receiver ignores CEC from a non-selected input, so this must report
  a transmit that merely left the adapter honestly rather than as success.
- **Health** — the state machine over the four observed facts, and `av-health`.
- **The IP recovery leg** — the Denon/Marantz telnet client ported from the
  never-merged jedwards1230/tv-shell#191 onto typed config, `Mac::parse` and
  `magic_packet` from `daemon/src/wol.rs`, and the failover decision with
  hysteresis. Zone 2 and a cold TV wake have **no** CEC equivalent, so the IP leg
  is a capability complement on the cold path, not only a fallback.
- **Enabling the unit** — adding it to `scripts/install-v2.sh`'s `UNITS=()`.
