//! **PURE**: the health state machine, and the one rule the watchdog feed is
//! gated on.
//!
//! # What `av-health` is for, and what v1's `cec-health` could not know
//!
//! v1 inferred adapter health from **the outcome of our own transmits**
//! (`daemon/src/cec.rs:14-18`), and the Ansible CEC watchdog then inferred it a
//! *second* time from IPC reachability. The daemon does not unlink its socket on
//! shutdown, so a stale node outlived every stop, every read-only probe timed
//! out into "unreachable", and three of those "recovered" a daemon that was
//! never broken. Two inferences stacked on top of a signal that was never an
//! observation in the first place.
//!
//! So this module publishes **what was observed and when**, plus a tri-state
//! derived from it, and **`unknown` is a first-class value that is never
//! rendered as healthy**. The four independent, observed facts:
//!
//! 1. **The fd is alive** — `CEC_ADAP_G_CAPS` round-trips. A pure ioctl on our
//!    own file descriptor: no bus traffic, so probing it has no effect on
//!    anyone's television. A wedged USB device fails it; a healthy idle bus does
//!    not. [`Health::record_fd_probe`].
//! 2. **The adapter has an address** — `CEC_ADAP_G_PHYS_ADDR` reads back a valid
//!    address and `CEC_ADAP_G_LOG_ADDRS` is non-empty. Both are pure gets too.
//!    `PollResult::StateChange` is the kernel telling us this may have changed,
//!    with no probe of ours. [`Health::record_addressing`].
//! 3. **The bus is physically moving** — `PollResult::PinEvent`, line-level
//!    electrical activity observed passively. This is the one signal that
//!    separates *"the bus is quiet because everything is off"* from *"our
//!    adapter has stopped hearing"*. [`Health::record_bus_activity`]. **It is
//!    not in force on this deployment** — see [`PinMonitor`], which is published
//!    in the reason string rather than assumed either way.
//! 4. **Last successful tx / last rx**, reported as **ages**, never as a
//!    verdict. [`Health::record_tx_ok`] / [`Health::record_rx`].
//!
//! # Which facts derive the verdict, and which are only published
//!
//! **The verdict comes from facts 1 and 2 alone.** Facts 3 and 4 are published
//! as ages and judged by nobody here, because on this bus silence is not
//! evidence of anything: everything in the rack can be switched off, and without
//! the pin monitor there is no way to tell that apart from a deaf adapter.
//! Deriving `degraded` from a quiet bus would be inventing exactly the kind of
//! verdict this module exists to stop inventing — it is v1's mistake with the
//! sign flipped. If the pin monitor ever comes into force, fact 3 is the signal
//! that makes "we have stopped hearing" a *real* observation, and that is the
//! point at which it may sharpen the verdict; it does not before then.
//!
//! # The fail-safe direction inverts per consumer — so publish the tri-state
//!
//! `daemon/src/display_owner.rs:12-19` articulates it and [`crate::ownership`]
//! already ports it for display ownership: a caller deciding whether to attempt
//! an AV action wants "unknown ⇒ try anyway", while a caller deciding whether to
//! show the operator a fault wants "unknown ⇒ do not claim it is healthy".
//! Neither default is right for both, so this daemon publishes the tri-state
//! plus the observations it came from and each consumer picks its own safe side.
//!
//! # The liveness signal that matters is systemd's, not an IPC verb
//!
//! [`Health::should_feed_watchdog`] is the whole gate on `WATCHDOG=1`, and it is
//! **fact 1 and nothing else**. A wedged fd stops the feed and systemd SIGABRTs
//! and restarts the unit on a bounded timer: no polling script, no `cec-health`
//! probe with bus side effects, and no second supervisor holding restart
//! authority on the box (V2_DESIGN §9 — only one supervisor may). That is what
//! *retires* the Ansible watchdog rather than merely disabling it.
//!
//! **It is deliberately NOT gated on the derived state.** A lost address is
//! `Degraded`, and restarting the daemon does not give an HDMI topology back —
//! the TV is off, or the AVR is on another input. Feeding on the verdict instead
//! of on fact 1 would turn every standby into a restart loop, which is the
//! "recovered a daemon that was never broken" failure with a new mechanism.

use serde::Serialize;

use crate::state::Observation;

/// The published verdict. **Three values, and `unknown` is one of them.**
///
/// Serialized kebab-case, like every other enum on this wire.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum HealthState {
    /// The fd answers and the adapter holds an address.
    Healthy,
    /// Something observed is wrong: the fd stopped answering, or the adapter
    /// holds no address. `reason` names which.
    Degraded,
    /// Not enough has been observed to say. **Never rendered as healthy.**
    Unknown,
}

impl HealthState {
    /// The wire token, for callers that want it without serializing.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            HealthState::Healthy => "healthy",
            HealthState::Degraded => "degraded",
            HealthState::Unknown => "unknown",
        }
    }
}

/// Whether fact 3 — passive, line-level bus activity — is actually available.
///
/// **Read, never assumed** (plan §7 item 7). The capability comes from
/// `CEC_ADAP_G_CAPS` at open and the answer is *named in the reason string*
/// either way. On htpc-1's Pulse-Eight adapter it is measured absent
/// (2026-09-16), so [`PinMonitor::Unsupported`] is the deployed reality — but
/// other adapters differ, which is why this is read at runtime.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PinMonitor {
    /// `CEC_ADAP_G_CAPS` did not report `CEC_CAP_MONITOR_PIN`. Fact 3 does not
    /// exist on this adapter and fact 4 is the whole bus signal.
    Unsupported,
    /// The adapter reports the capability, but this daemon is not receiving pin
    /// events.
    ///
    /// Entering `CEC_MODE_MONITOR_PIN` is not a free upgrade and this daemon
    /// deliberately does not attempt it: `FollowerMode` is one value, so a
    /// monitor mode *replaces* `FollowerMode::Enabled` and the receive loop
    /// would stop folding `<Active Source>` — and the kernel gates the monitor
    /// modes on `CAP_NET_ADMIN`, which a `systemd --user` unit does not have.
    /// Both would have to be answered before fact 3 could be switched on.
    SupportedNotInForce,
    /// Pin events are actually arriving. Set by observation
    /// ([`Health::record_bus_activity`]), never by the capability flag — "the
    /// adapter could do this" and "we are receiving it" are different claims,
    /// and only the second one makes `busActivityMs` mean anything.
    InForce,
}

impl PinMonitor {
    /// The capability as read at open. Never [`PinMonitor::InForce`]: that is an
    /// observation, not a capability.
    #[must_use]
    pub const fn from_capability(present: bool) -> PinMonitor {
        if present {
            PinMonitor::SupportedNotInForce
        } else {
            PinMonitor::Unsupported
        }
    }

    /// How this is named in [`HealthReport::reason`].
    #[must_use]
    pub const fn describe(self) -> &'static str {
        match self {
            PinMonitor::Unsupported => {
                "bus liveness from last-heard ages (CEC_CAP_MONITOR_PIN absent)"
            }
            PinMonitor::SupportedNotInForce => {
                "bus liveness from last-heard ages (CEC_CAP_MONITOR_PIN present but not in \
                 force: a monitor follower mode would replace follower mode and needs \
                 CAP_NET_ADMIN)"
            }
            PinMonitor::InForce => "bus liveness from observed CEC pin transitions",
        }
    }
}

/// One `av-health` reply.
///
/// **Every time here is an AGE in milliseconds, not a timestamp**, and `null`
/// means it has never happened. An age is what a consumer can act on without
/// knowing this daemon's clock, and — the point — it is an observation rather
/// than a verdict: `busActivityMs` says when the bus last moved, and says
/// nothing at all about whether that is good.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HealthReport {
    /// The tri-state. `unknown` is never healthy.
    pub state: HealthState,
    /// How long the current state has held, in ms.
    pub since_ms: u64,
    /// Age of the last transmit the bus ACKed, in ms. `null` if none ever.
    pub last_tx_ok: Option<u64>,
    /// Age of the last message received from the bus, in ms. `null` if none
    /// ever — which on a quiet rack is the normal state, not a fault.
    pub last_rx_ms: Option<u64>,
    /// Age of the last observed CEC pin transition, in ms. `null` unless the pin
    /// monitor is in force (see [`PinMonitor`]).
    pub bus_activity_ms: Option<u64>,
    /// What was observed, and which bus-liveness signal is in force. Free text,
    /// one line.
    pub reason: String,
}

/// The observed facts, and the state derived from them.
///
/// Pure: every method takes the time from its caller, so the whole machine is
/// testable with no clock and no device. The I/O that feeds it lives in
/// [`crate::kernel`].
#[derive(Debug, Clone)]
pub struct Health {
    /// **Fact 1.** A plain `bool`, not a tri-state: this is a local ioctl that
    /// either answered or did not, and the only constructor takes the answer, so
    /// "not probed yet" is not a state this type can be in.
    fd_alive: bool,
    /// **Fact 2.** Tri-state, because the read itself can fail (a device going
    /// away mid-read) and because a `StateChange` invalidates it until it is
    /// re-read — neither of which is "the adapter has no address".
    addressed: Observation<bool>,
    /// **Fact 4.**
    last_tx_ok_at: Option<u64>,
    /// **Fact 4.**
    last_rx_at: Option<u64>,
    /// **Fact 3.**
    last_bus_activity_at: Option<u64>,
    pin_monitor: PinMonitor,
    state: HealthState,
    /// When the current state was entered. Moved on a real transition only, so
    /// "degraded for 40 s" does not reset every time the prober confirms it —
    /// the same rule [`crate::state::Observations`] applies to the ownership
    /// timestamp, for the same reason.
    state_since: u64,
}

impl Health {
    /// The facts established by a successful open: the fd answered
    /// (`CEC_ADAP_G_CAPS` is what produced `monitor_pin`), and the adapter's
    /// addressing as read back.
    ///
    /// The **only** constructor, on purpose. A `Default` would represent a
    /// daemon that has an open device and has observed nothing about it, which
    /// no code path can produce.
    #[must_use]
    pub fn at_open(monitor_pin: bool, addressed: Observation<bool>, now_ms: u64) -> Health {
        let pin_monitor = PinMonitor::from_capability(monitor_pin);
        Health {
            fd_alive: true,
            addressed,
            last_tx_ok_at: None,
            last_rx_at: None,
            last_bus_activity_at: None,
            pin_monitor,
            state: classify(true, addressed).0,
            state_since: now_ms,
        }
    }

    /// **Fact 1.** Record the outcome of a `CEC_ADAP_G_CAPS` probe.
    pub fn record_fd_probe(&mut self, alive: bool, now_ms: u64) {
        self.fd_alive = alive;
        self.reclassify(now_ms);
    }

    /// **Fact 2.** Record the adapter's addressing as read back, or `Unknown`
    /// when the read itself failed.
    pub fn record_addressing(&mut self, addressed: Observation<bool>, now_ms: u64) {
        self.addressed = addressed;
        self.reclassify(now_ms);
    }

    /// **Fact 2.** The kernel reported a `StateChange`: whatever we last read
    /// about the addressing may no longer be true.
    ///
    /// Goes to `unknown`, **not** to "unaddressed": a `StateChange` also fires
    /// when the adapter *gains* an address. The receive loop re-reads
    /// immediately, so this is normally a state that lasts milliseconds — but it
    /// is a real one, and reporting `healthy` across it would be reporting a
    /// topology we had just been told was stale.
    pub fn record_state_change(&mut self, now_ms: u64) {
        self.record_addressing(Observation::Unknown, now_ms);
    }

    /// **Fact 4.** A transmit the bus ACKed.
    pub fn record_tx_ok(&mut self, now_ms: u64) {
        self.last_tx_ok_at = Some(now_ms);
    }

    /// **Fact 4.** A message received from the bus — any message, including one
    /// this daemon does not otherwise track. The question is "did we hear
    /// anything", not "did we hear something we care about".
    pub fn record_rx(&mut self, now_ms: u64) {
        self.last_rx_at = Some(now_ms);
    }

    /// **Fact 3.** A CEC pin transition — line-level electrical activity.
    ///
    /// Receiving one is also the proof that the pin monitor is in force, so it
    /// promotes [`PinMonitor`]. Nothing else does: the capability flag says what
    /// the adapter *could* do.
    pub fn record_bus_activity(&mut self, now_ms: u64) {
        self.last_bus_activity_at = Some(now_ms);
        self.pin_monitor = PinMonitor::InForce;
    }

    /// **The watchdog gate: fact 1, and nothing else.** See the module docs for
    /// why it is not the derived state.
    #[must_use]
    pub const fn should_feed_watchdog(&self) -> bool {
        self.fd_alive
    }

    /// The current verdict, without the ages.
    #[must_use]
    pub const fn state(&self) -> HealthState {
        self.state
    }

    /// Which bus-liveness signal is in force.
    #[must_use]
    pub const fn pin_monitor(&self) -> PinMonitor {
        self.pin_monitor
    }

    /// The published report, as of `now_ms`.
    #[must_use]
    pub fn report(&self, now_ms: u64) -> HealthReport {
        let (state, verdict) = classify(self.fd_alive, self.addressed);
        HealthReport {
            state,
            since_ms: age(now_ms, self.state_since),
            last_tx_ok: self.last_tx_ok_at.map(|t| age(now_ms, t)),
            last_rx_ms: self.last_rx_at.map(|t| age(now_ms, t)),
            bus_activity_ms: self.last_bus_activity_at.map(|t| age(now_ms, t)),
            reason: format!("{verdict}; {}", self.pin_monitor.describe()),
        }
    }

    /// Re-derive the state, moving the timestamp only on a real transition.
    fn reclassify(&mut self, now_ms: u64) {
        let next = classify(self.fd_alive, self.addressed).0;
        if next == self.state {
            return;
        }
        self.state = next;
        self.state_since = now_ms;
    }
}

/// **The derivation, in one place: facts 1 and 2 to a verdict and its reason.**
///
/// Pure and total. Facts 3 and 4 are deliberately not inputs — see the module
/// docs: a quiet bus is not evidence, and turning it into one would be the same
/// class of invention as judging health from our own transmit outcomes.
#[must_use]
pub fn classify(fd_alive: bool, addressed: Observation<bool>) -> (HealthState, &'static str) {
    match (fd_alive, addressed) {
        // Fact 1 failed. The USB device is wedged or gone; nothing else matters,
        // and this is the case systemd's watchdog is about to act on.
        (false, _) => (
            HealthState::Degraded,
            "the adapter fd is not answering CEC_ADAP_G_CAPS",
        ),
        // Addressed and answering.
        (true, Observation::Known(true)) => (
            HealthState::Healthy,
            "the adapter fd answers and the adapter holds a physical and logical address",
        ),
        // Answering, but off the bus: an unplugged HDMI cable, a TV at mains
        // standby, an AVR that dropped the link. NOT a reason to restart the
        // daemon — see `should_feed_watchdog`.
        (true, Observation::Known(false)) => (
            HealthState::Degraded,
            "the adapter fd answers but the adapter holds no physical/logical address",
        ),
        // We were told the topology changed, or the read-back itself failed.
        // `unknown`, and it is never rendered as healthy.
        (true, Observation::Unknown) => (
            HealthState::Unknown,
            "the adapter fd answers; its addressing has not been read back since the kernel \
             last reported a state change",
        ),
    }
}

/// `now - then`, saturating.
///
/// A clock that went backwards yields 0 rather than a huge age or a panic: this
/// is a long-running daemon and an NTP step is not a reason to take it down.
const fn age(now_ms: u64, then_ms: u64) -> u64 {
    now_ms.saturating_sub(then_ms)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn healthy(now: u64) -> Health {
        Health::at_open(false, Observation::Known(true), now)
    }

    /// **THE RULE OF THIS MODULE: `unknown` is a first-class state and is never
    /// rendered as healthy.**
    ///
    /// Reachable from the real path two ways, both of which the kernel layer
    /// produces: a `PollResult::StateChange` (the kernel saying the adapter was
    /// reconfigured) and a failed `CEC_ADAP_G_PHYS_ADDR`/`G_LOG_ADDRS` read-back
    /// (which `kernel::device::read_addressing` maps to `Observation::Unknown`).
    ///
    /// Mutation-check (run 2026-09-14): make the `(true, Unknown)` arm of
    /// `classify` return `HealthState::Healthy` — i.e. render unknown as
    /// healthy — and this fails on the first assertion, together with
    /// `the_reply_never_renders_unknown_as_healthy` and the panel's
    /// `an_unknown_av_health_is_not_rendered_as_healthy`.
    #[test]
    fn an_unread_addressing_is_unknown_and_never_healthy() {
        let mut h = healthy(1_000);
        assert_eq!(h.state(), HealthState::Healthy);

        h.record_state_change(1_100);
        assert_eq!(h.state(), HealthState::Unknown);
        assert_ne!(h.state(), HealthState::Healthy);

        // A failed read-back is the same answer, for the same reason.
        let mut h = healthy(1_000);
        h.record_addressing(Observation::Unknown, 1_100);
        assert_eq!(h.state(), HealthState::Unknown);
    }

    /// The three verdicts, each reached by the observation that produces it.
    #[test]
    fn the_three_verdicts_come_from_the_facts_that_produce_them() {
        let mut h = healthy(0);
        assert_eq!(h.state(), HealthState::Healthy);

        // Fact 2 observed false — the adapter is on the bus but unaddressed.
        h.record_addressing(Observation::Known(false), 10);
        assert_eq!(h.state(), HealthState::Degraded);

        // Fact 1 observed false — the fd stopped answering.
        let mut h = healthy(0);
        h.record_fd_probe(false, 10);
        assert_eq!(h.state(), HealthState::Degraded);

        // And a dead fd stays degraded whatever the addressing last said.
        h.record_addressing(Observation::Known(true), 20);
        assert_eq!(h.state(), HealthState::Degraded);
    }

    /// **THE WATCHDOG RULE: the feed is gated on fact 1 alone.**
    ///
    /// Two halves, and both matter. A dead fd must stop the feed (that is what
    /// makes systemd restart a wedged daemon on a bounded timer). A *degraded*
    /// or *unknown* daemon whose fd still answers must KEEP feeding — restarting
    /// does not give an HDMI address back, so gating on the verdict would make
    /// every television standby a restart loop: v1's "recovered a daemon that
    /// was never broken", with a new mechanism.
    ///
    /// Mutation-check (run 2026-09-14): make `should_feed_watchdog` return
    /// `true` unconditionally and the first assertion fails; make it
    /// `self.state == HealthState::Healthy` and the last two fail.
    #[test]
    fn the_watchdog_feed_is_gated_on_the_fd_and_on_nothing_else() {
        let mut h = healthy(0);
        assert!(h.should_feed_watchdog());

        h.record_fd_probe(false, 10);
        assert!(
            !h.should_feed_watchdog(),
            "a wedged fd must stop the feed so systemd restarts the unit"
        );

        h.record_fd_probe(true, 20);
        assert!(h.should_feed_watchdog());

        // Degraded, but answering: keep feeding.
        h.record_addressing(Observation::Known(false), 30);
        assert_eq!(h.state(), HealthState::Degraded);
        assert!(
            h.should_feed_watchdog(),
            "an unaddressed adapter is not a reason to restart the daemon"
        );

        // Unknown, but answering: keep feeding.
        h.record_state_change(40);
        assert_eq!(h.state(), HealthState::Unknown);
        assert!(h.should_feed_watchdog());
    }

    /// **The rule: the four observations are reported as AGES, never as a
    /// verdict.**
    ///
    /// In particular a long-silent bus is reported as a large `busActivityMs` /
    /// `lastRxMs` and a `healthy` state, because on this rack everything being
    /// switched off is the normal case and calling it degraded would be an
    /// invented verdict.
    ///
    /// Mutation-check (run 2026-09-14): add a `last_rx`/`bus_activity` age
    /// threshold to `classify` (so silence degrades the state) and the state
    /// assertion here fails; change `bus_activity_ms` to a bool/verdict and the
    /// crate does not compile against this test's numeric comparison.
    #[test]
    fn the_observations_are_reported_as_ages_and_judged_by_nobody() {
        let mut h = healthy(1_000);
        h.record_rx(1_500);
        h.record_tx_ok(1_800);
        h.record_bus_activity(1_900);

        let r = h.report(2_000);
        assert_eq!(r.last_rx_ms, Some(500));
        assert_eq!(r.last_tx_ok, Some(200));
        assert_eq!(r.bus_activity_ms, Some(100));
        assert_eq!(r.since_ms, 1_000);

        // Hours of silence later, with the fd and the addressing unchanged: the
        // ages are enormous and the verdict has not moved.
        let r = h.report(1_000 + 6 * 3_600_000);
        assert!(r.last_rx_ms.unwrap() > 6 * 3_500_000);
        assert_eq!(r.state, HealthState::Healthy);
    }

    /// A fact that has never happened is `null` — never `0`, which would read as
    /// "just now".
    #[test]
    fn an_observation_that_never_happened_is_null_and_not_zero() {
        let h = healthy(1_000);
        let json: serde_json::Value =
            serde_json::from_str(&serde_json::to_string(&h.report(2_000)).unwrap()).unwrap();
        for field in ["lastTxOk", "lastRxMs", "busActivityMs"] {
            assert_eq!(json[field], serde_json::Value::Null, "{field}: {json}");
            assert_ne!(json[field], serde_json::json!(0), "{field}");
        }
    }

    /// **The rule: the state's age moves on a real transition and on nothing
    /// else.**
    ///
    /// The prober confirms the same fact every few seconds; if each confirmation
    /// reset the clock, "degraded for four minutes" would be unaskable — and
    /// that is the reading an operator uses to tell a blip from a fault.
    ///
    /// Mutation-check (run 2026-09-14): drop the `if next == self.state {
    /// return; }` guard in `reclassify` and the repeat-probe assertion fails.
    #[test]
    fn the_state_age_moves_only_on_a_real_transition() {
        let mut h = healthy(1_000);
        h.record_fd_probe(false, 2_000);
        assert_eq!(h.report(2_000).since_ms, 0);

        // Three more probes agreeing with the first.
        h.record_fd_probe(false, 3_000);
        h.record_fd_probe(false, 4_000);
        h.record_fd_probe(false, 5_000);
        assert_eq!(
            h.report(5_000).since_ms,
            3_000,
            "a confirmation is not a transition"
        );

        // A real recovery does move it.
        h.record_fd_probe(true, 6_000);
        assert_eq!(h.report(6_000).since_ms, 0);
        assert_eq!(h.report(6_000).state, HealthState::Healthy);
    }

    /// **The rule: the reason names which bus-liveness signal is in force, and
    /// it never claims the pin monitor when `CEC_ADAP_G_CAPS` says it is
    /// absent** (plan §7 item 7 — unverifiable without a device).
    ///
    /// Mutation-check (run 2026-09-14): make `PinMonitor::from_capability`
    /// return `InForce`, or make `describe` claim pin transitions for the
    /// `Unsupported` arm, and the first two assertions fail.
    #[test]
    fn the_reason_names_the_signal_in_force_and_never_claims_an_absent_one() {
        let absent = Health::at_open(false, Observation::Known(true), 0);
        assert_eq!(absent.pin_monitor(), PinMonitor::Unsupported);
        let reason = absent.report(0).reason;
        assert!(reason.contains("CEC_CAP_MONITOR_PIN absent"), "{reason}");
        assert!(
            !reason.contains("pin transitions"),
            "a reason must not claim a signal the adapter does not report: {reason}"
        );

        // Present in the capability set is still not "in force": the mode is not
        // entered, so no pin event can arrive.
        let present = Health::at_open(true, Observation::Known(true), 0);
        assert_eq!(present.pin_monitor(), PinMonitor::SupportedNotInForce);
        let reason = present.report(0).reason;
        assert!(reason.contains("not in force"), "{reason}");
        assert!(reason.contains("CAP_NET_ADMIN"), "{reason}");

        // Only an actually-observed pin event promotes it.
        let mut present = Health::at_open(true, Observation::Known(true), 0);
        present.record_bus_activity(10);
        assert_eq!(present.pin_monitor(), PinMonitor::InForce);
        assert!(present.report(10).reason.contains("pin transitions"));
    }

    /// The documented shape, whole, with `unknown` reaching the wire as the
    /// token and never as an omission.
    #[test]
    fn the_reply_never_renders_unknown_as_healthy() {
        let mut h = healthy(1_000);
        h.record_state_change(1_000);
        let json: serde_json::Value =
            serde_json::from_str(&serde_json::to_string(&h.report(1_500)).unwrap()).unwrap();
        for field in [
            "state",
            "sinceMs",
            "lastTxOk",
            "lastRxMs",
            "busActivityMs",
            "reason",
        ] {
            assert!(
                json.as_object().unwrap().contains_key(field),
                "av-health must carry {field}: {json}"
            );
        }
        assert_eq!(json["state"], serde_json::json!("unknown"));
        assert_ne!(json["state"], serde_json::json!("healthy"));
        assert_eq!(json["sinceMs"], serde_json::json!(500));
    }

    /// A clock that steps backwards yields 0, not an enormous age.
    #[test]
    fn a_backwards_clock_is_not_a_panic() {
        let mut h = healthy(10_000);
        h.record_rx(10_000);
        let r = h.report(5_000);
        assert_eq!(r.since_ms, 0);
        assert_eq!(r.last_rx_ms, Some(0));
    }

    #[test]
    fn the_state_tokens_are_stable() {
        assert_eq!(HealthState::Healthy.as_str(), "healthy");
        assert_eq!(HealthState::Degraded.as_str(), "degraded");
        assert_eq!(HealthState::Unknown.as_str(), "unknown");
    }
}
