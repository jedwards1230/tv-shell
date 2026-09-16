//! The published AV snapshot — **what was observed, and when**.
//!
//! # The one rule this module exists to enforce
//!
//! **`unknown` is a first-class value, and it is never rendered as healthy and
//! never as `false`.** Every field here is an [`Observation`], which serializes
//! to JSON `null` until something actually observed it. A daemon that reported
//! `weAreSource: false` because nothing had told it otherwise would be making a
//! claim it cannot support — and a consumer cannot tell that apart from a
//! genuine "someone else holds the display".
//!
//! `daemon/src/display_owner.rs` already articulates why this has to be a
//! tri-state rather than a boolean with a default: **the fail-safe direction
//! inverts depending on the consumer.** A caller deciding whether to send
//! `<Standby>` wants "unknown ⇒ do not" and a caller deciding whether to claim
//! the display wants "unknown ⇒ go ahead". Neither default is right for both, so
//! this daemon publishes the tri-state and each consumer picks its own safe
//! side. That reasoning is ported here deliberately; it is the best thing in the
//! v1 CEC code.
//!
//! # Why the folding lives here and not in the rx loop
//!
//! [`Observations::apply`] takes a [`BusObservation`] — **this crate's own
//! type**, carrying no `linux-cec` types at all — so the whole "what does a bus
//! event mean for the published state" decision is a pure function that CI can
//! cover on a runner with no adapter. `kernel::follower` does the translation
//! and nothing else. That is the same split `core/` uses between its pure
//! decision modules and its I/O, and it is what makes the seam in
//! [`crate::backend`] a real one.

use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Serialize, Serializer};

/// A CEC physical address — the `a.b.c.d` port path of a device in the HDMI
/// topology.
///
/// Pure, so `cec.toml` parsing and the `weAreSource` comparison are both covered
/// with no device present. The kernel layer converts this to `linux-cec`'s own
/// `PhysicalAddress` at the seam; that type never reaches [`AvState`] or the IPC
/// layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PhysAddr(u16);

impl PhysAddr {
    /// `f.f.f.f`, the address the CEC specification reserves for "invalid".
    pub const INVALID: PhysAddr = PhysAddr(0xFFFF);

    /// Build from the four nibbles of the port path.
    #[must_use]
    pub const fn from_nibbles(a: u8, b: u8, c: u8, d: u8) -> PhysAddr {
        PhysAddr(
            ((a as u16 & 0xF) << 12)
                | ((b as u16 & 0xF) << 8)
                | ((c as u16 & 0xF) << 4)
                | (d as u16 & 0xF),
        )
    }

    /// The raw 16-bit encoding, as the kernel carries it.
    #[must_use]
    pub const fn raw(self) -> u16 {
        self.0
    }

    /// From the kernel's 16-bit encoding.
    #[must_use]
    pub const fn from_raw(raw: u16) -> PhysAddr {
        PhysAddr(raw)
    }

    /// `f.f.f.f` means the adapter has no address — it is not a port path.
    #[must_use]
    pub const fn is_valid(self) -> bool {
        self.0 != 0xFFFF
    }
}

impl std::fmt::Display for PhysAddr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{:x}.{:x}.{:x}.{:x}",
            self.0 >> 12,
            (self.0 >> 8) & 0xF,
            (self.0 >> 4) & 0xF,
            self.0 & 0xF
        )
    }
}

impl Serialize for PhysAddr {
    /// As the `a.b.c.d` text form, never as the raw 16-bit integer.
    ///
    /// The dotted form is what an operator reads off `cec-ctl --show-topology`,
    /// and a bare `9472` would be unrecognisable beside it.
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_string())
    }
}

impl std::str::FromStr for PhysAddr {
    type Err = String;

    /// Exactly `a.b.c.d`, four single hex digits.
    ///
    /// Deliberately strict: a physical address that does not name a real port
    /// makes a later `<Active Source>` address a port that does not exist, and
    /// the CEC bus reports nothing when that happens. A typo must fail at
    /// startup, not silently.
    fn from_str(s: &str) -> Result<PhysAddr, String> {
        let parts: Vec<&str> = s.split('.').collect();
        if parts.len() != 4 {
            return Err(format!(
                "{s:?} is not a physical address of the form a.b.c.d"
            ));
        }
        let mut nibbles = [0u8; 4];
        for (slot, part) in nibbles.iter_mut().zip(parts) {
            if part.len() != 1 {
                return Err(format!(
                    "{s:?}: each place of a physical address is one hex digit"
                ));
            }
            *slot = u8::from_str_radix(part, 16)
                .map_err(|_| format!("{s:?}: {part:?} is not a hex digit"))?;
        }
        Ok(PhysAddr::from_nibbles(
            nibbles[0], nibbles[1], nibbles[2], nibbles[3],
        ))
    }
}

/// One field of the snapshot: observed, or honestly unknown.
///
/// Serializes as the value when known and as `null` when not. **There is no
/// other rendering** — see the module docs. `Option` would behave the same way
/// today; this is a named type because the rule is the point, and a named type
/// is where the rule can carry its reasoning and its test.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Observation<T> {
    /// Nothing has told us. Never `false`, never zero, never "ok".
    #[default]
    Unknown,
    /// Observed.
    Known(T),
}

impl<T> Observation<T> {
    /// Whether anything has been observed yet.
    #[must_use]
    pub const fn is_known(&self) -> bool {
        matches!(self, Observation::Known(_))
    }

    /// The observed value, if there is one.
    #[must_use]
    pub const fn get(&self) -> Option<&T> {
        match self {
            Observation::Known(v) => Some(v),
            Observation::Unknown => None,
        }
    }
}

impl<T> From<Option<T>> for Observation<T> {
    fn from(v: Option<T>) -> Observation<T> {
        match v {
            Some(v) => Observation::Known(v),
            None => Observation::Unknown,
        }
    }
}

impl<T: Serialize> Serialize for Observation<T> {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self {
            Observation::Known(v) => v.serialize(s),
            // The rule, in one line. An `unknown` boolean must not reach the
            // wire as `false`, and an `unknown` level must not reach it as `0`.
            Observation::Unknown => s.serialize_none(),
        }
    }
}

/// Which device on the bus a power reading is about.
///
/// Only the two this daemon has any business tracking. A reading from anything
/// else on the bus (a shared bus may carry other playback devices) is
/// recorded as [`AvDevice::Other`] and folded into nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AvDevice {
    /// The television.
    Tv,
    /// The audio system — the AVR.
    AudioSystem,
    /// Something else on the shared bus.
    Other,
}

/// A device's CEC power state, as reported by that device.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum PowerState {
    On,
    Standby,
    /// Transitioning from standby to on.
    ToOn,
    /// Transitioning from on to standby.
    ToStandby,
}

/// One thing the bus told us.
///
/// This crate's own vocabulary. `kernel::follower` maps `linux-cec`'s
/// `PollResult` onto it; nothing downstream of here knows that crate exists.
#[derive(Debug, Clone, PartialEq)]
pub enum BusObservation {
    /// `<Active Source>` — the named address now drives the display.
    ActiveSource(PhysAddr),
    /// `<Inactive Source>` — the named address has given the display up.
    ///
    /// Note this does **not** say who has it now, so it makes the active source
    /// unknown rather than making it anything in particular.
    InactiveSource(PhysAddr),
    /// `<Report Power Status>` from a device we track.
    PowerStatus { device: AvDevice, state: PowerState },
    /// `<Report Audio Status>` — the AVR's volume and mute.
    ///
    /// **The volume is itself an [`Observation`]**, because the wire has a value
    /// for "I do not know": CEC's 7-bit audio-volume field reserves `0x7F` for
    /// *"audio volume status unknown"* and defines only `0..=100` as levels. An
    /// AVR that has dropped out of system-audio mode really does send it. The
    /// mute flag is one bit and always means something, so it is a plain
    /// `bool`. See [`crate::volume::level_observation`].
    AudioStatus {
        volume: Observation<u8>,
        muted: bool,
    },
    /// The kernel says the adapter's configuration changed: it gained or lost
    /// its physical or logical address. Everything cached about the topology is
    /// stale from here.
    StateChange,
    /// The kernel's rx queue overflowed and dropped messages.
    ///
    /// Recorded because it means the snapshot may have missed an edge — not
    /// because anything acts on it yet.
    LostMessages(u32),
}

/// The mutable half: what has been observed so far, each with its own timestamp.
///
/// Held behind a plain `Mutex` rather than atomics. The v1 model was
/// lock-free because it lived inside a libcec callback thread; under the kernel
/// API the rx loop is an ordinary tokio task, so that constraint is gone. The
/// tri-state and the timestamps are what carried over.
#[derive(Debug, Clone, Default)]
pub struct Observations {
    active_source: Observation<PhysAddr>,
    tv_power: Observation<PowerState>,
    avr_power: Observation<PowerState>,
    volume: Observation<u8>,
    muted: Observation<bool>,
    /// Unix ms of the most recent observation of any kind, or of the last
    /// [`BusObservation::StateChange`]. `None` means nothing has been heard.
    observed_at: Option<u64>,
    /// Unix ms when `active_source` last **changed**.
    ///
    /// Stamped only on a real change, not on every repeat broadcast — otherwise
    /// "how long has the current owner held the display" would reset on each
    /// periodic re-announce. Ported from v1's `DisplayOwner::store_owner`, which
    /// made the same distinction for the same reason.
    active_source_changed_at: Option<u64>,
    /// Whether an ownership claim has ever been RECEIVED from the bus.
    ///
    /// Set by `<Active Source>` and `<Inactive Source>` only, never by a claim
    /// of our own ([`Observations::record_our_claim`]): its whole job is to
    /// answer "does this bus actually broadcast ownership?", and a self-claim
    /// would make that always true. Without it, "we are listening and this bus
    /// never announces ownership" and "we are not listening" read identically
    /// from outside.
    ever_observed_claim: bool,
    /// Count of messages the kernel told us it dropped.
    lost_messages: u64,
}

impl Observations {
    /// Fold one bus observation in, stamping it at `now_ms`.
    ///
    /// Pure: no I/O, no clock read. The caller supplies the time, so the rules
    /// are testable without one.
    pub fn apply(&mut self, obs: BusObservation, now_ms: u64) {
        self.observed_at = Some(now_ms);
        match obs {
            BusObservation::ActiveSource(addr) => {
                self.ever_observed_claim = true;
                self.store_active_source(Observation::Known(addr), now_ms);
            }
            // Who holds the display AFTER a release is not stated by the
            // message, so this returns the field to `unknown` rather than
            // guessing. Inventing "nobody" here is exactly the shape of claim
            // this module exists to refuse.
            BusObservation::InactiveSource(_) => {
                self.ever_observed_claim = true;
                self.store_active_source(Observation::Unknown, now_ms);
            }
            BusObservation::PowerStatus { device, state } => match device {
                AvDevice::Tv => self.tv_power = Observation::Known(state),
                AvDevice::AudioSystem => self.avr_power = Observation::Known(state),
                AvDevice::Other => {}
            },
            // A report the AVR could not fill in returns the published level to
            // `unknown` rather than leaving the previous number in place: the
            // AVR has just said it does not know its own volume, and carrying a
            // stale value forward would publish it as current.
            BusObservation::AudioStatus { volume, muted } => {
                self.volume = volume;
                self.muted = Observation::Known(muted);
            }
            // The adapter was reconfigured under us. Everything observed about
            // the bus is now suspect, so it all goes back to `unknown` rather
            // than being carried forward as if it were still current.
            BusObservation::StateChange => {
                self.store_active_source(Observation::Unknown, now_ms);
                self.tv_power = Observation::Unknown;
                self.avr_power = Observation::Unknown;
                self.volume = Observation::Unknown;
                self.muted = Observation::Unknown;
            }
            BusObservation::LostMessages(n) => {
                self.lost_messages += u64::from(n);
            }
        }
    }

    /// Swap in a new active source, stamping the change time only on a real
    /// change.
    ///
    /// A repeat `<Active Source>` for the same address is not a transition, so
    /// it must not move the timestamp — otherwise "how long has this device held
    /// the display" resets on every periodic re-announce. v1's
    /// `DisplayOwner::store_owner` drew the same line.
    fn store_active_source(&mut self, next: Observation<PhysAddr>, now_ms: u64) {
        if self.active_source == next {
            return;
        }
        self.active_source = next;
        self.active_source_changed_at = Some(now_ms);
    }

    /// Record an ownership claim **we made ourselves**, which never comes back
    /// through the receive loop.
    ///
    /// Deliberately does NOT set the ever-observed flag and does NOT move
    /// `observed_at`: neither is a thing the bus told us, and conflating the two
    /// would make "this bus announces ownership" always true and "the bus said
    /// something" true on a bus that said nothing.
    pub fn record_our_claim(&mut self, ours: PhysAddr, now_ms: u64) {
        self.store_active_source(Observation::Known(ours), now_ms);
    }

    /// Record that we gave the display up, for the same reason as
    /// [`Observations::record_our_claim`].
    ///
    /// Goes to `unknown`, not to "nobody": `<Inactive Source>` says a device
    /// released the display, never who has it now.
    pub fn record_our_release(&mut self, now_ms: u64) {
        self.store_active_source(Observation::Unknown, now_ms);
    }

    /// The current display-ownership report, given our own physical address.
    ///
    /// The verdict is derived by [`crate::ownership::classify`] and published
    /// beside the two addresses it came from, so a consumer that wants to apply
    /// its own fail-safe direction can.
    #[must_use]
    pub fn ownership(&self, ours: Observation<PhysAddr>) -> crate::ownership::OwnershipReport {
        crate::ownership::OwnershipReport {
            state: crate::ownership::classify(self.active_source, ours),
            owner: self.active_source,
            ours,
            changed_at: self.active_source_changed_at,
            ever_observed: self.ever_observed_claim,
        }
    }

    /// The last observed active source, for the pure gates in
    /// [`crate::ownership`].
    #[must_use]
    pub const fn active_source(&self) -> Observation<PhysAddr> {
        self.active_source
    }

    /// How many messages the kernel has reported dropping.
    #[must_use]
    pub const fn lost_messages(&self) -> u64 {
        self.lost_messages
    }

    /// The last audio level the AVR reported, for `volume-state`'s fallback.
    #[must_use]
    pub const fn volume(&self) -> Observation<u8> {
        self.volume
    }

    /// The last mute flag the AVR reported.
    #[must_use]
    pub const fn muted(&self) -> Observation<bool> {
        self.muted
    }

    /// Unix ms of the most recent observation of any kind, or `None`.
    #[must_use]
    pub const fn observed_at(&self) -> Option<u64> {
        self.observed_at
    }

    /// Whether `ours` currently holds the display.
    ///
    /// **`unknown` in, `unknown` out.** With no `<Active Source>` observed there
    /// is no answer, and `false` is not it: `false` would tell a caller that
    /// somebody else holds the display, which is a different fact.
    #[must_use]
    pub fn we_are_source(&self, ours: Observation<PhysAddr>) -> Observation<bool> {
        match (self.active_source, ours) {
            (Observation::Known(active), Observation::Known(ours)) => {
                Observation::Known(active == ours)
            }
            _ => Observation::Unknown,
        }
    }
}

/// The static half: what the adapter is, established once at open.
///
/// Separate from [`Observations`] because these are things we *configured or
/// read back*, not things the bus told us.
#[derive(Debug, Clone)]
pub struct Topology {
    /// Which backend produced this snapshot. `"cec"` today; the IP leg (step 7)
    /// adds a second value and `backend`/`backend-pin` verbs to choose.
    pub backend: &'static str,
    /// The device node actually opened.
    pub device: String,
    /// The physical address this daemon **asked** the adapter to take.
    pub phys_addr_configured: PhysAddr,
    /// The physical address `CEC_ADAP_G_PHYS_ADDR` **read back**.
    ///
    /// Published beside the configured value on purpose. `2.5.0.0` is the
    /// pre-2026-08-07 value and is UNVERIFIED against the current rack (plan
    /// §7 item 3); the adapter's own port has no readable EDID, so it cannot be
    /// derived. Reporting both is what makes a wrong value visible instead of
    /// silently addressing a port that does not exist.
    pub phys_addr_read_back: Observation<PhysAddr>,
    /// The logical addresses the adapter holds, as read back.
    pub log_addrs: Vec<String>,
    /// The capability flags `CEC_ADAP_G_CAPS` actually reported.
    pub capabilities: Vec<String>,
    /// Whether `CEC_CAP_MONITOR_PIN` is among them.
    ///
    /// **Read, never assumed** (plan §7 item 7): whether `pulse8-cec`
    /// implements the pin monitor could not be checked without a device. It is
    /// the one signal that separates "the bus is quiet because everything is
    /// off" from "our adapter has stopped hearing", so step 6's `av-health`
    /// consumes this to name which health signal is in force.
    pub monitor_pin: bool,
}

/// One `av-state` reply, as JSON.
///
/// Every field is "what was observed and when", never an inferred verdict. The
/// shape is shipped whole from this first step — with honest `null`s for what
/// this step cannot observe — rather than growing and changing shape later.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AvState {
    pub backend: &'static str,
    pub device: String,
    /// The address read back from the adapter, or `null` if the read failed.
    pub phys_addr: Observation<String>,
    /// The address this daemon set. See [`Topology::phys_addr_read_back`].
    pub phys_addr_configured: String,
    pub log_addrs: Vec<String>,
    pub capabilities: Vec<String>,
    pub monitor_pin: bool,
    pub tv_power: Observation<PowerState>,
    pub avr_power: Observation<PowerState>,
    pub active_source: Observation<String>,
    pub we_are_source: Observation<bool>,
    /// The full display-ownership report — the tri-state plus the two addresses
    /// it was derived from, when it last changed, and whether the bus has ever
    /// announced ownership at all.
    ///
    /// Published **beside** `weAreSource` rather than instead of it. They answer
    /// different questions: `weAreSource` is a plain observation, while
    /// `displayOwnership.state` is the verdict the transmit gates act on, and a
    /// consumer whose fail-safe direction is the opposite one (suspend-when-
    /// nobody-is-watching) needs the tri-state and the `everObserved` flag, not
    /// a boolean.
    pub display_ownership: crate::ownership::OwnershipReport,
    pub volume: Observation<u8>,
    pub muted: Observation<bool>,
    /// Unix ms of the most recent bus observation, or `null` if the bus has
    /// said nothing at all yet.
    pub observed_at: Option<u64>,
    /// Messages the kernel reported dropping since start.
    pub lost_messages: u64,
}

impl AvState {
    /// Assemble a snapshot from the static topology and the observed state.
    #[must_use]
    pub fn assemble(topology: &Topology, obs: &Observations) -> AvState {
        AvState {
            backend: topology.backend,
            device: topology.device.clone(),
            phys_addr: match topology.phys_addr_read_back {
                Observation::Known(a) => Observation::Known(a.to_string()),
                Observation::Unknown => Observation::Unknown,
            },
            phys_addr_configured: topology.phys_addr_configured.to_string(),
            log_addrs: topology.log_addrs.clone(),
            capabilities: topology.capabilities.clone(),
            monitor_pin: topology.monitor_pin,
            tv_power: obs.tv_power,
            avr_power: obs.avr_power,
            active_source: match obs.active_source {
                Observation::Known(a) => Observation::Known(a.to_string()),
                Observation::Unknown => Observation::Unknown,
            },
            we_are_source: obs.we_are_source(topology.phys_addr_read_back),
            display_ownership: obs.ownership(topology.phys_addr_read_back),
            volume: obs.volume,
            muted: obs.muted,
            observed_at: obs.observed_at,
            lost_messages: obs.lost_messages,
        }
    }
}

/// Wall-clock milliseconds since the Unix epoch.
///
/// A clock that is before the epoch yields 0 rather than panicking: this is a
/// long-running daemon and a bad clock is not a reason to take it down.
#[must_use]
pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn addr(s: &str) -> PhysAddr {
        s.parse().unwrap()
    }

    fn topology() -> Topology {
        Topology {
            backend: "cec",
            device: "/dev/cec0".into(),
            phys_addr_configured: addr("2.5.0.0"),
            phys_addr_read_back: Observation::Known(addr("2.5.0.0")),
            log_addrs: vec!["playback-device1".into()],
            capabilities: vec!["PHYS_ADDR".into(), "LOG_ADDRS".into()],
            monitor_pin: false,
        }
    }

    #[test]
    fn a_physical_address_round_trips_through_its_text_form() {
        for s in ["0.0.0.0", "2.5.0.0", "1.2.3.4", "f.f.f.f"] {
            assert_eq!(addr(s).to_string(), s);
        }
        assert_eq!(addr("2.5.0.0").raw(), 0x2500);
        assert!(!PhysAddr::INVALID.is_valid());
        assert!(addr("0.0.0.0").is_valid());
    }

    /// A physical address that is not `a.b.c.d` is refused, never coerced.
    ///
    /// The failure mode this guards is silent: a wrong address makes a later
    /// `<Active Source>` name a port that does not exist, and the bus says
    /// nothing about it.
    #[test]
    fn a_malformed_physical_address_is_a_parse_error() {
        for s in [
            "",
            "2.5.0",
            "2.5.0.0.0",
            "25.0.0",
            "2,5,0,0",
            "2.5.0.g",
            "0x2500",
            "2500",
            "2.50.0.0",
            " 2.5.0.0",
        ] {
            assert!(
                s.parse::<PhysAddr>().is_err(),
                "{s:?} must not parse as a physical address"
            );
        }
    }

    /// **The rule: `unknown` reaches the wire as `null` — never `false`, never
    /// `0`, never a verdict.**
    ///
    /// Mutation-check (run 2026-09-14): make `Observation::serialize` emit
    /// `false`/`0` for `Unknown` (or derive it as an untagged enum with a unit
    /// variant) and this fails on the first field.
    #[test]
    fn an_unknown_field_serializes_as_null_and_never_as_false() {
        let obs = Observations::default();
        let mut topology = topology();
        topology.phys_addr_read_back = Observation::Unknown;
        let json: serde_json::Value = serde_json::from_str(
            &serde_json::to_string(&AvState::assemble(&topology, &obs)).unwrap(),
        )
        .unwrap();

        for field in [
            "physAddr",
            "tvPower",
            "avrPower",
            "activeSource",
            "weAreSource",
            "volume",
            "muted",
            "observedAt",
        ] {
            assert_eq!(
                json[field],
                serde_json::Value::Null,
                "{field} must be null while unobserved, got {}",
                json[field]
            );
            assert_ne!(json[field], serde_json::json!(false), "{field}");
            assert_ne!(json[field], serde_json::json!(0), "{field}");
        }
    }

    /// **The rule: `weAreSource` is `unknown` until an `<Active Source>` says
    /// otherwise, and `false` is a different fact.**
    ///
    /// All three states are reachable from the real path: startup before any
    /// traffic (unknown), a broadcast naming somebody else (false), and one
    /// naming us (true).
    ///
    /// Mutation-check (run 2026-09-14): change `we_are_source`'s fallthrough arm
    /// to `Observation::Known(false)` and the first assertion fails.
    #[test]
    fn we_are_source_is_unknown_until_the_bus_says_who_it_is() {
        let ours = Observation::Known(addr("2.5.0.0"));
        let mut obs = Observations::default();
        assert_eq!(obs.we_are_source(ours), Observation::Unknown);

        obs.apply(BusObservation::ActiveSource(addr("1.0.0.0")), 10);
        assert_eq!(obs.we_are_source(ours), Observation::Known(false));

        obs.apply(BusObservation::ActiveSource(addr("2.5.0.0")), 20);
        assert_eq!(obs.we_are_source(ours), Observation::Known(true));
    }

    /// An unknown address of OUR OWN also yields `unknown`, not `false`.
    ///
    /// Reachable: `CEC_ADAP_G_PHYS_ADDR` failing at open leaves
    /// `phys_addr_read_back` unknown while the bus keeps broadcasting.
    #[test]
    fn we_are_source_is_unknown_when_our_own_address_is_unknown() {
        let mut obs = Observations::default();
        obs.apply(BusObservation::ActiveSource(addr("2.5.0.0")), 10);
        assert_eq!(
            obs.we_are_source(Observation::Unknown),
            Observation::Unknown
        );
    }

    /// `<Inactive Source>` says a device gave the display up — not who has it.
    #[test]
    fn an_inactive_source_returns_the_active_source_to_unknown() {
        let mut obs = Observations::default();
        obs.apply(BusObservation::ActiveSource(addr("1.0.0.0")), 10);
        obs.apply(BusObservation::InactiveSource(addr("1.0.0.0")), 20);
        assert_eq!(
            obs.we_are_source(Observation::Known(addr("2.5.0.0"))),
            Observation::Unknown
        );
    }

    /// A reconfigured adapter invalidates everything observed through it.
    #[test]
    fn a_state_change_returns_every_observed_field_to_unknown() {
        let mut obs = Observations::default();
        obs.apply(BusObservation::ActiveSource(addr("2.5.0.0")), 10);
        obs.apply(
            BusObservation::PowerStatus {
                device: AvDevice::Tv,
                state: PowerState::On,
            },
            11,
        );
        obs.apply(
            BusObservation::AudioStatus {
                volume: Observation::Known(42),
                muted: true,
            },
            12,
        );

        obs.apply(BusObservation::StateChange, 13);

        let state = AvState::assemble(&topology(), &obs);
        let json: serde_json::Value =
            serde_json::from_str(&serde_json::to_string(&state).unwrap()).unwrap();
        for field in [
            "tvPower",
            "avrPower",
            "activeSource",
            "weAreSource",
            "volume",
            "muted",
        ] {
            assert_eq!(json[field], serde_json::Value::Null, "{field}");
        }
        // The state change is itself an observation, so the timestamp moves.
        assert_eq!(json["observedAt"], serde_json::json!(13));
    }

    #[test]
    fn power_and_audio_observations_land_on_the_right_fields() {
        let mut obs = Observations::default();
        obs.apply(
            BusObservation::PowerStatus {
                device: AvDevice::Tv,
                state: PowerState::On,
            },
            1,
        );
        obs.apply(
            BusObservation::PowerStatus {
                device: AvDevice::AudioSystem,
                state: PowerState::Standby,
            },
            2,
        );
        obs.apply(
            BusObservation::PowerStatus {
                device: AvDevice::Other,
                state: PowerState::On,
            },
            3,
        );
        obs.apply(
            BusObservation::AudioStatus {
                volume: Observation::Known(0),
                muted: false,
            },
            4,
        );

        let json: serde_json::Value = serde_json::from_str(
            &serde_json::to_string(&AvState::assemble(&topology(), &obs)).unwrap(),
        )
        .unwrap();
        assert_eq!(json["tvPower"], serde_json::json!("on"));
        assert_eq!(json["avrPower"], serde_json::json!("standby"));
        // A volume of 0 and an unmuted AVR are OBSERVED values, and must be
        // distinguishable from "nothing has told us" — which is the whole point
        // of the tri-state.
        assert_eq!(json["volume"], serde_json::json!(0));
        assert_eq!(json["muted"], serde_json::json!(false));
    }

    /// **The rule: an AVR that reports it does not know its own volume returns
    /// the published level to `unknown` — it does not leave the last number in
    /// place and it does not become `0`.**
    ///
    /// Reachable from the real path: CEC's 7-bit audio-volume field reserves
    /// `0x7F` for "audio volume status unknown", which a receiver sends when it
    /// has dropped out of system-audio mode. `kernel::follower` folds it through
    /// `volume::level_observation`, so this is the value the wire produces, not
    /// one a test invented.
    ///
    /// Mutation-check (run 2026-09-14): make the `AudioStatus` arm keep the old
    /// level (`if let Observation::Known(v) = volume { self.volume = ... }`) or
    /// coerce it to `Known(0)` and this fails.
    #[test]
    fn an_avr_that_does_not_know_its_volume_returns_the_level_to_unknown() {
        let mut obs = Observations::default();
        obs.apply(
            BusObservation::AudioStatus {
                volume: Observation::Known(37),
                muted: false,
            },
            1,
        );
        assert_eq!(obs.volume(), Observation::Known(37));

        obs.apply(
            BusObservation::AudioStatus {
                volume: Observation::Unknown,
                muted: true,
            },
            2,
        );
        let json: serde_json::Value = serde_json::from_str(
            &serde_json::to_string(&AvState::assemble(&topology(), &obs)).unwrap(),
        )
        .unwrap();
        assert_eq!(json["volume"], serde_json::Value::Null);
        assert_ne!(json["volume"], serde_json::json!(0));
        assert_ne!(json["volume"], serde_json::json!(37));
        // The mute flag is one bit and always means something, so it stays
        // known even when the level does not.
        assert_eq!(json["muted"], serde_json::json!(true));
    }

    /// A device we do not track cannot move our fields.
    ///
    /// A shared bus may carry other playback devices; their power reports
    /// are not the television's.
    #[test]
    fn a_third_party_power_report_changes_nothing() {
        let mut obs = Observations::default();
        obs.apply(
            BusObservation::PowerStatus {
                device: AvDevice::Other,
                state: PowerState::On,
            },
            1,
        );
        let json: serde_json::Value = serde_json::from_str(
            &serde_json::to_string(&AvState::assemble(&topology(), &obs)).unwrap(),
        )
        .unwrap();
        assert_eq!(json["tvPower"], serde_json::Value::Null);
        assert_eq!(json["avrPower"], serde_json::Value::Null);
    }

    /// **The rule: the change timestamp moves on a real change and on nothing
    /// else.**
    ///
    /// A device re-announcing the same `<Active Source>` on a timer must not
    /// make "held for" reset to zero, which is the reading a consumer uses to
    /// decide whether the display has been someone else's for a while.
    ///
    /// Mutation-check (run 2026-09-14): drop the `if self.active_source == next
    /// { return; }` guard in `store_active_source` and the repeat-broadcast
    /// assertion fails.
    #[test]
    fn the_ownership_timestamp_moves_only_on_a_real_change() {
        let ours = Observation::Known(addr("2.5.0.0"));
        let mut obs = Observations::default();
        assert_eq!(obs.ownership(ours).changed_at, None);
        assert!(!obs.ownership(ours).ever_observed);

        obs.apply(BusObservation::ActiveSource(addr("1.0.0.0")), 1_000);
        assert_eq!(obs.ownership(ours).changed_at, Some(1_000));
        assert!(obs.ownership(ours).ever_observed);

        // A repeat broadcast of the SAME owner is not a transition.
        obs.apply(BusObservation::ActiveSource(addr("1.0.0.0")), 1_050);
        assert_eq!(obs.ownership(ours).changed_at, Some(1_000));
        // It IS still an observation, so the "we heard something" clock moves.
        assert_eq!(
            obs.ownership(ours).state,
            crate::ownership::Ownership::OwnedByOther
        );

        // A different device taking over is a change.
        obs.apply(BusObservation::ActiveSource(addr("2.5.0.0")), 1_060);
        assert_eq!(obs.ownership(ours).changed_at, Some(1_060));
        assert_eq!(
            obs.ownership(ours).state,
            crate::ownership::Ownership::OwnedByUs
        );
    }

    /// **The rule: a claim of OUR OWN is not evidence that this bus announces
    /// ownership.**
    ///
    /// `everObserved` exists to tell "we are listening and the bus never says
    /// anything" apart from "we are not listening". A self-claim setting it
    /// would make it always true and the distinction unaskable — v1 drew the
    /// same line in `DisplayOwner::record_local`.
    #[test]
    fn our_own_claim_does_not_count_as_bus_traffic() {
        let ours = Observation::Known(addr("2.5.0.0"));
        let mut obs = Observations::default();
        obs.record_our_claim(addr("2.5.0.0"), 500);

        let report = obs.ownership(ours);
        assert_eq!(report.state, crate::ownership::Ownership::OwnedByUs);
        assert_eq!(report.changed_at, Some(500));
        assert!(!report.ever_observed, "a self-claim is not bus traffic");
        // And it is not an observation either: nothing was heard.
        assert_eq!(obs.observed_at, None);

        obs.record_our_release(600);
        let after = obs.ownership(ours);
        assert_eq!(after.state, crate::ownership::Ownership::Unknown);
        assert_eq!(after.changed_at, Some(600));
        assert!(!after.ever_observed);
    }

    /// Once the bus has demonstrably announced ownership, that fact survives the
    /// owner going away — the flag is about the bus, not about the current
    /// value.
    #[test]
    fn ever_observed_survives_the_owner_becoming_unknown() {
        let ours = Observation::Known(addr("2.5.0.0"));
        let mut obs = Observations::default();
        obs.apply(BusObservation::ActiveSource(addr("1.0.0.0")), 10);
        obs.apply(BusObservation::InactiveSource(addr("1.0.0.0")), 20);
        let report = obs.ownership(ours);
        assert_eq!(report.state, crate::ownership::Ownership::Unknown);
        assert!(report.ever_observed);
        assert_eq!(report.changed_at, Some(20));
    }

    #[test]
    fn dropped_messages_are_counted_and_reported() {
        let mut obs = Observations::default();
        obs.apply(BusObservation::LostMessages(3), 1);
        obs.apply(BusObservation::LostMessages(4), 2);
        assert_eq!(obs.lost_messages(), 7);
        let state = AvState::assemble(&topology(), &obs);
        assert_eq!(state.lost_messages, 7);
    }

    /// The whole documented shape ships from this first step.
    #[test]
    fn the_reply_carries_every_documented_key() {
        let state = AvState::assemble(&topology(), &Observations::default());
        let json: serde_json::Value =
            serde_json::from_str(&serde_json::to_string(&state).unwrap()).unwrap();
        for field in [
            "backend",
            "device",
            "physAddr",
            "physAddrConfigured",
            "logAddrs",
            "capabilities",
            "monitorPin",
            "tvPower",
            "avrPower",
            "activeSource",
            "weAreSource",
            "displayOwnership",
            "volume",
            "muted",
            "observedAt",
            "lostMessages",
        ] {
            assert!(
                json.as_object().unwrap().contains_key(field),
                "av-state must carry {field}: {json}"
            );
        }
        assert_eq!(json["backend"], serde_json::json!("cec"));
        assert_eq!(json["physAddrConfigured"], serde_json::json!("2.5.0.0"));
    }
}
