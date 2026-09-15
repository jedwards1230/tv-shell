//! Power and input switching — **the wire half**.
//!
//! Two things live here, and the split is deliberate:
//!
//! 1. [`message_for`] — the **pure** translation of one [`CecTx`] into a
//!    `linux-cec` `Message` plus the `LogicalAddress` it is addressed to. It
//!    needs no device, so CI covers the whole message table on a runner with no
//!    adapter. That is the `core/` discipline, and it is what lets CI assert
//!    anything at all about a subsystem whose hardware it does not have.
//! 2. [`Transmitter`] — the seam over the actual ioctl, plus [`execute`], which
//!    walks a [`Plan`] and reports honestly on what the bus did with it.
//!
//! # `ok` means the bus accepted it, not that it left the adapter
//!
//! `linux-cec`'s `tx_message` is a thin wrapper over `CEC_TRANSMIT` that
//! **errors unless the kernel's `tx_status` carries `OK`** — i.e. the frame was
//! acknowledged by the destination, or broadcast without a NAK. So a `Done`
//! outcome from here is a bus-level acknowledgement rather than a queue receipt.
//! Where a reply exists, [`execute`] additionally reads it back
//! (`<Give Device Power Status>` → `<Report Power Status>`, request/reply with a
//! bounded timeout); where none does, it reports the transmit status as it
//! found it.
//!
//! That distinction is the same failure shape as v1's `cec-health` reporting a
//! health it could not know, one layer down: a daemon that answers "woken"
//! because two frames were queued has told the caller something it did not
//! observe.
//!
//! # Ordering is load-bearing
//!
//! V2_DESIGN §8 records that a receiver ignores CEC from a non-selected input.
//! An action therefore only lands while we are the selected input, which is why
//! [`crate::action::plan`] puts the claim first and why `standby` refuses unless
//! we already positively hold the display. Nothing here reorders a plan: it
//! transmits in the order given, and stops at the first hard failure.

use std::sync::{Arc, Mutex};

use linux_cec::device::AsyncDevice;
use linux_cec::message::{Message, Opcode};
use linux_cec::{LogicalAddress, PhysicalAddress, Timeout};

use crate::action::{CecTx, LocalRecord, Plan, PowerRead, StandbyTarget};
use crate::backend::ActionOutcome;
use crate::state::{now_ms, AvDevice, BusObservation, Observations, PowerState};

/// How long a request/reply read-back waits.
///
/// One second is `Timeout::MAX`: `CEC_TRANSMIT` coerces anything larger (and
/// anything zero) to one second, so naming it is stating what the kernel will
/// do rather than pretending to a choice.
const REPLY_TIMEOUT: Timeout = Timeout::MAX;

/// The logical address a standby target is addressed to.
///
/// **This function is the only place a `<Standby>` gets a destination, and
/// `LogicalAddress::Broadcast` is not among its results.** A broadcast standby
/// (`0x0F`) powers off every device on the bus; the living-room bus carries an
/// Apple TV and a PS5 as well as the television and the AVR.
#[must_use]
pub fn standby_destination(target: StandbyTarget) -> LogicalAddress {
    match target {
        StandbyTarget::Tv => LogicalAddress::Tv,
        StandbyTarget::AudioSystem => LogicalAddress::AudioSystem,
    }
}

/// Translate one intended transmit into the message and destination it becomes.
///
/// **Pure**, and on the no-device side of the seam on purpose: it is the whole
/// message table, and CI has no adapter.
#[must_use]
pub fn message_for(tx: CecTx) -> (Message, LogicalAddress) {
    match tx {
        // `<Image View On>`, not `<Text View On>`: the latter also asks the TV
        // to dismiss menus and show text, a visible side effect nobody asked
        // for. `Device::wake(set_active, text_view)` would send both halves,
        // but going through the plan keeps the order and the gate in one
        // testable place.
        CecTx::ImageViewOn => (Message::ImageViewOn, LogicalAddress::Tv),
        // `<Active Source>` is a broadcast by specification.
        CecTx::ActiveSource(addr) => (
            Message::ActiveSource {
                address: phys(addr),
            },
            LogicalAddress::Broadcast,
        ),
        // `<Inactive Source>` is DIRECTED to the TV, not broadcast.
        //
        // NOTE, against the plan for jedwards1230/tv-shell#504, which says
        // `set_active_source(None)` sends this: it does not. Read
        // `linux-cec` 0.2.1 `device.rs:855` — `set_active_source(None)` falls
        // back to the device's OWN physical address and sends `<Active
        // Source>`, i.e. it CLAIMS the display rather than releasing it. Using
        // it for `input-release` would have done the exact opposite of the
        // verb's name. The message is constructed explicitly instead.
        CecTx::InactiveSource(addr) => (
            Message::InactiveSource {
                address: phys(addr),
            },
            LogicalAddress::Tv,
        ),
        CecTx::Standby(target) => (Message::Standby, standby_destination(target)),
        // `<Set Stream Path>` is a broadcast routing directive: "whoever is at
        // this physical address, take the display".
        CecTx::SetStreamPath(addr) => (
            Message::SetStreamPath {
                address: phys(addr),
            },
            LogicalAddress::Broadcast,
        ),
    }
}

/// The logical address a power read-back is addressed to.
#[must_use]
pub fn power_query_for(read: PowerRead) -> (Message, LogicalAddress, Opcode) {
    (
        Message::GiveDevicePowerStatus,
        standby_destination(read.target),
        Opcode::ReportPowerStatus,
    )
}

/// Which published field a power read-back belongs to.
#[must_use]
pub fn power_read_device(read: PowerRead) -> AvDevice {
    match read.target {
        StandbyTarget::Tv => AvDevice::Tv,
        StandbyTarget::AudioSystem => AvDevice::AudioSystem,
    }
}

/// This crate's physical address as `linux-cec` carries it.
///
/// `From<u16>`, not `TryFrom`: the 16-bit encoding *is* a physical address,
/// including `f.f.f.f`. Whether the value names a real port is a decision, and
/// it was already taken in [`crate::action::plan`].
fn phys(addr: crate::state::PhysAddr) -> PhysicalAddress {
    PhysicalAddress::from(addr.raw())
}

/// The seam over the transmit ioctls.
///
/// Small on purpose: two methods, both taking values this crate already owns.
/// Keeping it here rather than on [`crate::backend::AvBackend`] means the
/// decision layer never sees a device, and the device layer never sees a verb.
#[async_trait::async_trait]
pub trait Transmitter: Send + Sync {
    /// Put one message on the bus, returning an error unless the bus accepted
    /// it.
    async fn send(&self, message: &Message, destination: LogicalAddress) -> Result<(), String>;

    /// Ask a device for its power state and wait, bounded, for the reply.
    async fn query_power(&self, read: PowerRead) -> Result<PowerState, String>;
}

/// A real `/dev/cecN`.
pub struct DeviceTransmitter {
    device: Arc<AsyncDevice>,
}

impl DeviceTransmitter {
    #[must_use]
    pub fn new(device: Arc<AsyncDevice>) -> DeviceTransmitter {
        DeviceTransmitter { device }
    }
}

#[async_trait::async_trait]
impl Transmitter for DeviceTransmitter {
    async fn send(&self, message: &Message, destination: LogicalAddress) -> Result<(), String> {
        self.device
            .tx_message(message, destination)
            .await
            .map(|_sequence| ())
            // The sequence number is of no use to a caller and the error is:
            // `tx_message` fails unless `tx_status` carries OK, so this really
            // is "the bus did not accept it".
            .map_err(|e| format!("{e}"))
    }

    async fn query_power(&self, read: PowerRead) -> Result<PowerState, String> {
        let (message, destination, reply) = power_query_for(read);
        let envelope = self
            .device
            .tx_rx_message(&message, destination, reply, REPLY_TIMEOUT)
            .await
            .map_err(|e| format!("{e}"))?;
        match envelope.message {
            linux_cec::device::MessageData::Valid(Message::ReportPowerStatus { status }) => {
                power_state(status).ok_or_else(|| {
                    format!("{destination} reported a power status this daemon does not recognise")
                })
            }
            other => Err(format!(
                "{destination} answered <Give Device Power Status> with {other:?}"
            )),
        }
    }
}

/// The four power states CEC defines.
///
/// `PowerStatus` is `#[non_exhaustive]`, and a status this daemon does not
/// recognise yields `None` rather than being folded into the nearest of the
/// four — reporting an unrecognised value as `on` or `standby` would be exactly
/// the confident-and-wrong answer this crate publishes `unknown` to avoid.
#[must_use]
pub fn power_state(status: linux_cec::operand::PowerStatus) -> Option<PowerState> {
    use linux_cec::operand::PowerStatus;
    Some(match status {
        PowerStatus::On => PowerState::On,
        PowerStatus::Standby => PowerState::Standby,
        PowerStatus::ToOn => PowerState::ToOn,
        PowerStatus::ToStandby => PowerState::ToStandby,
        _ => return None,
    })
}

/// Transmit a plan and report what the bus did with it.
///
/// Stops at the first transmit the bus does not accept and reports it, naming
/// which message failed and how many had already landed — a half-performed
/// sequence reported as a bare failure tells an operator nothing about the state
/// the rack was left in.
///
/// **One deliberate exception**: the standby sequence addresses two devices, and
/// a site with no AVR NAKs the second. That is not a failed standby — the
/// television, the device whose state the user can see, went off. So a failure
/// on a *later* standby message is logged and the action still reports `Done`,
/// while a failure on the first is a failure.
pub async fn execute(
    transmitter: &dyn Transmitter,
    observations: &Mutex<Observations>,
    plan: Plan,
) -> ActionOutcome {
    let mut landed = 0usize;
    for tx in &plan.transmits {
        let (message, destination) = message_for(*tx);
        match transmitter.send(&message, destination).await {
            Ok(()) => landed += 1,
            Err(e) if tolerable(&plan, *tx, landed) => {
                tracing::warn!("{tx:?} to {destination} was not accepted ({e}); continuing");
            }
            Err(e) => {
                return ActionOutcome::Failed(format!(
                    "{tx:?} to {destination} was not accepted by the bus ({e}); {landed} of {} \
                     messages had already landed",
                    plan.transmits.len()
                ));
            }
        }
    }

    // Record what we now know locally. Our own `<Active Source>` is never looped
    // back to us by the kernel, so without this the daemon would claim the
    // display and still publish `activeSource: null`.
    if let Some(record) = plan.record {
        let now = now_ms();
        let mut guard = lock(observations);
        match record {
            LocalRecord::Claimed(addr) => guard.record_our_claim(addr, now),
            LocalRecord::Released => guard.record_our_release(now),
        }
    }

    // Read back where a reply exists. A device that does not answer leaves the
    // published power `unknown`; it never becomes a guess, and it never turns a
    // landed action into a failure — a television mid-transition legitimately
    // does not reply.
    if let Some(read) = plan.then_read {
        match transmitter.query_power(read).await {
            Ok(state) => {
                let device = power_read_device(read);
                tracing::info!("read back after the action: {device:?} reports {state:?}");
                lock(observations).apply(BusObservation::PowerStatus { device, state }, now_ms());
            }
            Err(e) => tracing::warn!(
                "the power read-back after the action got no usable reply ({e}); the published \
                 power stays unknown rather than being guessed"
            ),
        }
    }

    ActionOutcome::Done
}

/// Whether a failed transmit may be survived rather than failing the action.
///
/// Only the standby sequence's *later* targets: see [`execute`]'s docs.
fn tolerable(plan: &Plan, tx: CecTx, already_landed: usize) -> bool {
    matches!(tx, CecTx::Standby(_)) && already_landed > 0 && plan.transmits.len() > 1
}

/// Take the observation lock, recovering from poisoning.
///
/// A poisoned lock means a fold panicked. The observations are still
/// structurally valid — the fold is total — and dropping a record we just
/// learned is worse than carrying on.
fn lock(observations: &Mutex<Observations>) -> std::sync::MutexGuard<'_, Observations> {
    observations.lock().unwrap_or_else(|p| p.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::action::{plan, Action};
    use crate::state::{Observation, PhysAddr};

    fn addr(s: &str) -> PhysAddr {
        s.parse().unwrap()
    }

    /// The whole message table, asserted with no device present.
    #[test]
    fn every_intended_transmit_maps_to_its_message_and_destination() {
        let ours = addr("2.5.0.0");
        let cases = [
            (CecTx::ImageViewOn, Message::ImageViewOn, LogicalAddress::Tv),
            (
                CecTx::ActiveSource(ours),
                Message::ActiveSource {
                    address: phys(ours),
                },
                LogicalAddress::Broadcast,
            ),
            (
                CecTx::InactiveSource(ours),
                Message::InactiveSource {
                    address: phys(ours),
                },
                LogicalAddress::Tv,
            ),
            (
                CecTx::Standby(StandbyTarget::Tv),
                Message::Standby,
                LogicalAddress::Tv,
            ),
            (
                CecTx::Standby(StandbyTarget::AudioSystem),
                Message::Standby,
                LogicalAddress::AudioSystem,
            ),
            (
                CecTx::SetStreamPath(addr("1.0.0.0")),
                Message::SetStreamPath {
                    address: phys(addr("1.0.0.0")),
                },
                LogicalAddress::Broadcast,
            ),
        ];
        for (tx, message, destination) in cases {
            assert_eq!(message_for(tx), (message, destination), "{tx:?}");
        }
    }

    /// **The rule: wake uses `<Image View On>`, never `<Text View On>`.**
    ///
    /// `<Text View On>` additionally asks the television to dismiss whatever is
    /// on screen and show text — a visible side effect on someone's living room
    /// that nobody asked for.
    #[test]
    fn wake_uses_image_view_on_and_not_text_view_on() {
        let plan = plan(
            Action::Wake,
            Observation::Known(addr("2.5.0.0")),
            Observation::Unknown,
        )
        .unwrap();
        let messages: Vec<Message> = plan.transmits.iter().map(|tx| message_for(*tx).0).collect();
        assert!(messages.contains(&Message::ImageViewOn));
        assert!(!messages.contains(&Message::TextViewOn));
    }

    /// **The rule: a `<Standby>` is NEVER broadcast.**
    ///
    /// A broadcast standby powers off every device on the bus. The type system
    /// already makes it unspellable — [`StandbyTarget`] has no broadcast
    /// variant — and this asserts the translation keeps that promise, since a
    /// destination is the one place it could still be reintroduced.
    ///
    /// Mutation-check (run 2026-09-14): return `LogicalAddress::Broadcast` from
    /// either arm of `standby_destination` and this fails.
    #[test]
    fn a_standby_is_always_addressed_and_never_broadcast() {
        for target in StandbyTarget::ALL {
            let (message, destination) = message_for(CecTx::Standby(target));
            assert_eq!(message, Message::Standby);
            assert_ne!(destination, LogicalAddress::Broadcast, "{target:?}");
            assert_ne!(destination, LogicalAddress::Unregistered, "{target:?}");
            assert!(
                matches!(
                    destination,
                    LogicalAddress::Tv | LogicalAddress::AudioSystem
                ),
                "{target:?} -> {destination:?}"
            );
        }
        // And the whole standby plan, so a future edit that adds a third target
        // is covered too.
        let ours = Observation::Known(addr("2.5.0.0"));
        let plan = plan(Action::Standby, ours, ours).unwrap();
        for tx in plan.transmits {
            assert_ne!(message_for(tx).1, LogicalAddress::Broadcast, "{tx:?}");
        }
    }

    /// The power read-back is a request/reply pair, addressed and typed.
    #[test]
    fn the_power_query_names_the_reply_it_waits_for() {
        let (message, destination, reply) = power_query_for(PowerRead {
            target: StandbyTarget::Tv,
        });
        assert_eq!(message, Message::GiveDevicePowerStatus);
        assert_eq!(destination, LogicalAddress::Tv);
        assert_eq!(reply, Opcode::ReportPowerStatus);
        assert_eq!(
            power_read_device(PowerRead {
                target: StandbyTarget::AudioSystem
            }),
            AvDevice::AudioSystem
        );
        // Bounded, and at the kernel's own ceiling — anything larger is coerced
        // to one second by CEC_TRANSMIT, so a longer value would be a fiction.
        assert_eq!(REPLY_TIMEOUT.as_ms(), 1000);
        assert!(REPLY_TIMEOUT.as_ms() > 0, "a zero timeout is coerced too");
    }

    /// An unrecognised power status is `None`, never the nearest of the four.
    #[test]
    fn an_unrecognised_power_status_is_not_folded_into_a_neighbour() {
        use linux_cec::operand::PowerStatus;
        assert_eq!(power_state(PowerStatus::On), Some(PowerState::On));
        assert_eq!(power_state(PowerStatus::Standby), Some(PowerState::Standby));
        assert_eq!(power_state(PowerStatus::ToOn), Some(PowerState::ToOn));
        assert_eq!(
            power_state(PowerStatus::ToStandby),
            Some(PowerState::ToStandby)
        );
    }
}
