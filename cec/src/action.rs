//! Power and input switching — **the decision half, and it is pure**.
//!
//! An [`Action`] plus the two observed addresses becomes either a [`Plan`] (the
//! exact list of messages to put on the bus, in order) or a [`Refusal`] (zero
//! messages, and why). No I/O, no device, no `linux-cec` type: the whole
//! decision is covered by CI on a runner with no adapter, which is the `core/`
//! discipline this crate follows everywhere.
//!
//! [`crate::kernel::ops`] is the other half — the pure translation of each
//! [`CecTx`] into a `linux-cec` `Message`, plus the transmitter that puts it on
//! the wire.
//!
//! # The three rules this module exists to enforce
//!
//! 1. **A broadcast `<Standby>` is unrepresentable, not merely avoided.**
//!    [`CecTx::Standby`] takes a [`StandbyTarget`], which has two variants and
//!    no broadcast. CEC's broadcast standby (`0x0F`) powers off *every* device
//!    on the bus, and the living-room bus carries an Apple TV and a PS5 as well
//!    as the television and the AVR.
//! 2. **A refusal transmits nothing at all.** Every gate is evaluated before any
//!    message is built, so [`Refusal`] means exactly zero bus traffic. That is
//!    what makes "standby refused" a state a caller can trust, and it is why the
//!    reply for it is distinguishable from `ok` (see [`crate::protocol`]).
//! 3. **Ordering is load-bearing.** V2_DESIGN §8 records that a receiver ignores
//!    CEC from a non-selected input, so an action only lands while we are the
//!    selected input: claim the display first, then act. `wake` therefore
//!    transmits `<Image View On>` and then the `<Active Source>` claim, and
//!    `standby` refuses outright unless we already positively hold the display.
//!
//! # Why `wake` is gated as a whole rather than half-gated
//!
//! v1 gated only the `<Active Source>` half of its wake sequence, leaving the
//! power-on to go out regardless. Here the gate is checked first and refuses the
//! whole sequence, for two reasons. Positive proof that a *different real
//! device* holds the display is itself proof the chain is not cold — something
//! is on and someone is watching it — so the power-on half has nothing to do.
//! And a partial success would have to be reported either as `ok` (which claims
//! we became the active source) or as an error (which claims a fault); neither
//! is true, and rule 2 above is worth more than the half-action.

use crate::ownership::{may_claim_active_source, owns_display};
use crate::state::{Observation, PhysAddr};

/// A device that may be told to enter standby.
///
/// **Two variants and no broadcast, deliberately.** See rule 1 in the module
/// docs: `LogicalAddress::Broadcast` is not spellable here, so no future edit
/// can reintroduce a bus-wide power-off by passing the wrong constant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StandbyTarget {
    /// The television.
    Tv,
    /// The audio system — the AVR.
    AudioSystem,
}

impl StandbyTarget {
    /// The two targets a standby is addressed to, in order.
    ///
    /// The television first: it is the device whose state the user actually
    /// sees, so if only one message lands it should be that one. A site with no
    /// AVR simply NAKs the second, which [`crate::kernel::ops`] treats as a
    /// per-target failure rather than a failed action.
    pub const ALL: [StandbyTarget; 2] = [StandbyTarget::Tv, StandbyTarget::AudioSystem];
}

/// One message this daemon intends to put on the bus.
///
/// This crate's own vocabulary, not `linux-cec`'s, for the same reason
/// [`crate::state::BusObservation`] is: it keeps the decision pure and keeps the
/// backend seam real. [`crate::kernel::ops::message_for`] is the only
/// translation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CecTx {
    /// `<Image View On>` to the TV — One Touch Play's power-on.
    ///
    /// **`<Image View On>`, not `<Text View On>`.** The latter also asks the TV
    /// to clear any menu and show text, which is a visible side effect nobody
    /// asked for.
    ImageViewOn,
    /// `<Active Source>` broadcast — we are driving the display.
    ActiveSource(PhysAddr),
    /// `<Inactive Source>` to the TV — we are giving the display up.
    ///
    /// Addressed to the TV, not broadcast: the CEC specification directs this
    /// one, and it carries our own address as the source being released.
    InactiveSource(PhysAddr),
    /// `<Standby>` addressed to one device. Never broadcast — see rule 1.
    Standby(StandbyTarget),
    /// `<Set Stream Path>` broadcast — make the named address the active
    /// source.
    ///
    /// **A capability the libcec path did not have.** Under `cec-rs` 12.0.1 the
    /// only "make X the active source" primitive was `send_power_on_devices`
    /// (`daemon/src/cec.rs:37-43`), which conflates powering a device on with
    /// handing it the display. The kernel API expresses the routing directive
    /// directly.
    SetStreamPath(PhysAddr),
}

/// A power status to read back after the transmits land.
///
/// Request/reply with a bounded timeout, never fire-and-forget plus a guess:
/// `<Give Device Power Status>` has a defined reply, so the daemon asks for it
/// rather than assuming the action worked. A missing reply leaves the published
/// power `unknown`; it does not turn into a verdict.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PowerRead {
    pub target: StandbyTarget,
}

/// What to record locally once the plan's transmits have landed.
///
/// Our own `<Active Source>` never comes back through the receive loop — the
/// kernel does not loop our own transmits back to us — so without this the
/// daemon would claim the display and then still report `activeSource: null`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocalRecord {
    /// We now hold the display.
    Claimed(PhysAddr),
    /// We gave it up. Recorded as `unknown`, never as "nobody": the message says
    /// who released it, never who has it now.
    Released,
}

/// One request from a client.
///
/// A closed set with an exhaustive match in [`plan`], so a verb added here
/// without a decision is a compile error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// Power the chain on and become the active source.
    Wake,
    /// Put the television and the AVR into standby. Gated on positive proof of
    /// ownership.
    Standby,
    /// Become the active source without touching power.
    InputClaim,
    /// Give the display up.
    InputRelease,
    /// Hand the display to the named physical address.
    InputSelect(PhysAddr),
}

/// The messages an action becomes, in transmit order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    /// Transmitted in order. Empty is not a valid plan — an action with nothing
    /// to send is a [`Refusal`], so that "we did nothing" always carries a
    /// reason.
    pub transmits: Vec<CecTx>,
    /// A status to read back afterwards, if this action has one.
    pub then_read: Option<PowerRead>,
    /// What the local observation store should record once the transmits land.
    pub record: Option<LocalRecord>,
}

/// A deliberate decision NOT to act, and why.
///
/// **Zero transmits, always.** A `Refusal` is not an error: nothing is broken
/// and nothing was attempted. The reply grammar keeps the two apart for exactly
/// that reason.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refusal {
    pub reason: String,
}

impl Refusal {
    fn new(reason: impl Into<String>) -> Refusal {
        Refusal {
            reason: reason.into(),
        }
    }
}

/// Decide what an action becomes, given what has been observed.
///
/// `ours` is our own physical address as read back from the adapter, and `owner`
/// is the last `<Active Source>` heard on the bus. Both are
/// [`Observation`]s — "nothing has told us" is an input here, not an absence.
pub fn plan(
    action: Action,
    ours: Observation<PhysAddr>,
    owner: Observation<PhysAddr>,
) -> Result<Plan, Refusal> {
    match action {
        Action::Wake => {
            let ours = claimable_address(ours)?;
            check_may_claim(owner, Observation::Known(ours))?;
            Ok(Plan {
                // Power first, then the claim: One Touch Play's own order, and
                // a claim to a television that is still off is heard by nobody.
                transmits: vec![CecTx::ImageViewOn, CecTx::ActiveSource(ours)],
                // Read the TV's power back rather than reporting "woken"
                // because two messages were ACKed.
                then_read: Some(PowerRead {
                    target: StandbyTarget::Tv,
                }),
                record: Some(LocalRecord::Claimed(ours)),
            })
        }
        Action::InputClaim => {
            let ours = claimable_address(ours)?;
            check_may_claim(owner, Observation::Known(ours))?;
            Ok(Plan {
                transmits: vec![CecTx::ActiveSource(ours)],
                then_read: None,
                record: Some(LocalRecord::Claimed(ours)),
            })
        }
        Action::Standby => {
            // POSITIVE PROOF ONLY. "Nothing has told us" is not permission: a
            // standby sent from a box that is not the selected input either
            // does nothing or powers off a television someone else is watching.
            if !owns_display(owner, ours) {
                return Err(Refusal::new(format!(
                    "standby needs positive proof that we hold the display; \
                     activeSource={} ours={}. Nothing was transmitted",
                    describe(owner),
                    describe(ours)
                )));
            }
            Ok(Plan {
                transmits: StandbyTarget::ALL
                    .iter()
                    .copied()
                    .map(CecTx::Standby)
                    .collect(),
                then_read: Some(PowerRead {
                    target: StandbyTarget::Tv,
                }),
                // Standby is not a release: we do not send `<Inactive Source>`,
                // and a powered-off bus tells us nothing about who holds the
                // display next. Leaving the record alone is the honest answer.
                record: None,
            })
        }
        Action::InputRelease => {
            let ours = claimable_address(ours)?;
            // UNGATED on purpose. Releasing a display we do not hold is a no-op
            // at every receiver — `<Inactive Source>` naming an address that is
            // not the active source changes nothing — so there is no harm case
            // for the gate to prevent, and gating it would make "give the
            // display up" fail exactly when the daemon's own record is stale.
            Ok(Plan {
                transmits: vec![CecTx::InactiveSource(ours)],
                then_read: None,
                record: Some(LocalRecord::Released),
            })
        }
        Action::InputSelect(target) => {
            if !crate::ownership::is_addressable(target) {
                return Err(Refusal::new(format!(
                    "{target} is the CEC invalid physical address, which names no port"
                )));
            }
            // UNGATED on purpose, and this is the one place that deserves
            // saying out loud. The ownership gate exists to stop the daemon's
            // OWN lifecycle actions from yanking a display somebody is using.
            // `input-select` is not a lifecycle action: an operator naming a
            // physical address has stated exactly which device should get the
            // screen, and refusing that because a third device currently holds
            // it would make the verb useless in the only situation it is for.
            Ok(Plan {
                transmits: vec![CecTx::SetStreamPath(target)],
                then_read: None,
                // The selected device confirms with its own `<Active Source>`,
                // which the receive loop folds. Recording a guess here would
                // publish an outcome we have not observed.
                record: None,
            })
        }
    }
}

/// Our own address, or a refusal naming why we cannot claim without it.
fn claimable_address(ours: Observation<PhysAddr>) -> Result<PhysAddr, Refusal> {
    match ours {
        Observation::Known(a) if crate::ownership::is_addressable(a) => Ok(a),
        Observation::Known(a) => Err(Refusal::new(format!(
            "the adapter reports its physical address as {a}, the CEC invalid address, so \
             there is no port to name as the active source"
        ))),
        Observation::Unknown => Err(Refusal::new(
            "our own physical address could not be read back from the adapter \
             (CEC_ADAP_G_PHYS_ADDR), so a claim would name no port",
        )),
    }
}

/// The wake/claim gate, as a refusal.
fn check_may_claim(
    owner: Observation<PhysAddr>,
    ours: Observation<PhysAddr>,
) -> Result<(), Refusal> {
    if may_claim_active_source(owner, ours) {
        return Ok(());
    }
    Err(Refusal::new(format!(
        "{} positively holds the display, so claiming it would take the screen from \
         whoever is watching. Nothing was transmitted",
        describe(owner)
    )))
}

/// An observed address for an operator-facing message.
fn describe(value: Observation<PhysAddr>) -> String {
    match value {
        Observation::Known(a) => a.to_string(),
        Observation::Unknown => "unknown".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn a(s: &str) -> PhysAddr {
        s.parse().unwrap()
    }
    fn known(s: &str) -> Observation<PhysAddr> {
        Observation::Known(a(s))
    }
    const UNSEEN: Observation<PhysAddr> = Observation::Unknown;

    const OURS: &str = "2.5.0.0";
    const SOMEONE_ELSE: &str = "1.0.0.0";

    #[test]
    fn wake_is_image_view_on_then_the_claim_in_that_order() {
        let plan = plan(Action::Wake, known(OURS), UNSEEN).unwrap();
        assert_eq!(
            plan.transmits,
            vec![CecTx::ImageViewOn, CecTx::ActiveSource(a(OURS))]
        );
        assert_eq!(
            plan.then_read,
            Some(PowerRead {
                target: StandbyTarget::Tv
            })
        );
        assert_eq!(plan.record, Some(LocalRecord::Claimed(a(OURS))));
    }

    /// **THE LOAD-BEARING NEGATIVE TEST: standby while another device holds the
    /// display refuses and transmits ZERO messages.**
    ///
    /// Asserted on the transmits the decision produces, not on which readers
    /// were consulted — v1's tests drew that line and this mirrors it. A
    /// standby that reached the bus here would power off a television somebody
    /// is watching on another input.
    ///
    /// Mutation-check (run 2026-09-14): delete the `owns_display` gate from the
    /// `Action::Standby` arm and this fails on the first row, together with
    /// `ipc::standby_refuses_and_transmits_nothing_when_someone_else_holds_the_display`.
    #[test]
    fn standby_refuses_and_transmits_nothing_without_positive_proof() {
        // (owner, ours) — every case that is not positive proof.
        let cases = [
            // Someone else positively holds it: the Apple TV case.
            (known(SOMEONE_ELSE), known(OURS)),
            (known("0.0.0.0"), known(OURS)),
            // Nothing has told us: a daemon started mid-session.
            (UNSEEN, known(OURS)),
            // The wire's no-address is nobody, not us.
            (Observation::Known(PhysAddr::INVALID), known(OURS)),
            // Our own address is undeterminable, so "is the owner us?" has no
            // answer.
            (known(OURS), UNSEEN),
            (known(OURS), Observation::Known(PhysAddr::INVALID)),
            (UNSEEN, UNSEEN),
        ];
        for (owner, ours) in cases {
            let refusal = plan(Action::Standby, ours, owner).expect_err(&format!(
                "standby must refuse for owner={owner:?} ours={ours:?}"
            ));
            assert!(
                refusal.reason.contains("Nothing was transmitted"),
                "the refusal must say so: {}",
                refusal.reason
            );
        }
    }

    /// And the one case that does proceed, so the gate is not vacuously passing.
    #[test]
    fn standby_proceeds_on_positive_proof_and_never_broadcasts() {
        let plan = plan(Action::Standby, known(OURS), known(OURS)).unwrap();
        assert_eq!(
            plan.transmits,
            vec![
                CecTx::Standby(StandbyTarget::Tv),
                CecTx::Standby(StandbyTarget::AudioSystem),
            ]
        );
        // The television first — the device whose state the user can see.
        assert_eq!(plan.transmits[0], CecTx::Standby(StandbyTarget::Tv));
        // A broadcast standby is not spellable, so this asserts the shape the
        // type already guarantees: every standby names a device.
        for tx in &plan.transmits {
            assert!(
                matches!(
                    tx,
                    CecTx::Standby(StandbyTarget::Tv | StandbyTarget::AudioSystem)
                ),
                "{tx:?}"
            );
        }
    }

    /// **The rule: the wake claim yields only to a KNOWN OTHER owner**, and a
    /// refusal is zero transmits.
    ///
    /// Mutation-check (run 2026-09-14): make `check_may_claim` call
    /// `owns_display` instead and the two "nothing observed" rows fail — the
    /// claim becomes a permanent no-op, which is exactly the asymmetry v1
    /// documented.
    #[test]
    fn the_claim_gate_refuses_only_a_known_other_owner() {
        for action in [Action::Wake, Action::InputClaim] {
            // Refused: a different real device holds the screen.
            for owner in [known(SOMEONE_ELSE), known("0.0.0.0")] {
                let refusal = plan(action, known(OURS), owner).expect_err("must refuse");
                assert!(
                    refusal.reason.contains("Nothing was transmitted"),
                    "{}",
                    refusal.reason
                );
            }
            // Permitted: nothing observed, the wire's no-address, or us.
            for owner in [UNSEEN, Observation::Known(PhysAddr::INVALID), known(OURS)] {
                let plan = plan(action, known(OURS), owner).unwrap_or_else(|e| {
                    panic!("{action:?} must proceed for {owner:?}: {}", e.reason)
                });
                assert!(plan.transmits.contains(&CecTx::ActiveSource(a(OURS))));
            }
        }
    }

    /// A real other owner wins even when our own address is unknown — but the
    /// refusal then names the missing self-address, because that is the fault an
    /// operator can act on.
    #[test]
    fn a_claim_without_our_own_address_refuses_and_says_why() {
        let refusal = plan(Action::Wake, UNSEEN, UNSEEN).expect_err("must refuse");
        assert!(
            refusal.reason.contains("CEC_ADAP_G_PHYS_ADDR"),
            "{}",
            refusal.reason
        );
        let refusal = plan(
            Action::InputClaim,
            Observation::Known(PhysAddr::INVALID),
            UNSEEN,
        )
        .expect_err("must refuse");
        assert!(
            refusal.reason.contains("invalid address"),
            "{}",
            refusal.reason
        );
    }

    /// Release is ungated: it must work when our own record of ownership is
    /// stale, which is the situation it exists for.
    #[test]
    fn release_is_ungated_and_names_our_own_address() {
        for owner in [UNSEEN, known(SOMEONE_ELSE), known(OURS)] {
            let plan = plan(Action::InputRelease, known(OURS), owner).unwrap();
            assert_eq!(plan.transmits, vec![CecTx::InactiveSource(a(OURS))]);
            assert_eq!(plan.record, Some(LocalRecord::Released));
        }
    }

    /// `input-select` hands the display to a named device, and is deliberately
    /// not subject to the lifecycle gate.
    #[test]
    fn select_is_a_routing_directive_and_records_no_guess() {
        let plan = plan(
            Action::InputSelect(a(SOMEONE_ELSE)),
            known(OURS),
            known("3.0.0.0"),
        )
        .unwrap();
        assert_eq!(plan.transmits, vec![CecTx::SetStreamPath(a(SOMEONE_ELSE))]);
        // The selected device confirms with its own <Active Source>; recording
        // an outcome here would publish something unobserved.
        assert_eq!(plan.record, None);
    }

    /// A physical address that names no port is refused rather than broadcast.
    #[test]
    fn select_refuses_the_invalid_address() {
        let refusal =
            plan(Action::InputSelect(PhysAddr::INVALID), known(OURS), UNSEEN).expect_err("refuse");
        assert!(
            refusal.reason.contains("invalid physical address"),
            "{}",
            refusal.reason
        );
    }

    /// **The invariant that makes a refusal trustworthy: a `Refusal` is never
    /// accompanied by a plan, so it can never have transmitted anything.**
    ///
    /// Stated as a test over the whole action set rather than left implicit,
    /// because the reply grammar depends on it: `refused:` tells a caller
    /// nothing reached the bus.
    #[test]
    fn every_plan_transmits_something_and_every_refusal_transmits_nothing() {
        let actions = [
            Action::Wake,
            Action::Standby,
            Action::InputClaim,
            Action::InputRelease,
            Action::InputSelect(a(SOMEONE_ELSE)),
        ];
        let addresses = [UNSEEN, known(OURS), Observation::Known(PhysAddr::INVALID)];
        for action in actions {
            for ours in addresses {
                for owner in addresses {
                    match plan(action, ours, owner) {
                        // A plan with no transmits would be a silent no-op
                        // reported as success.
                        Ok(p) => assert!(!p.transmits.is_empty(), "{action:?} {ours:?} {owner:?}"),
                        Err(r) => assert!(!r.reason.is_empty(), "{action:?} needs a reason"),
                    }
                }
            }
        }
    }
}
