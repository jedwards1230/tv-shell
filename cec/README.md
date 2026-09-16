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
> A shared bus may carry other playback devices (a streaming box, a console)
> besides the television and the AVR, so two rules are enforced by
> construction rather than by convention:
>
> - **A `<Standby>` is always ADDRESSED, never broadcast.** A broadcast standby
>   (`0x0F`) powers off every device on the bus. `action::StandbyTarget` has two
>   variants and no broadcast, so the wrong constant is not spellable.
> - **`standby` needs positive proof that this box holds the display**, and a
>   refusal transmits *nothing at all*.
>
> - **A volume action reports what the AVR did, not what left the adapter.** A
>   receiver *may* ignore CEC from a non-selected input after ACKing the frame —
>   vendor-specific, and measured **not** to bite on a Denon AVR-X1700H on
>   2026-09-16 — so success is judged from the AVR's own
>   `<Report Audio Status>` read before and after. An unchanged level is an
>   `error:` naming that cause, never an `ok`. The readback rule stands
>   regardless: it is what makes the answer true on a receiver that *does*
>   ignore it.
> - **A key press is inseparable from its release.** `volume::VolumeTx::
>   KeyPressAndRelease` is one intent and `kernel::ops::wire_for` turns it into a
>   `Wire::KeyPair` carrying both messages, so a press cannot be spelled on its
>   own. An unreleased press auto-repeats on the AVR.
>
> - **Health is observed, never inferred, and it is a READ with no bus traffic.**
>   `av-health` publishes four facts and the tri-state derived from two of them.
>   Probing the fd is a pure ioctl; asking for health puts nothing on the bus and
>   answers from recorded facts, so the verb still answers when the device is the
>   thing being diagnosed.
>
> **The IP leg is a capability complement first and a failover second.** Two
> things have no CEC expression at all — a receiver's **Zone 2** (`Z2OFF`) and a
> **cold wake of a television at mains standby** — so those steps run *before*
> the CEC steps of `wake` and `standby`, with a perfectly healthy bus. On top of
> that, `failover` decides the warm path: which backend carries an action when
> the adapter stops answering. `backend` publishes that decision and
> `backend-pin` overrides it. See "The IP leg" below.

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
| `health` | **PURE.** The four observed facts, the tri-state derived from **two** of them, and the single rule the `WATCHDOG=1` feed is gated on. `unknown` is a first-class state and is never rendered as healthy |
| `volume` | **PURE.** The volume/mute sequence: system-audio mode first, an inseparable press/release pair, and success judged from the AVR's own report. The bus is a one-method `VolumeBus` trait, so the whole sequence runs in CI with no adapter |
| `failover` | **PURE.** Which backend is authoritative on the warm path, from the *observed* health plus a transmit-failure rule, with hysteresis on both edges and a reason for every change. Its four thresholds live in `cec.toml` and every one of them is consumed |
| `ip` | The IP leg: `ip/wol.rs` (a magic packet, and nothing else) and `ip/avr.rs` (Denon/Marantz ASCII telnet). All I/O is behind the one-trait `IpWire` seam, so **no test in this crate opens a socket or sends a packet** |
| `kernel` | `/dev/cecN`: open, configure, read the topology back, listen, and transmit a plan. **Linux-only**. `kernel/ops.rs` holds the pure `CecTx` → `linux-cec` `Message` table, so CI covers the whole message set with no adapter |
| `notify` | `sd_notify` — `READY=1` for the unit's `Type=notify`, `WATCHDOG=1` for its `WatchdogSec=`. Transport only; it decides nothing |

`../core/units/tv-shell-v2-cec.service` is this daemon's unit. It lives beside
the other v2 units because the session target that `Wants=` it does, and since
the cutover it is **installed**: it is the fourth entry in
`scripts/install-v2.sh`'s `UNITS=()`, and that script now builds and installs
this binary alongside `tv-shell-core`.

Installing it on a box with no adapter is a **silent, correct skip** — the unit
carries `ConditionPathExists=/dev/cec0`, so systemd records the unmet condition
and starts nothing, and the session target only `Wants=` it in any case. On the
deploy host `/dev/cec0` does not exist yet; creating it is an operator step in
`jedwards1230/homelab-ansible#338`, taken with nobody at the television.

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

`phys_addr` is a `cec.toml` key and is **not** auto-derived. The kernel cannot
derive it either: `CEC_ADAP_G_CONNECTOR_INFO` answers `None` on this adapter, so
it has no DRM connector to read an EDID from and the value *must* be set by hand.

**`2.5.0.0` is confirmed against the live installation, measured 2026-09-16 — and
claiming it is a deliberate fiction, not a bug.** `2.5.0.0` is the **video
leg's** address, read from `card1-HDMI-A-1`'s EDID; it is not the AVR port the
adapter's own HDMI plug sits in. That is the point: `<Active Source>` and the
volume traffic should refer to the input that actually shows picture, which is
the video leg. A future reader should not "correct" this to the adapter's port.

It does not collide. On a bus that also carries other playback devices, the
negotiated *logical* address is not the one a first playback device would get:
on the measured bus we landed on **Playback Device 2 (LA 8), not LA 4**, because
another playback device already held LA 4. So nothing may assume a particular
logical address — the daemon reads back what it was actually given. Successful
negotiation on a live bus is itself proof that tx and rx work.

A wrong value would fail *silently*: a later `<Active Source>` addresses a port
that does not exist and nothing on the bus complains. So the daemon logs the
value it set alongside what `CEC_ADAP_G_PHYS_ADDR` reads back, warns loudly on a
mismatch, and `av-state` publishes `physAddrConfigured` and `physAddr` side by
side.

### Capabilities are read, never assumed

`get_capabilities()` runs before anything is configured. It gates the two calls
that need a capability (`CEC_CAP_PHYS_ADDR`, `CEC_CAP_LOG_ADDRS`) with an error
naming the capability rather than an opaque ioctl errno, and the full flag set
reaches `av-state` verbatim — read off the bitflags, so a flag this crate has
never heard of is still reported rather than dropped.

**`CEC_CAP_MONITOR_PIN` is measured ABSENT on this adapter** — the Pulse-Eight
USB-CEC, firmware `000c`, read live on 2026-09-16: `CEC_ADAP_G_CAPS` reports
`0x23f` = Physical Address, Logical Addresses, Transmit, Passthrough, Remote
Control Support, Monitor All, Reply Vendor ID. No pin monitor.

That is a fact about *this* adapter, not about the code: the capability is still
read at open and never assumed, because other adapters differ and the runtime
check is what makes the degradation correct anywhere. It matters because the pin
monitor is the one signal that separates "the bus is quiet because everything is
off" from "our adapter has stopped hearing"; `av-state.monitorPin` reports what
was actually found, and `av-health`'s `reason` names which signal is in force.
**`Monitor All` is the fallback vantage** this hardware does have — passive
visibility of all bus traffic without transmitting — but it is not fact 3: it
sees frames, not line level, so a bus with nothing to say still looks the same
as a deaf adapter.

### Health is four observed facts, not one inferred verdict

v1 inferred adapter health from **the outcome of our own transmits**
(`daemon/src/cec.rs:14-18`), and the Ansible CEC watchdog then inferred it a
*second* time from IPC reachability. The v1 daemon does not unlink its socket on
shutdown, so a stale node outlived every stop, every read-only probe timed out
into "unreachable", and three of those "recovered" a daemon that was never
broken. `av-health` replaces that with what was **observed**, and when:

| Fact | How it is observed | Where it lands |
|---|---|---|
| 1. The fd is alive | `CEC_ADAP_G_CAPS` round-trips — a pure ioctl, **no bus traffic** | the verdict, and the watchdog gate |
| 2. The adapter has an address | `CEC_ADAP_G_PHYS_ADDR` valid **and** `CEC_ADAP_G_LOG_ADDRS` non-empty; `PollResult::StateChange` says it may have changed | the verdict |
| 3. The bus is physically moving | `PollResult::PinEvent` — line level, observed passively | `busActivityMs`, as an age |
| 4. Last accepted tx / last rx | recorded by the transmit path and the receive loop | `lastTxOk` / `lastRxMs`, as ages |

**Only facts 1 and 2 derive the verdict.** Facts 3 and 4 are published as ages
and judged by nobody, because in this deployment silence is not evidence:
everything can be switched off, and without the pin monitor there is no way to tell that
apart from a deaf adapter. Degrading on a quiet bus would be inventing exactly
the kind of verdict this module exists to stop inventing — v1's mistake with the
sign flipped. If the pin monitor ever comes into force, fact 3 is what makes "we
have stopped hearing" a real observation, and that is when it may sharpen the
verdict.

**`CEC_CAP_MONITOR_PIN` is absent on the deployed adapter, and the reply says
so.** It is read at open rather than assumed — other adapters may have it — and
`reason` always names which bus-liveness signal is in force; on the reference
deployment that is `last-heard ages`, permanently, because the capability is not
there. Even where the capability is present the daemon does **not** enter a
pin-monitoring mode: `FollowerMode` is one value, so a monitor mode *replaces*
`FollowerMode::Enabled` and the receive loop would stop folding `<Active
Source>` — and the kernel gates the monitor modes on `CAP_NET_ADMIN`, which a
`systemd --user` unit does not have. So fact 3 degrades to fact 4, `reason` says
which, and `PinMonitor::InForce` is set only by an actually-observed pin event.

The tri-state itself is `daemon/src/display_owner.rs`'s argument again: the
fail-safe direction **inverts per consumer**, so the daemon publishes
`healthy` / `degraded` / `unknown` plus the observations behind it, and each
consumer picks its own safe side. **`unknown` is never rendered as healthy** — in
this crate, and in the panel page that reads it.

### The watchdog answers one question, and systemd owns the response

The unit is `Type=notify` with `WatchdogSec=30s`. Both halves are implemented
here, because shipping either directive without its message ships a unit that
never comes up or one that kills itself every 30 s.

`READY=1` is sent **last** — after the socket is bound and the receive loop is
running — so systemd never reports the daemon as serving while a client
connecting on that promise would get `ENOENT`.

`WATCHDOG=1` is fed at half the interval, and only while **fact 1** holds:
`Health::should_feed_watchdog` is `fd_alive` and nothing else. It is deliberately
**not** gated on the derived verdict — a lost address is `degraded`, and
restarting the daemon does not give an HDMI topology back, so feeding on the
verdict would make every television standby a restart loop, which is v1's
"recovered a daemon that was never broken" with a new mechanism. When fact 1
stops holding the feed simply **stops**: the daemon does not kill anything,
because systemd's `WatchdogSec=` already owns the response and a second mechanism
with restart authority over the same process is §9's "only one supervisor" rule
being broken.

**That is what RETIRES the Ansible CEC watchdog** rather than merely disabling
it. No polling script, no `cec-health` probe with bus side effects, no second
supervisor. The watchdog timer is already `disabled`/`inactive` on the deploy box
(verified read-only 2026-09-14, and `htpc_cec_watchdog_active` derives from
`htpc_boot_session`), so nothing needs stopping — it must simply never be
re-enabled.

### Every caller must bound its own timeout

The unit's isolation from the session is topological: the only edge is the
session target's `Wants=`, which carries no ordering and no failure propagation.
**That isolation is only real if every caller of this socket uses a bounded
connect+read timeout and renders a degraded state on expiry**, because the real
hazard is not systemd — it is a caller blocking on a wedged backend, which is how
that backend eventually reaches the television. The rule cannot be enforced from
inside this daemon; it lives in the callers. The panel's `/devices/av` page is
the first of them (800 ms, `panel/src/pages/av.rs`), and it has a test that
stands up a socket which accepts and never replies.

**The bounds are sane against measured round trips (reference deployment,
2026-09-16).** On the live bus a `<Give Audio Status>` was answered in **18 ms** and a
`<Give System Audio Mode Status>` in **39 ms**. Against that:

| Bound | Value | Verdict |
|---|---|---|
| `kernel::ops::REPLY_TIMEOUT` | 1000 ms | ~25x the slowest measured reply. Not a free choice anyway — `CEC_TRANSMIT` coerces anything larger, and zero, to one second |
| `[failover] tx_error_window_ms` | 10 000 ms | Three orders above a round trip; a window this wide cannot mistake one slow reply for a wedge |
| `[failover] fail_after_ms` | 5 000 ms | Same |
| `[failover] recover_after_ms` | 10 000 ms | Same |
| The panel's caller bound | 800 ms | The tightest of the set, and still ~20x the slowest measured reply |

No value is changed on the strength of these numbers. The measurement narrows
nothing: every bound already sits orders of magnitude above the observed
latency, and they are sized for a *wedged* adapter — an absence of any reply —
not for a slow one.

## The IP leg

### Q7's "IP only when CEC is unavailable" is too narrow, and this crate models the complement

V2_DESIGN §13 Q7 describes the IP leg as used "when the CEC bus is unavailable or
the adapter has wedged" — purely a failover. The never-merged
jedwards1230/tv-shell#191's problem statement documents two things **CEC
physically cannot do at all**:

1. **AVR Zone 2 is not CEC-addressable.** `Z2OFF` has no CEC equivalent
   whatsoever. If Zone 2 is wanted, telnet runs on *every* standby, with a
   perfectly healthy bus. `ip::avr::Avr::standby_commands` takes no role
   parameter, which is that statement in a signature.
2. **A fully-off television cannot be cold-woken by CEC.** `<Image View On>`
   reaches nothing at mains standby; that needs a magic packet — as does the
   receiver itself, when its network-control-in-standby menu setting is off.

So the IP steps run **before** the CEC steps on both `wake` and `standby`,
unconditionally, exactly as #191 sequenced them. Standby is ordered that way for
a second reason: a `Z2OFF` has to reach a receiver that is still *awake*, and the
CEC `<Standby>` going first would put it (and, with network control in standby
off, its NIC) to sleep before the one command CEC cannot express had been sent.

**§13 Q7's wording wants amending to say this.** That edit is step 8 of the plan,
not this change.

### The television's IP leg is Wake-on-LAN only — by decision

§8 promises "webOS for state and standby". **There is no webOS code in this tree
— no SSAP client, no pairing key — and none was written here.** The TV IP leg is
**WoL-only, write-only, with no state read**:

- WoL is the only IP operation the television genuinely needs that CEC cannot do.
- A webOS client is a **second auth surface** with a documented history of
  breaking across firmware, which is one of the three reasons §13 Q7 demoted IP
  in the first place.

A magic packet is acknowledged by nobody, so `wol_packets` counts what left this
host and claims nothing about what received it. `av-state` gains no field from
the IP leg at all.

### Ported, not copied — and #191 is shape coverage, not hardware evidence

`ip/avr.rs` is a port of #191's `daemon/src/av_net.rs` onto typed `cec.toml`
(§8's own instruction); #191 was env-var-driven via `AvNetConfig::from_env`.
`ip/wol.rs` ports `Mac::parse` and `magic_packet` from `daemon/src/wol.rs` and
**only** those two — everything from `pick_mac` onward there (`ip neigh`
scraping, the `host-macs.json` cache, `handle_wol`) is Steam-host wiring for
waking the streaming PC, and `daemon/src/wol.rs` is not the television's WoL.
#191's own `MacAddr`/`magic_packet` pair is dropped in favour of these, so there
is one implementation rather than two.

**#191 was never hardware-tested, by its own admission** ("No on-device test of
the actual WoL/telnet against the real TV/AVR"). Its nine tests pin command
strings and config parsing; they are not evidence that a given receiver
answers to them. Neither is anything here — see the on-box checklist in the pull
request.

One deliberate correction to #191: it ordered standby as `[PWSTANDBY, Z2OFF]`.
This sends **`Z2OFF` first**, because a receiver told to stand by may drop the
control connection before the second line is read, which would silently lose the
one command the leg exists for.

### Failover: what moves the backend, and what does not

`failover.rs` is pure and takes its clock from its caller. Five rules:

1. **CEC is authoritative whenever `health.state == Healthy`** — the four
   *observed* facts, not a count of our own transmit failures. v1 inferred
   adapter health from transmit outcomes and that is the model this crate
   replaces.
2. **A single failed transmit does not fail over.** One NAK is the normal texture
   of a bus whose television is off. The transmit-side trigger is *N consecutive*
   failures **with no receive traffic in the same window** — traffic proves the
   adapter is still hearing, which makes the failures a fact about the *other*
   device. `Thresholds::validate` refuses `tx_error_threshold < 2`, so the
   "fail over on one NAK" mutation is unspellable in config as well as untrue in
   code.
3. **Hysteresis on both edges**, from `cec.toml`, and both consumed.
4. **Un-failover is kernel-driven, not timer-driven.** The device is kept OPEN
   through a degraded period — nothing is closed, so nothing has to be
   re-opened — and `PollResult::StateChange` re-reads the addressing the moment
   the adapter regains it. A recovery is committed **only** inside a fresh
   healthy *health* observation; elapsed time, heard traffic and accepted
   transmits cannot commit one. That is the structural fix for "a watchdog
   recovered a daemon three times that was never broken".
5. **Every change publishes its reason** — one log line naming the observation,
   and the same sentence in `backend`.

With **no IP leg configured** there is nowhere to fail over to, so `cec` stays
active and the reason says the adapter is degraded *and* that no IP leg exists.
Announcing an `ip` backend that does not exist would be the same class of claim
as reporting `unknown` as healthy.

While the IP leg is carrying actions, `volume` answers `error:` naming that fact
rather than transmitting into a bus the daemon has just concluded it cannot use.

### What this does and does not retire of jedwards1230/tv-shell#251

- **Retired: the reboot requirement.** A true adapter wedge is now a degraded
  mode recoverable in place — the backend moves to `ip`, says why, and moves back
  on a kernel event with no restart.
- **NOT retired: the USB-reset self-heal.** Re-enumerating the adapter by writing
  `authorized` or issuing `USBDEVFS_RESET` on the hub port needs root or a
  udev-granted write, and is **explicitly out of scope** here; it belongs with
  the privilege-model work (§13 Q8: a sudoers allowlist now, polkit later). Until
  then the daemon detects the wedge, fails over, and says so.

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

# The IP leg. BOTH SECTIONS ARE OPT-IN: with `host` and `wol_mac` empty — the
# defaults — nothing here opens a socket or sends a packet, and `backend`
# reports `cec` as the only backend there is.
[avr]
host       = ""           # the receiver's telnet control host; empty = no AVR
port       = 23           # Denon/Marantz speak ASCII over TCP/23
input      = ""           # `SI<input>` on wake, e.g. "GAME"; ASCII alphanumeric
main_power = false        # PWON / PWSTANDBY over telnet. OPT-IN: see below
zone2_off  = true         # Z2OFF on EVERY standby — CEC cannot address Zone 2

[tv]
wol_mac       = ""                  # the television's MAC; empty = no WoL
wol_broadcast = "255.255.255.255:9" # numeric addr:port, never a name

# The warm-path failover decision. Every key here is READ — see the mutation
# table's `every_threshold_changes_a_decision` row.
[failover]
tx_error_threshold = 3      # consecutive transmit failures. At least 2, enforced
tx_error_window_ms = 10000  # how far apart they may be and still be one run
fail_after_ms      = 5000   # how long a failing reading must hold
recover_after_ms   = 10000  # how long health must hold to come back
```

`[avr].main_power` stays an explicit opt-in **even when the IP leg is the
authority**: powering the receiver's main zone *down* is the action that can
black out a television somebody is watching, and this daemon does not grant
itself that authority merely because its own adapter stopped answering. What it
does instead is say so — an `ip`-carried `standby` with `main_power` off answers
`error:` naming the setting, after sending the `Z2OFF` that CEC could never
send. A power-**on** is not symmetrical (it blacks nothing out), so the
authority path sends `PWON` without an opt-in.

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
| `av-health` | one compact JSON document (below) | — |
| `wake` | `ok` / `refused:` / `error:` | `<Image View On>` → TV, then `<Active Source>` broadcast, then a `<Give Device Power Status>` read-back |
| `standby` | `ok` / `refused:` / `error:` | `<Standby>` → TV, then → Audio System. **Never broadcast.** Gated on `owns_display` |
| `input-claim` | `ok` / `refused:` / `error:` | `<Active Source>` broadcast |
| `input-release` | `ok` / `error:` | `<Inactive Source>` → TV |
| `input-select <phys-addr>` | `ok` / `error:usage:` / `error:` | `<Set Stream Path>` broadcast |
| `volume up\|down` | `ok` / `refused:` / `error:` / `error:usage:` | `<Give System Audio Mode Status>` (+ `<System Audio Mode Request>` if off) → `<Give Audio Status>` → `<User Control Pressed>[Volume Up/Down]` **and** `<User Control Released>` → `<Give Audio Status>` |
| `volume mute\|unmute` | as above | the same sequence, with `<User Control Pressed>[Mute]` — and **no key at all** when the AVR already reports the state asked for |
| `volume-state` | one compact JSON document (below) | `<Give Audio Status>`, falling back to the last one overheard |
| `backend` | `{active, available:[…], pin, reason}` | — (a read of a decision already made; no bus, no network) |
| `backend-pin cec\|ip\|auto` | `ok` / `error:` / `error:usage:` | — (it changes which wire the NEXT action uses) |

Anything else is `unknown`. Every verb but `input-select`, `volume` and
`backend-pin` is a bare read or bare action, so nothing may follow it: `av-stateX`, `av-healthX`,
`av-health 1`, `av-state 1`,
`standby now` and `volume-state 1` are all `unknown`. The two that take a body
take exactly one word, and a missing, malformed or extra body is
`error:usage: …` — **never a silent default**, and never `unknown` (the client
knows the verb; it got the call wrong). `input-select` because a
`<Set Stream Path>` naming a port that does not exist fails *silently* on the
bus; `volume` because a typo that read as `ok` would report a change nobody
asked for; `backend-pin` because defaulting a bare call to `auto` would silently
clear an override an operator had set.

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

`av-health`:

```json
{"state":"healthy","sinceMs":65000,"lastTxOk":800,"lastRxMs":4200,
 "busActivityMs":null,
 "reason":"the adapter fd answers and the adapter holds a physical and logical address; bus liveness from last-heard ages (CEC_CAP_MONITOR_PIN absent)"}
```

`state` is `healthy` / `degraded` / `unknown`, and **`unknown` is never rendered
as healthy**. **Every time here is an AGE in milliseconds, not a timestamp**, and
`null` means it has never happened — `lastRxMs: null` on a rack where everything
is switched off is a silence, not a fault, which is exactly why it is published
as an observation and not folded into the verdict. `sinceMs` is how long the
current state has held, and it moves only on a real transition, so a prober
confirming the same fact every few seconds cannot reset "degraded for four
minutes" to zero. `reason` names what was observed **and** which bus-liveness
signal is in force. It is answered from the recorded facts with no device access
at all, for the same reason `av-state` is.

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
| **`unknown` health is never rendered as healthy** | `health::classify`'s `(true, Unknown)` arm → `Healthy` | **4 tests** across `health` and `ipc` |
| **The watchdog feed is gated on fact 1 alone** | `should_feed_watchdog` → `true` | `the_watchdog_feed_is_gated_on_the_fd_and_on_nothing_else` + the `ipc` twin |
| …and NOT on the derived verdict | `should_feed_watchdog` → `state == Healthy` | the same two |
| Health comes from the four observed facts, not our transmit outcomes (v1's model) | derive the verdict from `last_tx_ok_at` | **4 tests** across `health` and `ipc` |
| `busActivityMs` is an age, never a verdict | collapse it to a moving/not constant | `the_observations_are_reported_as_ages_and_judged_by_nobody` |
| A silent bus is not a fault | add a last-rx age threshold that degrades the state | **5 tests** across `health` and `ipc` |
| The reason never claims a pin monitor `CEC_ADAP_G_CAPS` says is absent | `PinMonitor::from_capability` → `InForce` | `the_reason_names_the_signal_in_force_and_never_claims_an_absent_one` + the `ipc` twin |
| The state's age moves only on a real transition | drop the equality guard in `reclassify` | `the_state_age_moves_only_on_a_real_transition` |
| **A single failed transmit does not fail over** | set `deaf = true` on any `TxError` (i.e. `tx_error_threshold = 1`) | **4 tests** in `failover` |
| **A degraded adapter is not authoritative** | `desired` returns `Cec` for a degraded verdict | **10 tests** across `failover` and `ipc` |
| **The transmit rule needs silence in the same window** | drop `&& !heard_in_window` | `transmit_failures_with_traffic_in_the_window_do_not_fail_over` |
| **Hysteresis on the failing edge** | `hold = 0` for a failing candidate | **3 tests** in `failover` |
| **Hysteresis on the recovering edge** | `hold = 0` for a recovering candidate | `a_blip_is_suppressed_on_the_recovering_edge`, `every_threshold_changes_a_decision` |
| **Un-failover is kernel-driven, not timer-driven** | `may_commit = true` (any observation may commit a recovery) | `recovery_needs_a_fresh_healthy_observation_and_not_merely_elapsed_time` |
| **The IP leg is a complement, not only a fallback** | `plan_for` returns an empty plan unless the role is `Authority` | **6 tests** across `ip` and `ipc` |
| An accepted transmit ends the run of failures | drop the counter reset from `record`'s `TxOk` arm | `an_accepted_transmit_ends_the_run_of_failures` |
| Every `[failover]` threshold is READ | delete a field's only reader | `every_threshold_changes_a_decision` |

Two more live in `panel/` (`cargo test -p tv-shell-panel`), because that is where
the caller-side rules are:

| Rule | Mutation | What went red |
|---|---|---|
| The panel never renders `unknown` health as healthy | `pages::av::dot_class`'s fallthrough → `dot-ok` | `an_unknown_av_health_is_not_rendered_as_healthy` |
| **The panel never renders an `ip` backend as healthy** | `pages::av::backend_dot_class` → `dot-ok` | `an_ip_backend_is_not_rendered_as_healthy` |
| **A caller bounds its own wait** | `command_timeout(line, AV_TIMEOUT)` → `command(line)`, and → a one-hour bound | `av_page_renders_degraded_rather_than_hanging_on_a_wedged_daemon` (both times; the test wraps the render in its own 5 s bound so the unbounded case FAILS rather than hanging the suite) |

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

- **The USB-reset self-heal of jedwards1230/tv-shell#251** — re-enumerating a
  wedged adapter needs root or a udev-granted write, so it belongs with the
  privilege-model work (§13 Q8). This crate detects the wedge, fails over to the
  IP leg, and says so.
- **An LG webOS / SSAP client** — and it should stay absent. The television's IP
  leg is Wake-on-LAN only, by decision; see "The IP leg" above.
- **A `backend-pin` control on the panel** — the `/devices/av` page stays
  read-only. The verb is available on the daemon's socket.
- **An on-hardware run of any of it.** Everything below "Not yet here" used to
  include the §8 rewrite and enabling the unit; both landed. What is left is the
  operator step that gives this daemon a device to open.

Nothing here can be verified against real hardware yet: `/dev/cec0` does not
exist on the deploy box, and the receiver and television are live equipment in a
living room. The on-box checklists for after that operator step are in the pull
requests that added `health` and the IP leg. Note that the deploy box's journal
retains about a day (jedwards1230/tv-shell#509), so a failover soak **cannot be
evidenced after the fact** — raise retention or capture to a file before running
one.
