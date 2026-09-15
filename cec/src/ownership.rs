//! Display ownership — **PURE**, and the best thing in the v1 CEC code.
//!
//! Ported from `daemon/src/display_owner.rs` and the two decision functions in
//! `daemon/src/cec.rs`. The reasoning is carried over verbatim because it is the
//! reasoning, not the code, that is worth keeping.
//!
//! # The fail-safe direction INVERTS depending on the consumer
//!
//! Read this before consuming anything here. For a transmit gate, "we don't own
//! the display" is the safe answer: it declines to touch the bus. For the
//! opposite use case — suspend this box when nobody is looking at it — treating
//! "unknown" as "not focused" would suspend a machine someone is actively
//! watching. Neither default is right for both.
//!
//! So this module reports a **tri-state** ([`Ownership`]) and only
//! [`Ownership::OwnedByOther`] means *another device positively claimed the
//! display*. [`Ownership::Unknown`] means there is no evidence either way and it
//! **must not** be rendered as "not focused", as `false`, or as healthy. Each
//! consumer picks its own safe side from the tri-state; the daemon picks none
//! for them.
//!
//! # What changed from v1, and what did not
//!
//! **Only the sensor changed.** v1 folded libcec's `command_received_callback`
//! into an `AtomicI32`; here the `linux-cec` receive loop
//! ([`crate::kernel::follower`]) folds `<Active Source>` / `<Inactive Source>`
//! into [`crate::state::Observations`]. The lock-free-atomics constraint that
//! shaped the original is gone with it — we are no longer inside a libcec
//! callback thread that may not block or re-enter libcec — so the store is a
//! plain `Mutex`.
//!
//! **The tri-state and the timestamps are kept.** `Observations` stamps the
//! moment the active source last *changed* (not every repeat broadcast, so "held
//! for" does not reset on a periodic re-announce) and records whether an
//! ownership claim has ever been received at all. That last flag is what
//! separates "we are listening and this bus never announces ownership" from "we
//! are not listening"; without it the two read identically from outside.
//!
//! # One thing genuinely differs: the address space
//!
//! v1 tracked ownership by **logical** address, because libcec's callback handed
//! it the initiator. The kernel path folds the **physical** address carried in
//! the `<Active Source>` payload, which is the address the message is actually
//! about and the one this daemon can compare against its own
//! `CEC_ADAP_G_PHYS_ADDR` read-back. The predicates are otherwise identical, and
//! `f.f.f.f` plays the role `Unregistered` played: the wire's "no address", an
//! answer rather than a device.

use serde::Serialize;

use crate::state::{Observation, PhysAddr};

/// Whether `addr` names a real port that could hold the display.
///
/// `f.f.f.f` is the CEC "invalid physical address" — an answer, not a device,
/// exactly as `Unregistered` was in v1's logical-address version. Nothing that
/// is not addressable can own a display, so both predicates below treat it as
/// nobody. Pure.
#[must_use]
pub fn is_addressable(addr: PhysAddr) -> bool {
    addr.is_valid()
}

/// Who holds the display, as a tri-state.
///
/// **Only [`Ownership::OwnedByOther`] is positive evidence that someone switched
/// away from us.** See the module docs on the inverted fail-safe.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Ownership {
    /// We are the active source.
    OwnedByUs,
    /// A different real device claimed the display.
    OwnedByOther,
    /// No claim observed, the owner went inactive, the adapter was
    /// reconfigured, or our own address is undeterminable. **Never render this
    /// as "not focused".**
    Unknown,
}

/// Pure tri-state classification, for publication.
///
/// Proof is required in BOTH directions: an unknown owner *or* an unknown
/// self-address yields [`Ownership::Unknown`] rather than a guess. There is no
/// `now` input — CEC ownership is edge-driven and does not expire, which is why
/// the published timestamp is "how long the current owner has held it", never a
/// staleness verdict.
#[must_use]
pub fn classify(owner: Observation<PhysAddr>, ours: Observation<PhysAddr>) -> Ownership {
    let (Observation::Known(owner), Observation::Known(ours)) = (owner, ours) else {
        return Ownership::Unknown;
    };
    if !is_addressable(owner) || !is_addressable(ours) {
        return Ownership::Unknown;
    }
    if owner == ours {
        Ownership::OwnedByUs
    } else {
        Ownership::OwnedByOther
    }
}

/// Standby-path decision: do we **positively** own the display?
///
/// True only when the last observed ownership claim was ours. "Never seen a
/// claim", "someone else claimed it", and "our own address is undeterminable"
/// all yield false — the standby transmit needs proof, not the absence of
/// counter-evidence. Suspending this box must not be able to power off a
/// television someone is watching on another input. Pure.
#[must_use]
pub fn owns_display(owner: Observation<PhysAddr>, ours: Observation<PhysAddr>) -> bool {
    matches!(classify(owner, ours), Ownership::OwnedByUs)
}

/// Wake-path decision: may we claim active source?
///
/// **Asymmetric with [`owns_display`] on purpose, and it is NOT
/// `classify(..) != OwnedByOther`.** Requiring `owner == ours` here would make
/// the claim a permanent no-op — if we already owned the display there would be
/// nothing to claim — so this skips only on POSITIVE PROOF that a *different
/// real device* holds the screen. That is the harm case: yanking a television
/// off someone's Apple TV mid-show.
///
/// An unobserved or unaddressable owner means nobody demonstrably owns the
/// screen, so claiming is allowed; `owner == ours` is a harmless re-assert. And
/// a real *other* owner still wins **even when our own address is unknown** —
/// which is exactly where a `classify`-derived version would go wrong, since
/// that case classifies as [`Ownership::Unknown`]. Pure.
#[must_use]
pub fn may_claim_active_source(owner: Observation<PhysAddr>, ours: Observation<PhysAddr>) -> bool {
    match owner {
        // Nothing has told us who holds it.
        Observation::Unknown => true,
        // The wire's "no address" is nobody.
        Observation::Known(owner) if !is_addressable(owner) => true,
        // A real device holds it. Only a match with our own address permits the
        // claim, and an unknown self-address is not a match.
        Observation::Known(owner) => matches!(ours, Observation::Known(u) if u == owner),
    }
}

/// Everything published about display ownership: the verdict, the two addresses
/// it was derived from, when it last changed, and whether the bus has ever said
/// anything at all.
///
/// Every one of these is "what was observed and when". The verdict is derived
/// from the other fields and is published beside them rather than instead of
/// them, so a consumer that disagrees with the classification can see why.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OwnershipReport {
    /// The tri-state. `unknown` is a first-class value.
    pub state: Ownership,
    /// The physical address that currently holds the display, or `null`.
    pub owner: Observation<PhysAddr>,
    /// Our own physical address as read back from the adapter, or `null` when
    /// `CEC_ADAP_G_PHYS_ADDR` could not be read. Published because when it is
    /// `null` the verdict can never be `owned-by-us`/`owned-by-other`, and that
    /// should be diagnosable rather than mysterious.
    pub ours: Observation<PhysAddr>,
    /// Unix ms when the owner last **changed**. `null` if it never has.
    ///
    /// **Not a staleness signal.** A claim observed six hours ago is still the
    /// current truth, so a large age means "unchanged for a long time", which is
    /// normal.
    pub changed_at: Option<u64>,
    /// Whether an `<Active Source>` / `<Inactive Source>` has ever been
    /// RECEIVED since start. Deliberately not set by our own claims: its job is
    /// to answer "does this bus actually broadcast ownership?", and a self-claim
    /// would make that always true.
    pub ever_observed: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn addr(s: &str) -> Observation<PhysAddr> {
        Observation::Known(s.parse().unwrap())
    }
    const UNSEEN: Observation<PhysAddr> = Observation::Unknown;
    /// The wire's "no physical address". Reachable from the real receive path: a
    /// device on a shared bus can broadcast `<Active Source>` carrying `0xFFFF`,
    /// and `kernel::follower` folds the payload verbatim rather than dropping
    /// it — publishing what was heard is the whole rule of this crate.
    fn invalid() -> Observation<PhysAddr> {
        Observation::Known(PhysAddr::INVALID)
    }

    #[test]
    fn is_addressable_rejects_the_wires_no_address() {
        // Table-driven so the arguments are not compile-time constants (a bare
        // `assert!(is_addressable(..))` on a literal trips clippy's
        // assertions_on_constants).
        let cases = [
            ("0.0.0.0", true),
            ("2.5.0.0", true),
            ("1.0.0.0", true),
            ("f.f.f.e", true),
            ("f.f.f.f", false),
        ];
        for (s, want) in cases {
            let a: PhysAddr = s.parse().unwrap();
            assert_eq!(is_addressable(a), want, "is_addressable({s})");
        }
    }

    /// **The rule: `owns_display` requires positive proof.**
    ///
    /// Mutation-check (run 2026-09-14): invert the predicate — return
    /// `!matches!(classify(..), Ownership::OwnedByUs)`, or relax it to
    /// `classify(..) != OwnedByOther` — and this fails, along with the standby
    /// gate tests in `action` and `ipc`.
    #[test]
    fn owns_display_requires_positive_proof() {
        // (observed owner, our address, do we own the display?)
        let cases = [
            // The only true case: the last observed claim was ours.
            (addr("2.5.0.0"), addr("2.5.0.0"), true),
            (addr("0.0.0.0"), addr("0.0.0.0"), true),
            // Someone else claimed it — the Apple TV case.
            (addr("1.0.0.0"), addr("2.5.0.0"), false),
            (addr("0.0.0.0"), addr("2.5.0.0"), false),
            // NEVER SEEN a claim — a daemon started mid-session. Must skip.
            (UNSEEN, addr("2.5.0.0"), false),
            // `<Inactive Source>` / the wire's no-address: nobody owns it.
            (invalid(), addr("2.5.0.0"), false),
            // Undeterminable on both sides is never "we own it".
            (UNSEEN, UNSEEN, false),
            (invalid(), invalid(), false),
            // A real owner we cannot match ourselves against is not proof.
            (addr("2.5.0.0"), UNSEEN, false),
            (addr("2.5.0.0"), invalid(), false),
        ];
        for (owner, ours, want) in cases {
            assert_eq!(
                owns_display(owner, ours),
                want,
                "owns_display({owner:?}, {ours:?})"
            );
        }
    }

    /// **The rule: the wake claim yields only to a KNOWN OTHER owner.**
    ///
    /// Mutation-check (run 2026-09-14): make it symmetric with `owns_display`
    /// (`owns_display(owner, ours)`) and the "never seen a claim" rows fail —
    /// the claim would become a permanent no-op. Derive it from `classify`
    /// instead (`classify(..) != OwnedByOther`) and the last row fails: a real
    /// other owner must still win when our own address is unknown.
    #[test]
    fn may_claim_active_source_yields_only_to_a_known_other_owner() {
        // (observed owner, our address, may claim?)
        let cases = [
            // A different REAL device holds the screen — the one and only reason
            // to skip the claim.
            (addr("1.0.0.0"), addr("2.5.0.0"), false),
            (addr("0.0.0.0"), addr("2.5.0.0"), false),
            // We already hold it: re-asserting is harmless.
            (addr("2.5.0.0"), addr("2.5.0.0"), true),
            // Never seen a claim -> nobody demonstrably owns the screen.
            (UNSEEN, addr("2.5.0.0"), true),
            (invalid(), addr("2.5.0.0"), true),
            (UNSEEN, UNSEEN, true),
            // THE ASYMMETRY THAT `classify` CANNOT EXPRESS: a real other owner
            // wins even when our own address is unknown. `classify` calls that
            // `Unknown`, which would permit the claim.
            (addr("1.0.0.0"), UNSEEN, false),
            (addr("1.0.0.0"), invalid(), false),
        ];
        for (owner, ours, want) in cases {
            assert_eq!(
                may_claim_active_source(owner, ours),
                want,
                "may_claim_active_source({owner:?}, {ours:?})"
            );
        }
    }

    /// The whole point of two predicates: on a bus where no claim has been
    /// observed, standby must refuse to transmit while the wake claim proceeds.
    #[test]
    fn the_two_gates_are_asymmetric_when_nothing_has_been_observed() {
        for (owner, ours) in [(UNSEEN, addr("2.5.0.0")), (invalid(), addr("1.0.0.0"))] {
            assert!(
                !owns_display(owner, ours),
                "standby must refuse for owner={owner:?}"
            );
            assert!(
                may_claim_active_source(owner, ours),
                "the wake claim must proceed for owner={owner:?}"
            );
        }
    }

    #[test]
    fn classify_requires_proof_in_both_directions() {
        let cases = [
            (addr("2.5.0.0"), addr("2.5.0.0"), Ownership::OwnedByUs),
            (addr("1.0.0.0"), addr("2.5.0.0"), Ownership::OwnedByOther),
            (addr("0.0.0.0"), addr("2.5.0.0"), Ownership::OwnedByOther),
            // Never observed / went inactive / adapter reconfigured.
            (UNSEEN, addr("2.5.0.0"), Ownership::Unknown),
            (invalid(), addr("2.5.0.0"), Ownership::Unknown),
            // We do not know our own address, so we cannot tell "us" from
            // "other".
            (addr("2.5.0.0"), UNSEEN, Ownership::Unknown),
            (addr("2.5.0.0"), invalid(), Ownership::Unknown),
            (UNSEEN, UNSEEN, Ownership::Unknown),
        ];
        for (owner, ours, want) in cases {
            assert_eq!(classify(owner, ours), want, "classify({owner:?}, {ours:?})");
        }
    }

    /// The tri-state reaches the wire as three distinct words, and `unknown` is
    /// one of them — never `false`, never an omitted field.
    #[test]
    fn the_tristate_serialises_as_three_distinct_words() {
        for (value, want) in [
            (Ownership::OwnedByUs, r#""owned-by-us""#),
            (Ownership::OwnedByOther, r#""owned-by-other""#),
            (Ownership::Unknown, r#""unknown""#),
        ] {
            assert_eq!(serde_json::to_string(&value).unwrap(), want);
        }
    }

    #[test]
    fn an_unobserved_report_serialises_with_nulls_and_the_unknown_word() {
        let report = OwnershipReport {
            state: Ownership::Unknown,
            owner: UNSEEN,
            ours: UNSEEN,
            changed_at: None,
            ever_observed: false,
        };
        let json: serde_json::Value =
            serde_json::from_str(&serde_json::to_string(&report).unwrap()).unwrap();
        assert_eq!(json["state"], serde_json::json!("unknown"));
        for field in ["owner", "ours", "changedAt"] {
            assert_eq!(json[field], serde_json::Value::Null, "{field}");
            assert_ne!(json[field], serde_json::json!(false), "{field}");
        }
        assert_eq!(json["everObserved"], serde_json::json!(false));
    }
}
