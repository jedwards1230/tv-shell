//! **PURE**: which backend is authoritative on the warm path, and why.
//!
//! No I/O, no device, no clock read — every method takes the time from its
//! caller — so the whole decision is covered by CI on a runner with no adapter
//! and no network. [`crate::kernel::device`] feeds it observations;
//! [`crate::ip`] carries out what it decides.
//!
//! # What this decides, and what it does NOT
//!
//! It decides the **warm path only**: with a working CEC bus, CEC acts; with a
//! CEC bus this daemon can no longer use, the IP leg acts instead. The
//! **cold-path** steps — a Wake-on-LAN packet for a television at mains standby,
//! a `Z2OFF` for a zone CEC cannot address — run regardless of anything decided
//! here. See [`crate::ip`] for why §13 Q7's "IP only when CEC is unavailable" is
//! too narrow.
//!
//! # The five rules
//!
//! 1. **CEC is authoritative whenever [`crate::health::HealthState::Healthy`]
//!    holds.** Health is the four *observed* facts of [`crate::health`] — the fd
//!    answering `CEC_ADAP_G_CAPS` and the adapter holding an address — not a
//!    count of our own transmit failures. v1 inferred adapter health from
//!    transmit outcomes and that is the model this crate exists to replace.
//! 2. **A single failed transmit does not fail over.** One NAK is the normal
//!    texture of a shared bus with a television that is off. The transmit-side
//!    trigger is *N consecutive* [`Observed::TxError`]s **with no receive
//!    traffic in the same window** — because rx traffic proves the adapter is
//!    still hearing, which makes a transmit failure a fact about the *other*
//!    device rather than about us.
//! 3. **Hysteresis on both edges.** A candidate change has to hold for its
//!    threshold before it is committed, so a blip flips nothing. Both thresholds
//!    come from `cec.toml` and both are **consumed** — see
//!    `every_threshold_changes_a_decision`, which fails if a knob stops being
//!    read. `core.toml`'s `[supervisor].restart_threshold` is the standing
//!    example of the other thing: a key written and read by nothing.
//! 4. **Un-failover is kernel-driven, not timer-driven.** A recovery is
//!    committed **only** inside a fresh [`Observed::Health`] observation that
//!    says `Healthy` — the kernel's `PollResult::StateChange` re-reading the
//!    adapter's addressing, or the watchdog probe re-reading the same two facts.
//!    Elapsed time alone recovers nothing: [`Failover::observe`] is the only
//!    thing that can move the backend, and it cannot be called by a clock. That
//!    is the structural fix for "a watchdog recovered a daemon three times that
//!    was never broken" — the failure mode that produced this whole design.
//! 5. **Every change publishes its reason.** [`Transition`] carries the
//!    observation that caused it, the daemon logs exactly one line per change,
//!    and `backend` publishes the current one.
//!
//! # And the case with nowhere to go
//!
//! With no IP leg configured there is no backend to fail over **to**. The
//! decision then keeps `cec` active and says in its reason that CEC is degraded
//! and unconfigured-IP is why nothing changed. Announcing an `ip` backend that
//! does not exist would be the same class of claim as reporting `unknown` as
//! healthy.

use serde::Serialize;

use crate::health::HealthState;
use crate::ip::{IpConfig, IpRole};

/// Which backend is carrying out actions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Backend {
    /// The kernel CEC adapter.
    Cec,
    /// The IP leg — Wake-on-LAN and the receiver's telnet control port.
    Ip,
}

impl Backend {
    /// The wire token.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Backend::Cec => "cec",
            Backend::Ip => "ip",
        }
    }

    /// The role the IP leg plays while this backend is authoritative.
    #[must_use]
    pub const fn ip_role(self) -> IpRole {
        match self {
            Backend::Cec => IpRole::Complement,
            Backend::Ip => IpRole::Authority,
        }
    }
}

/// An operator override.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Pin {
    /// The automatic decision. The default.
    Auto,
    /// Force the kernel CEC backend, whatever the observations say.
    Cec,
    /// Force the IP leg.
    Ip,
}

impl Pin {
    /// Parse the `backend-pin` argument.
    #[must_use]
    pub fn parse(word: &str) -> Option<Pin> {
        match word {
            "auto" => Some(Pin::Auto),
            "cec" => Some(Pin::Cec),
            "ip" => Some(Pin::Ip),
            _ => None,
        }
    }

    /// The wire token.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Pin::Auto => "auto",
            Pin::Cec => "cec",
            Pin::Ip => "ip",
        }
    }
}

/// The four knobs, from `cec.toml`'s `[failover]`.
///
/// Every one of them is read by [`Failover`], and a test fails if that stops
/// being true. A configuration key whose stated consumer does not exist is the
/// jedwards1230/tv-shell#416 class.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Thresholds {
    /// How many consecutive transmit failures, with no receive traffic in the
    /// same window, count as "this adapter has stopped working".
    pub tx_error_threshold: u32,
    /// How long that run of failures may span before it is treated as a fresh
    /// run rather than a continuing one.
    pub tx_error_window_ms: u64,
    /// How long a failing observation must hold before the backend actually
    /// changes. The hysteresis on the way out.
    pub fail_after_ms: u64,
    /// How long health must hold before the backend changes back. The
    /// hysteresis on the way in — and it is counted across *observations*, never
    /// across elapsed time alone (rule 4).
    pub recover_after_ms: u64,
}

impl Default for Thresholds {
    fn default() -> Thresholds {
        Thresholds {
            tx_error_threshold: 3,
            tx_error_window_ms: 10_000,
            fail_after_ms: 5_000,
            recover_after_ms: 10_000,
        }
    }
}

impl Thresholds {
    /// Reject values that would make the decision meaningless.
    ///
    /// A zero transmit-error threshold is the exact mutation rule 2 forbids, so
    /// it is refused at startup rather than only in a test.
    pub fn validate(&self) -> Result<(), String> {
        if self.tx_error_threshold < 2 {
            return Err(format!(
                "tx_error_threshold must be at least 2 (got {}); a threshold of 1 makes a \
                 SINGLE failed transmit fail over, and one NAK is the normal texture of a bus \
                 whose television is switched off",
                self.tx_error_threshold
            ));
        }
        for (name, value) in [
            ("tx_error_window_ms", self.tx_error_window_ms),
            ("fail_after_ms", self.fail_after_ms),
            ("recover_after_ms", self.recover_after_ms),
        ] {
            if value == 0 {
                return Err(format!(
                    "{name} must be greater than zero; zero removes the hysteresis this \
                     decision depends on"
                ));
            }
        }
        Ok(())
    }
}

/// One thing that was observed, from the daemon's own I/O.
///
/// A closed set with an exhaustive match, so an observation added without a
/// decision does not compile.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Observed {
    /// The health machine's verdict, re-derived from facts 1 and 2. **The only
    /// observation that can commit a recovery** — see rule 4.
    Health(HealthState),
    /// A transmit the bus accepted.
    TxOk,
    /// A transmit the bus did not accept.
    TxError,
    /// A message heard from the bus. Proof the adapter is still hearing.
    Rx,
}

impl Observed {
    /// How this reads in a reason string.
    fn describe(self) -> &'static str {
        match self {
            Observed::Health(HealthState::Healthy) => "the adapter is healthy",
            Observed::Health(HealthState::Degraded) => "the adapter is degraded",
            Observed::Health(HealthState::Unknown) => "the adapter's health is unknown",
            Observed::TxOk => "a transmit was accepted",
            Observed::TxError => "a transmit failed",
            Observed::Rx => "a message was heard",
        }
    }
}

/// A committed change of backend, with the observation that caused it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Transition {
    pub from: Backend,
    pub to: Backend,
    /// One line naming what was observed. Logged verbatim.
    pub reason: String,
}

/// One `backend` reply.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BackendReport {
    /// Which backend is carrying out actions right now.
    pub active: Backend,
    /// Every backend that could carry one out. `cec` is always present — the
    /// adapter is open whether or not it is answering — and `ip` only when
    /// `cec.toml` configures one.
    pub available: Vec<Backend>,
    /// The operator override in force, `auto` when there is none.
    pub pin: Pin,
    /// Why `active` is what it is. Free text, one line.
    pub reason: String,
}

/// The decision, and the observations it was made from.
#[derive(Debug, Clone)]
pub struct Failover {
    thresholds: Thresholds,
    /// Whether there is anywhere to fail over to.
    ip_configured: bool,
    pin: Pin,
    /// The committed backend, under [`Pin::Auto`].
    active: Backend,
    /// Why `active` is what it is.
    reason: String,
    /// The last health verdict observed.
    health: HealthState,
    /// Whether the transmit-side rule has tripped: N consecutive failures with
    /// nothing heard. Cleared by any receive traffic or accepted transmit.
    deaf: bool,
    /// A change that has been observed but not yet held long enough.
    pending: Option<Pending>,
    /// The start of the current run of transmit failures.
    window_start: Option<u64>,
    tx_errors: u32,
    /// When the bus was last heard from.
    ///
    /// Compared against [`Thresholds::tx_error_window_ms`] to answer "was there
    /// receive traffic in the same window as this run of failures" — the
    /// condition that disqualifies a run as evidence about **our** adapter. A
    /// timestamp rather than a flag because the question is about recency, and a
    /// flag can only answer it for one arbitrary window boundary.
    last_rx_at: Option<u64>,
}

/// A candidate change, waiting out its hysteresis.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Pending {
    to: Backend,
    since: u64,
    reason: String,
}

impl Failover {
    /// The state at open: the adapter's health as the open sequence read it,
    /// and whether an IP leg is configured.
    ///
    /// The **only** constructor. A `Default` would describe a daemon that has an
    /// open adapter and has observed nothing about it, which no path produces.
    #[must_use]
    pub fn at_open(thresholds: Thresholds, ip: &IpConfig, health: HealthState) -> Failover {
        Failover {
            thresholds,
            ip_configured: ip.is_configured(),
            pin: Pin::Auto,
            active: Backend::Cec,
            reason: format!(
                "the kernel CEC adapter is the primary backend; at open its health was {}",
                health.as_str()
            ),
            health,
            deaf: false,
            pending: None,
            window_start: None,
            tx_errors: 0,
            last_rx_at: None,
        }
    }

    /// The backend that is carrying out actions.
    ///
    /// **Takes no clock**, which is rule 4 expressed in a signature: nothing can
    /// change the answer except an observation.
    #[must_use]
    pub const fn active(&self) -> Backend {
        match self.pin {
            Pin::Auto => self.active,
            Pin::Cec => Backend::Cec,
            Pin::Ip => Backend::Ip,
        }
    }

    /// Which backends exist on this box.
    #[must_use]
    pub fn available(&self) -> Vec<Backend> {
        if self.ip_configured {
            vec![Backend::Cec, Backend::Ip]
        } else {
            vec![Backend::Cec]
        }
    }

    /// The published report.
    #[must_use]
    pub fn report(&self) -> BackendReport {
        BackendReport {
            active: self.active(),
            available: self.available(),
            pin: self.pin,
            reason: self.reason(),
        }
    }

    /// Why the active backend is what it is.
    #[must_use]
    pub fn reason(&self) -> String {
        match self.pin {
            Pin::Auto => self.reason.clone(),
            pin => format!(
                "pinned to {} by an operator; the automatic decision would be {} ({})",
                pin.as_str(),
                self.active.as_str(),
                self.reason
            ),
        }
    }

    /// Apply an operator override.
    ///
    /// Refuses a pin to a backend this box does not have: answering `ok` and
    /// then acting over CEC anyway would be a control that reports an effect
    /// nothing applies.
    pub fn set_pin(&mut self, pin: Pin) -> Result<(), String> {
        if pin == Pin::Ip && !self.ip_configured {
            return Err(
                "no IP leg is configured ([avr] and [tv] are both absent from cec.toml), so \
                 there is no ip backend to pin to"
                    .to_string(),
            );
        }
        self.pin = pin;
        Ok(())
    }

    /// The override in force.
    #[must_use]
    pub const fn pin(&self) -> Pin {
        self.pin
    }

    /// Fold one observation in, and say whether the backend changed.
    ///
    /// `Some(transition)` exactly when the active backend moved, so the caller
    /// logs one line per change and not one per observation — on a box whose
    /// journal retains about a day (jedwards1230/tv-shell#509), a line per probe
    /// is a line that pushes out the one that mattered.
    pub fn observe(&mut self, observed: Observed, now_ms: u64) -> Option<Transition> {
        self.record(observed, now_ms);

        let (desired, why) = self.desired();
        if desired == self.active {
            // Whatever was pending has been overtaken by the current reading.
            self.pending = None;
            self.reason = why;
            return None;
        }

        // Rule 4: a recovery is committed only inside a fresh healthy health
        // observation. Elapsed time, a heard message or an accepted transmit may
        // all be true while the adapter is still unusable, and none of them is
        // the re-read of facts 1 and 2 that recovery has to be evidenced by.
        let recovery = desired == Backend::Cec;
        let may_commit = !recovery || matches!(observed, Observed::Health(HealthState::Healthy));

        let hold = if recovery {
            self.thresholds.recover_after_ms
        } else {
            self.thresholds.fail_after_ms
        };

        match &self.pending {
            Some(pending) if pending.to == desired => {
                let held = now_ms.saturating_sub(pending.since);
                if may_commit && held >= hold {
                    let from = self.active;
                    let reason = format!(
                        "{} ({} held for {held} ms, threshold {hold} ms)",
                        pending.reason,
                        observed.describe()
                    );
                    self.active = desired;
                    self.reason = reason.clone();
                    self.pending = None;
                    return Some(Transition {
                        from,
                        to: desired,
                        reason,
                    });
                }
            }
            // A different candidate, or none: start the clock now. A candidate
            // that changes resets its own hysteresis, which is what makes a flap
            // suppress rather than accumulate.
            _ => {
                self.pending = Some(Pending {
                    to: desired,
                    since: now_ms,
                    reason: why,
                });
            }
        }
        None
    }

    /// Update the raw signals. No decision here.
    fn record(&mut self, observed: Observed, now_ms: u64) {
        match observed {
            Observed::Health(state) => self.health = state,
            // An accepted transmit ENDS the run: the rule counts *consecutive*
            // failures, and this one did not fail.
            Observed::TxOk => {
                self.tx_errors = 0;
                self.window_start = None;
                self.deaf = false;
            }
            // A heard message does **not** end the run — the failures either
            // side of it are still consecutive — but it does mark the window as
            // one in which we were demonstrably still hearing, which is the
            // condition that disqualifies the run as evidence about *us*. It is
            // the stronger of the two signals: it separates "the bus is quiet"
            // from "we have stopped hearing", which nothing else here can.
            Observed::Rx => {
                self.last_rx_at = Some(now_ms);
                self.deaf = false;
            }
            Observed::TxError => {
                let fresh = match self.window_start {
                    Some(start) => {
                        now_ms.saturating_sub(start) > self.thresholds.tx_error_window_ms
                    }
                    None => true,
                };
                if fresh {
                    self.window_start = Some(now_ms);
                    self.tx_errors = 1;
                } else {
                    self.tx_errors = self.tx_errors.saturating_add(1);
                }
                // Rule 2, in one condition: a run of failures is evidence about
                // **our** adapter only if nothing was heard in the same window.
                // A bus that has spoken to us recently is a bus we can hear, so
                // the failures are a fact about the device that did not answer.
                let heard_in_window = self.last_rx_at.is_some_and(|t| {
                    now_ms.saturating_sub(t) <= self.thresholds.tx_error_window_ms
                });
                if self.tx_errors >= self.thresholds.tx_error_threshold && !heard_in_window {
                    self.deaf = true;
                }
            }
        }
    }

    /// The backend the current observations call for, and why.
    fn desired(&self) -> (Backend, String) {
        if self.health == HealthState::Healthy && !self.deaf {
            return (
                Backend::Cec,
                "the adapter fd answers and the adapter holds an address, so the kernel CEC \
                 backend is authoritative"
                    .to_string(),
            );
        }
        let why = if self.deaf {
            format!(
                "{} consecutive transmit failures with nothing heard from the bus in the same \
                 window",
                self.tx_errors
            )
        } else {
            format!("the adapter's observed health is {}", self.health.as_str())
        };
        if !self.ip_configured {
            // Nowhere to go. Say so, and stay put — an `ip` backend that does
            // not exist must not be announced.
            return (
                Backend::Cec,
                format!(
                    "{why}, but no IP leg is configured, so there is no backend to fall back to"
                ),
            );
        }
        (
            Backend::Ip,
            format!("{why}; the IP leg is carrying actions"),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::ip::avr::{Avr, AvrEndpoint};
    use crate::ip::{wol::Mac, WolTarget};

    fn ip() -> IpConfig {
        IpConfig {
            avr: Some(Avr {
                endpoint: AvrEndpoint {
                    host: "192.0.2.10".to_string(),
                    port: 23,
                },
                input: None,
                main_power: false,
                zone2_off: true,
            }),
            tv_wol: Some(WolTarget {
                mac: Mac::parse("aa:bb:cc:dd:ee:ff").unwrap(),
                broadcast: "255.255.255.255:9".parse().unwrap(),
            }),
        }
    }

    fn healthy() -> Failover {
        Failover::at_open(Thresholds::default(), &ip(), HealthState::Healthy)
    }

    /// Fail over the ordinary way: a degraded verdict that holds.
    fn fail_over(f: &mut Failover, at: u64) -> Transition {
        assert!(f
            .observe(Observed::Health(HealthState::Degraded), at)
            .is_none());
        f.observe(
            Observed::Health(HealthState::Degraded),
            at + Thresholds::default().fail_after_ms,
        )
        .expect("a degraded verdict that holds must fail over")
    }

    #[test]
    fn cec_is_authoritative_while_the_adapter_is_healthy() {
        let mut f = healthy();
        assert_eq!(f.active(), Backend::Cec);
        assert!(f
            .observe(Observed::Health(HealthState::Healthy), 1)
            .is_none());
        assert_eq!(f.active(), Backend::Cec);
        assert_eq!(f.active().ip_role(), IpRole::Complement);
    }

    /// **RULE 1, and mutation (d): a degraded adapter is NOT authoritative.**
    ///
    /// Mutation-check (run 2026-09-14): make `desired` return `Backend::Cec`
    /// for `HealthState::Degraded` (or drop the `health == Healthy` test
    /// entirely) and this fails on the post-transition assertion, together with
    /// `an_unknown_verdict_also_fails_over` and the two `ipc` backend tests.
    #[test]
    fn a_degraded_adapter_is_not_authoritative() {
        let mut f = healthy();
        let t = fail_over(&mut f, 1_000);
        assert_eq!(t.from, Backend::Cec);
        assert_eq!(t.to, Backend::Ip);
        assert_eq!(f.active(), Backend::Ip);
        assert_eq!(f.active().ip_role(), IpRole::Authority);
        assert!(t.reason.contains("degraded"), "{}", t.reason);
    }

    /// `unknown` is not healthy, here as everywhere else in this crate.
    #[test]
    fn an_unknown_verdict_also_fails_over() {
        let mut f = healthy();
        assert!(f
            .observe(Observed::Health(HealthState::Unknown), 0)
            .is_none());
        let t = f
            .observe(
                Observed::Health(HealthState::Unknown),
                Thresholds::default().fail_after_ms,
            )
            .expect("unknown is not healthy");
        assert_eq!(t.to, Backend::Ip);
    }

    /// **RULE 2, and mutation (a): ONE failed transmit changes nothing.**
    ///
    /// A NAK is the normal texture of a bus whose television is off. Failing
    /// over on one would make every powered-down evening a backend change.
    ///
    /// Mutation-check (run 2026-09-14): make `record`'s `TxError` arm set
    /// `deaf = true` unconditionally (equivalently, `tx_error_threshold = 1`) and
    /// this fails on the first assertion; `Thresholds::validate` refuses the
    /// config form of the same mutation.
    #[test]
    fn a_single_failed_transmit_does_not_fail_over() {
        // Isolated failures, each an eternity from the last — the texture of a
        // bus whose television is switched off. **Written this way on purpose:**
        // asserting on three failures in quick succession would pass even with
        // the rule deleted, because the hysteresis alone delays the commit. The
        // gap is what makes each failure a run of ONE, so a mutation that trips
        // on a single NAK fails here on the second round.
        let window = Thresholds::default().tx_error_window_ms;
        let mut f = healthy();
        for round in 0..5u64 {
            assert!(f.observe(Observed::TxError, round * window * 10).is_none());
            assert_eq!(
                f.active(),
                Backend::Cec,
                "one NAK is not a failover (round {round})"
            );
        }

        // Three in a row, inside one window, do trip the rule — so it is not
        // vacuously passing. The hysteresis still has to be served.
        let mut f = healthy();
        assert!(f.observe(Observed::TxError, 0).is_none());
        assert!(f.observe(Observed::TxError, 100).is_none());
        assert!(f.observe(Observed::TxError, 200).is_none());
        assert_eq!(f.active(), Backend::Cec, "the hysteresis is not yet served");
        let t = f
            .observe(Observed::TxError, 200 + Thresholds::default().fail_after_ms)
            .expect("a run of failures that holds must fail over");
        assert_eq!(t.to, Backend::Ip);
        assert!(
            t.reason.contains("nothing heard from the bus"),
            "the reason must name the observation: {}",
            t.reason
        );
    }

    /// **RULE 2's other half, and mutation (e): a run of transmit failures with
    /// receive traffic in the same window is NOT evidence about us.**
    ///
    /// Hearing the bus is proof the adapter still works, which makes a transmit
    /// failure a fact about the device that did not answer.
    ///
    /// Mutation-check (run 2026-09-14): drop the `&& !heard_in_window`
    /// condition from `record` and this fails on the first block.
    #[test]
    fn transmit_failures_with_traffic_in_the_window_do_not_fail_over() {
        // A wide window and a short failing-edge hysteresis, so a run of
        // failures INSIDE one window can outlast the hysteresis. **Written this
        // way on purpose:** at the default thresholds the hysteresis outlasts
        // the window, so a run that trips this rule could never commit anyway
        // and the test would pass with the condition deleted. It is also why
        // interleaving an `Rx` between every failure is not enough on its own —
        // each message clears the flag by itself, whatever this condition says.
        let thresholds = Thresholds {
            tx_error_window_ms: 60_000,
            fail_after_ms: 1_000,
            ..Thresholds::default()
        };
        let build = || Failover::at_open(thresholds, &ip(), HealthState::Healthy);

        let mut f = build();
        f.observe(Observed::Rx, 0);
        for at in [1_000, 2_000, 3_000, 5_000] {
            f.observe(Observed::TxError, at);
        }
        assert_eq!(
            f.active(),
            Backend::Cec,
            "a bus we heard from inside this window is not a deaf adapter"
        );

        // Ten failures with the bus talking throughout is the same answer.
        let mut f = build();
        for i in 0..10u64 {
            f.observe(Observed::TxError, i * 100);
            f.observe(Observed::Rx, i * 100 + 10);
        }
        f.observe(Observed::TxError, 10_000);
        assert_eq!(f.active(), Backend::Cec);

        // …and the SAME run, with the last message now older than the window,
        // does fail over — so the rule is not vacuously passing, and it is the
        // recency of the traffic that decides.
        let mut f = build();
        f.observe(Observed::Rx, 0);
        for at in [61_000, 62_000, 63_000, 65_000] {
            f.observe(Observed::TxError, at);
        }
        assert_eq!(f.active(), Backend::Ip);

        // And the same run with silence does trip it, so the rule is not
        // vacuously passing.
        let mut deaf = healthy();
        for i in 0..3u64 {
            deaf.observe(Observed::TxError, i * 100);
        }
        assert!(deaf
            .observe(Observed::TxError, Thresholds::default().fail_after_ms + 200)
            .is_some());
    }

    /// **The rule: an ACCEPTED transmit ends the run.** The rule counts
    /// consecutive failures, and one that succeeded is not one of them.
    ///
    /// Mutation-check (run 2026-09-14): drop the counter reset from `record`'s
    /// `TxOk` arm and this fails — four failures either side of a success would
    /// trip a threshold of three.
    #[test]
    fn an_accepted_transmit_ends_the_run_of_failures() {
        let mut f = healthy();
        f.observe(Observed::TxError, 0);
        f.observe(Observed::TxError, 10);
        f.observe(Observed::TxOk, 20);
        // Two more: the run restarted, so this is two, not four.
        f.observe(Observed::TxError, 30);
        f.observe(Observed::TxError, 40);
        assert!(f
            .observe(
                Observed::Health(HealthState::Healthy),
                Thresholds::default().fail_after_ms + 100
            )
            .is_none());
        assert_eq!(f.active(), Backend::Cec);
    }

    /// Failures spread beyond the window are separate runs, not one long one.
    #[test]
    fn transmit_failures_outside_the_window_are_a_fresh_run() {
        let window = Thresholds::default().tx_error_window_ms;
        let mut f = healthy();
        f.observe(Observed::TxError, 0);
        f.observe(Observed::TxError, window + 1);
        f.observe(Observed::TxError, 2 * window + 2);
        assert_eq!(
            f.active(),
            Backend::Cec,
            "three failures hours apart are not a wedged adapter"
        );
    }

    /// **RULE 3, and mutation (b): a blip flips nothing.**
    ///
    /// The degraded verdict has to hold for `fail_after_ms`. A degraded reading
    /// followed by a healthy one inside that window — an adapter whose
    /// addressing was momentarily `unknown` across a `StateChange`, which is the
    /// COMMON case, since every television standby produces one — must leave the
    /// backend alone.
    ///
    /// Mutation-check (run 2026-09-14): commit the change as soon as `desired`
    /// differs (delete the `pending`/`hold` arithmetic on the failing edge) and
    /// this fails on the first assertion.
    #[test]
    fn a_blip_is_suppressed_on_the_failing_edge() {
        let mut f = healthy();
        for round in 0..20u64 {
            let t = round * 1_000;
            // TWO degraded readings, not one: a `StateChange` marks the
            // addressing unknown and the immediate re-read confirms it, so a
            // pair milliseconds apart is what the receive loop really produces.
            // Asserting on a single reading would pass with the hysteresis set
            // to zero, since one observation only ever starts the clock.
            assert!(f
                .observe(Observed::Health(HealthState::Unknown), t)
                .is_none());
            assert!(f
                .observe(Observed::Health(HealthState::Unknown), t + 50)
                .is_none());
            assert!(f
                .observe(Observed::Health(HealthState::Healthy), t + 500)
                .is_none());
            assert_eq!(
                f.active(),
                Backend::Cec,
                "a 500 ms blip must not change the backend (round {round})"
            );
        }
    }

    /// **RULE 3 on the other edge, and the other half of mutation (b): a
    /// momentary recovery does not flip back.**
    ///
    /// Mutation-check (run 2026-09-14): use `recover_after_ms = 0`, or commit a
    /// recovery on the first healthy observation, and this fails on the
    /// mid-round assertion.
    #[test]
    fn a_blip_is_suppressed_on_the_recovering_edge() {
        let mut f = healthy();
        fail_over(&mut f, 0);

        for round in 0..20u64 {
            let t = 100_000 + round * 1_000;
            // Two healthy readings milliseconds apart, for the same reason the
            // failing-edge test uses two: one observation only ever starts the
            // clock, so a single reading would pass with the hysteresis at zero.
            assert!(f
                .observe(Observed::Health(HealthState::Healthy), t)
                .is_none());
            assert!(f
                .observe(Observed::Health(HealthState::Healthy), t + 50)
                .is_none());
            assert_eq!(
                f.active(),
                Backend::Ip,
                "one healthy reading is not a recovery (round {round})"
            );
            // …and it goes away again before the window is served.
            assert!(f
                .observe(Observed::Health(HealthState::Degraded), t + 500)
                .is_none());
            assert_eq!(f.active(), Backend::Ip);
        }
    }

    /// **RULE 4, and mutation (c): recovery is committed by a fresh HEALTH
    /// observation, never by elapsed time.**
    ///
    /// The three non-health observations are all things that can be true while
    /// the adapter is still unusable, and none of them is the re-read of facts 1
    /// and 2 that a recovery has to be evidenced by. The `Rx` at the far end of
    /// the test carries a timestamp long past the recovery threshold: a
    /// timer-driven implementation flips there, and this test is what catches
    /// it.
    ///
    /// Mutation-check (run 2026-09-14): drop the `may_commit` guard (so any
    /// observation can commit a recovery) and this fails on the `Rx`/`TxOk`
    /// assertions; make `active()` take `now_ms` and compare it against the
    /// pending timestamp and it fails the same way.
    #[test]
    fn recovery_needs_a_fresh_healthy_observation_and_not_merely_elapsed_time() {
        let mut f = healthy();
        fail_over(&mut f, 0);
        let recover = Thresholds::default().recover_after_ms;

        // The adapter is healthy again, and says so once. The recovery clock
        // starts here.
        assert!(f
            .observe(Observed::Health(HealthState::Healthy), 100_000)
            .is_none());

        // An eternity later, the only things arriving are bus traffic and
        // accepted transmits. Neither may recover the backend, however long it
        // has been.
        assert!(f.observe(Observed::Rx, 100_000 + recover * 100).is_none());
        assert_eq!(f.active(), Backend::Ip, "traffic is not a health re-read");
        assert!(f.observe(Observed::TxOk, 100_000 + recover * 200).is_none());
        assert_eq!(f.active(), Backend::Ip);

        // A fresh healthy verdict, past the threshold, is what commits it.
        let t = f
            .observe(
                Observed::Health(HealthState::Healthy),
                100_000 + recover * 300,
            )
            .expect("a re-observed healthy adapter must recover");
        assert_eq!(t.from, Backend::Ip);
        assert_eq!(t.to, Backend::Cec);
        assert!(t.reason.contains("healthy"), "{}", t.reason);
        assert_eq!(f.active(), Backend::Cec);
    }

    /// **The rule: with no IP leg configured, a degraded adapter changes nothing
    /// — and the reason says why.**
    ///
    /// Announcing an `ip` backend that does not exist is the same class of claim
    /// as rendering `unknown` as healthy.
    #[test]
    fn with_no_ip_leg_configured_there_is_nowhere_to_fail_over_to() {
        let mut f = Failover::at_open(
            Thresholds::default(),
            &IpConfig::default(),
            HealthState::Healthy,
        );
        assert_eq!(f.available(), vec![Backend::Cec]);

        for i in 0..5u64 {
            assert!(f
                .observe(
                    Observed::Health(HealthState::Degraded),
                    i * Thresholds::default().fail_after_ms
                )
                .is_none());
        }
        assert_eq!(f.active(), Backend::Cec);
        let report = f.report();
        assert_eq!(report.active, Backend::Cec);
        assert!(
            report.reason.contains("no IP leg is configured"),
            "{}",
            report.reason
        );
        assert!(report.reason.contains("degraded"), "{}", report.reason);
    }

    /// **The rule: a pin overrides the observations, and says it is doing so.**
    #[test]
    fn a_pin_overrides_the_automatic_decision_and_names_itself() {
        let mut f = healthy();
        fail_over(&mut f, 0);
        assert_eq!(f.active(), Backend::Ip);

        f.set_pin(Pin::Cec).unwrap();
        assert_eq!(f.active(), Backend::Cec);
        let report = f.report();
        assert_eq!(report.pin, Pin::Cec);
        assert!(
            report.reason.starts_with("pinned to cec"),
            "{}",
            report.reason
        );
        assert!(
            report.reason.contains("would be ip"),
            "the automatic answer must still be visible: {}",
            report.reason
        );

        f.set_pin(Pin::Auto).unwrap();
        assert_eq!(f.active(), Backend::Ip, "auto returns to the observations");
    }

    /// A pin to a backend this box does not have is refused, not accepted and
    /// ignored.
    #[test]
    fn pinning_to_an_absent_backend_is_refused() {
        let mut f = Failover::at_open(
            Thresholds::default(),
            &IpConfig::default(),
            HealthState::Healthy,
        );
        let e = f.set_pin(Pin::Ip).expect_err("there is no ip backend");
        assert!(e.contains("no IP leg is configured"), "{e}");
        assert_eq!(f.pin(), Pin::Auto, "a refused pin must not take effect");
        // Pinning to the one that does exist is fine.
        f.set_pin(Pin::Cec).unwrap();
    }

    /// **THE CONFIG RULE: every threshold in `cec.toml` changes a decision.**
    ///
    /// `core.toml`'s `[supervisor].restart_threshold` / `restart_window_secs`
    /// are written and read by nothing, and the core's own suite lists them as
    /// unconsumed. A second unconsumed knob would be the same mistake, so this
    /// test drives each of the four to two values and asserts the outcome
    /// differs. **Delete a field's only reader and this test fails**, which is
    /// what makes "consumed" an assertion rather than a claim.
    #[test]
    fn every_threshold_changes_a_decision() {
        let base = Thresholds::default();

        // 1. tx_error_threshold — how many failures it takes.
        let run = |threshold: u32| {
            let mut f = Failover::at_open(
                Thresholds {
                    tx_error_threshold: threshold,
                    ..base
                },
                &ip(),
                HealthState::Healthy,
            );
            for i in 0..3u64 {
                f.observe(Observed::TxError, i);
            }
            f.observe(Observed::TxError, base.fail_after_ms + 10);
            f.active()
        };
        assert_eq!(run(3), Backend::Ip);
        assert_eq!(run(9), Backend::Cec, "tx_error_threshold is unread");

        // 2. tx_error_window_ms — how far apart failures may be.
        let run = |window: u64| {
            let mut f = Failover::at_open(
                Thresholds {
                    tx_error_window_ms: window,
                    ..base
                },
                &ip(),
                HealthState::Healthy,
            );
            for i in 0..4u64 {
                f.observe(Observed::TxError, i * 5_000);
            }
            f.observe(Observed::TxError, 4 * 5_000 + base.fail_after_ms);
            f.active()
        };
        assert_eq!(run(10_000), Backend::Ip);
        assert_eq!(run(1_000), Backend::Cec, "tx_error_window_ms is unread");

        // 3. fail_after_ms — how long a failing reading must hold.
        let run = |fail_after: u64| {
            let mut f = Failover::at_open(
                Thresholds {
                    fail_after_ms: fail_after,
                    ..base
                },
                &ip(),
                HealthState::Healthy,
            );
            f.observe(Observed::Health(HealthState::Degraded), 0);
            f.observe(Observed::Health(HealthState::Degraded), 5_000);
            f.active()
        };
        assert_eq!(run(1_000), Backend::Ip);
        assert_eq!(run(60_000), Backend::Cec, "fail_after_ms is unread");

        // 4. recover_after_ms — how long health must hold to come back.
        let run = |recover: u64| {
            let mut f = Failover::at_open(
                Thresholds {
                    recover_after_ms: recover,
                    ..base
                },
                &ip(),
                HealthState::Healthy,
            );
            f.observe(Observed::Health(HealthState::Degraded), 0);
            f.observe(Observed::Health(HealthState::Degraded), base.fail_after_ms);
            assert_eq!(f.active(), Backend::Ip);
            f.observe(Observed::Health(HealthState::Healthy), 100_000);
            f.observe(Observed::Health(HealthState::Healthy), 110_000);
            f.active()
        };
        assert_eq!(run(5_000), Backend::Cec);
        assert_eq!(run(600_000), Backend::Ip, "recover_after_ms is unread");
    }

    /// The validator refuses the two configurations that would delete a rule.
    #[test]
    fn the_thresholds_refuse_a_configuration_that_deletes_a_rule() {
        Thresholds::default().validate().unwrap();
        for bad in [
            Thresholds {
                tx_error_threshold: 1,
                ..Thresholds::default()
            },
            Thresholds {
                tx_error_threshold: 0,
                ..Thresholds::default()
            },
            Thresholds {
                fail_after_ms: 0,
                ..Thresholds::default()
            },
            Thresholds {
                recover_after_ms: 0,
                ..Thresholds::default()
            },
            Thresholds {
                tx_error_window_ms: 0,
                ..Thresholds::default()
            },
        ] {
            assert!(bad.validate().is_err(), "{bad:?}");
        }
        assert!(Thresholds {
            tx_error_threshold: 1,
            ..Thresholds::default()
        }
        .validate()
        .unwrap_err()
        .contains("SINGLE failed transmit"));
    }

    /// A clock that steps backwards cannot commit a change early or panic.
    #[test]
    fn a_backwards_clock_is_not_a_panic() {
        let mut f = healthy();
        f.observe(Observed::Health(HealthState::Degraded), 10_000);
        assert!(f
            .observe(Observed::Health(HealthState::Degraded), 5_000)
            .is_none());
        assert_eq!(f.active(), Backend::Cec);
    }

    /// The published shape, whole.
    #[test]
    fn the_report_carries_every_documented_key() {
        let json: serde_json::Value =
            serde_json::from_str(&serde_json::to_string(&healthy().report()).unwrap()).unwrap();
        for field in ["active", "available", "pin", "reason"] {
            assert!(
                json.as_object().unwrap().contains_key(field),
                "backend must carry {field}: {json}"
            );
        }
        assert_eq!(json["active"], serde_json::json!("cec"));
        assert_eq!(json["available"], serde_json::json!(["cec", "ip"]));
        assert_eq!(json["pin"], serde_json::json!("auto"));
    }

    #[test]
    fn the_tokens_are_stable() {
        assert_eq!(Backend::Cec.as_str(), "cec");
        assert_eq!(Backend::Ip.as_str(), "ip");
        assert_eq!(Pin::parse("auto"), Some(Pin::Auto));
        assert_eq!(Pin::parse("cec"), Some(Pin::Cec));
        assert_eq!(Pin::parse("ip"), Some(Pin::Ip));
        for bad in ["", "CEC", "kernel", "libcec", "auto ", "none"] {
            assert_eq!(Pin::parse(bad), None, "{bad:?}");
        }
    }
}
