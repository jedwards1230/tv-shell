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
use linux_cec::operand::{AudioStatus, UiCommand};
use linux_cec::{LogicalAddress, PhysicalAddress, Timeout};

use crate::action::{CecTx, LocalRecord, Plan, PowerRead, StandbyTarget};
use crate::backend::ActionOutcome;
use crate::state::{now_ms, AvDevice, BusObservation, Observations, PowerState};
use crate::volume::{AudioReport, VolumeBus, VolumeKey, VolumeReply, VolumeTx};

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
/// Small on purpose: two methods, both taking `linux-cec` values that the pure
/// tables above produce. Keeping it here rather than on
/// [`crate::backend::AvBackend`] means the decision layer never sees a device,
/// and the device layer never sees a verb.
///
/// [`Transmitter::request`] is deliberately generic over the reply opcode rather
/// than being one method per query. Everything this daemon asks the bus —
/// `<Give Device Power Status>`, `<Give Audio Status>`,
/// `<Give System Audio Mode Status>`, `<System Audio Mode Request>` — is one
/// `tx_rx_message` with a bounded timeout, and the *parsing* of each reply is a
/// pure function beside it, which is what keeps the reply tables covered by CI
/// with no adapter.
#[async_trait::async_trait]
pub trait Transmitter: Send + Sync {
    /// Put one message on the bus, returning an error unless the bus accepted
    /// it.
    async fn send(&self, message: &Message, destination: LogicalAddress) -> Result<(), String>;

    /// Transmit and wait, bounded, for a reply carrying `reply`.
    async fn request(
        &self,
        message: &Message,
        destination: LogicalAddress,
        reply: Opcode,
    ) -> Result<Message, String>;
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

    async fn request(
        &self,
        message: &Message,
        destination: LogicalAddress,
        reply: Opcode,
    ) -> Result<Message, String> {
        let envelope = self
            .device
            .tx_rx_message(message, destination, reply, REPLY_TIMEOUT)
            .await
            .map_err(|e| format!("{e}"))?;
        match envelope.message {
            linux_cec::device::MessageData::Valid(message) => Ok(message),
            // A reply the crate could not parse tells us nothing, and must not
            // be folded into the nearest thing we were hoping for.
            other => Err(format!("{destination} answered with {other:?}")),
        }
    }
}

/// Ask a device for its power state and wait, bounded, for the reply.
pub async fn query_power(
    transmitter: &dyn Transmitter,
    read: PowerRead,
) -> Result<PowerState, String> {
    let (message, destination, reply) = power_query_for(read);
    match transmitter.request(&message, destination, reply).await? {
        Message::ReportPowerStatus { status } => power_state(status).ok_or_else(|| {
            format!("{destination} reported a power status this daemon does not recognise")
        }),
        other => Err(format!(
            "{destination} answered <Give Device Power Status> with {other:?}"
        )),
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
        match query_power(transmitter, read).await {
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

// ---------------------------------------------------------------------------
// Volume — the wire half.
// ---------------------------------------------------------------------------

/// What one [`VolumeTx`] is on the wire.
///
/// **[`Wire::KeyPair`] carries BOTH halves of a key press in one value**, which
/// is how the press/release pairing is made structural rather than conventional.
/// There is no variant, and no function in this crate, that yields a
/// `<User Control Pressed>` on its own — so no future edit can send one and
/// forget the release. An unreleased press auto-repeats on the AVR: the volume
/// runs away until something releases it.
// `LogicalAddress` is `PartialEq` but not `Eq` in `linux-cec` 0.2.1, so this
// derives what the dependency allows.
#[derive(Debug, Clone, PartialEq)]
pub enum Wire {
    /// Transmit and wait, bounded, for a reply of this opcode.
    Request {
        message: Message,
        destination: LogicalAddress,
        reply: Opcode,
    },
    /// A press and its release, to the same destination, in that order.
    KeyPair {
        press: Message,
        release: Message,
        destination: LogicalAddress,
    },
}

/// Translate one intended volume transmit into what goes on the wire.
///
/// **Pure**, on the no-device side of the seam, exactly like [`message_for`]:
/// it is the whole volume message table and CI has no adapter.
///
/// Everything here is addressed to `LogicalAddress::AudioSystem`. Volume is the
/// AVR's business; broadcasting a volume key would offer it to every device on a
/// bus that also carries an Apple TV and a PS5.
#[must_use]
pub fn wire_for(tx: VolumeTx) -> Wire {
    match tx {
        VolumeTx::SystemAudioModeQuery => Wire::Request {
            message: Message::GiveSystemAudioModeStatus,
            destination: LogicalAddress::AudioSystem,
            reply: Opcode::SystemAudioModeStatus,
        },
        // The AVR answers a request with `<Set System Audio Mode>`, not with a
        // status message — so that is the opcode to wait for.
        VolumeTx::SystemAudioModeRequest(addr) => Wire::Request {
            message: Message::SystemAudioModeRequest {
                physical_address: phys(addr),
            },
            destination: LogicalAddress::AudioSystem,
            reply: Opcode::SetSystemAudioMode,
        },
        VolumeTx::AudioStatusQuery => Wire::Request {
            message: Message::GiveAudioStatus,
            destination: LogicalAddress::AudioSystem,
            reply: Opcode::ReportAudioStatus,
        },
        VolumeTx::KeyPressAndRelease(key) => Wire::KeyPair {
            press: Message::UserControlPressed {
                ui_command: ui_command_for(key),
            },
            release: Message::UserControlReleased,
            destination: LogicalAddress::AudioSystem,
        },
    }
}

/// The UI command a key becomes.
///
/// `MuteToggle` is `UiCommand::Mute` (`0x43`), which is a **toggle**. The
/// absolute `MuteFunction` (`0x65`) and `RestoreVolumeFunction` (`0x66`) exist
/// in this enum and are deliberately not used: both are optional in the
/// specification and widely unimplemented, so an `unmute` built on them would
/// silently do nothing on the receivers that lack them. The idempotence comes
/// from [`crate::volume::mute_step`]'s read-back instead — see that module's
/// docs.
#[must_use]
pub fn ui_command_for(key: VolumeKey) -> UiCommand {
    match key {
        VolumeKey::VolumeUp => UiCommand::VolumeUp,
        VolumeKey::VolumeDown => UiCommand::VolumeDown,
        VolumeKey::MuteToggle => UiCommand::Mute,
    }
}

/// What a `<Report Audio Status>` payload means.
///
/// Pure. `0x7F` and everything above [`crate::volume::MAX_LEVEL`] become
/// `unknown` via [`crate::volume::level_observation`] — never a clamped number
/// and never `0`.
#[must_use]
pub fn audio_report(status: AudioStatus) -> AudioReport {
    AudioReport {
        // The field is 7 bits, so the conversion cannot fail; `u8::MAX` is out
        // of range and folds to `unknown` anyway.
        level: crate::volume::level_observation(u8::try_from(status.volume()).unwrap_or(u8::MAX)),
        muted: status.mute(),
    }
}

/// The real bus, as [`crate::volume::execute`] needs it.
///
/// Holds the transmitter by reference so the whole volume sequence can be driven
/// against a recording stand-in in this module's tests — which is where the
/// press/release pairing and the system-audio-mode step are asserted **on the
/// messages produced**, not on which readers were called.
pub struct DeviceVolumeBus<'a> {
    transmitter: &'a dyn Transmitter,
    observations: &'a Mutex<Observations>,
}

impl<'a> DeviceVolumeBus<'a> {
    #[must_use]
    pub fn new(
        transmitter: &'a dyn Transmitter,
        observations: &'a Mutex<Observations>,
    ) -> DeviceVolumeBus<'a> {
        DeviceVolumeBus {
            transmitter,
            observations,
        }
    }
}

#[async_trait::async_trait]
impl VolumeBus for DeviceVolumeBus<'_> {
    async fn perform(&self, tx: VolumeTx) -> Result<VolumeReply, String> {
        match wire_for(tx) {
            Wire::Request {
                message,
                destination,
                reply,
            } => {
                let answer = self
                    .transmitter
                    .request(&message, destination, reply)
                    .await?;
                interpret(answer, destination, self.observations)
            }
            Wire::KeyPair {
                press,
                release,
                destination,
            } => {
                let pressed = self.transmitter.send(&press, destination).await;
                // **ALWAYS, on every path.** The release goes out even when the
                // press was not accepted: if the NAK was spurious the AVR is now
                // auto-repeating, and an extra `<User Control Released>` on a
                // key that was never pressed is a no-op at every receiver. The
                // asymmetry is deliberate — one of these errors is a runaway
                // volume and the other is nothing at all.
                let released = self.transmitter.send(&release, destination).await;
                pressed?;
                released?;
                Ok(VolumeReply::None)
            }
        }
    }
}

/// What a reply to one of the volume queries means, and what it records.
///
/// A `<Report Audio Status>` is folded into [`Observations`] here, which is how
/// `av-state`'s `volume` and `muted` reflect what the last action read back —
/// our own request/reply exchanges never come past the receive loop.
fn interpret(
    answer: Message,
    destination: LogicalAddress,
    observations: &Mutex<Observations>,
) -> Result<VolumeReply, String> {
    match answer {
        Message::ReportAudioStatus { status } => {
            let report = audio_report(status);
            lock(observations).apply(
                BusObservation::AudioStatus {
                    volume: report.level,
                    muted: report.muted,
                },
                now_ms(),
            );
            Ok(VolumeReply::Audio(report))
        }
        Message::SystemAudioModeStatus { status } | Message::SetSystemAudioMode { status } => {
            Ok(VolumeReply::SystemAudioMode(status))
        }
        other => Err(format!("{destination} answered with {other:?}")),
    }
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

    // -----------------------------------------------------------------------
    // Volume — the wire table, and the press/release pairing.
    //
    // Asserted on the MESSAGES produced through the transmitter seam, which is
    // the discipline the rest of this crate's tests use: not on which readers
    // were consulted, but on what would have gone on the bus.
    // -----------------------------------------------------------------------

    use crate::volume::{self, AudioReport, VolumeAction, VolumeKey};
    use linux_cec::operand::AudioStatus;

    /// A transmitter that records everything and answers like an AVR.
    struct RecordingTransmitter {
        sent: Mutex<Vec<(Message, LogicalAddress)>>,
        /// What `<Give Audio Status>` answers. `None` = no reply.
        audio: Mutex<Option<AudioStatus>>,
        /// What `<Give System Audio Mode Status>` answers. `None` = no reply.
        system_audio_mode: Mutex<Option<bool>>,
        /// Whether a key press moves the reported level.
        acts: bool,
        /// Whether a `<User Control Pressed>` is NAKed.
        nak_press: bool,
    }

    impl RecordingTransmitter {
        fn new(level: u8, muted: bool) -> RecordingTransmitter {
            RecordingTransmitter {
                sent: Mutex::new(Vec::new()),
                audio: Mutex::new(Some(
                    AudioStatus::new()
                        .with_volume(usize::from(level))
                        .with_mute(muted),
                )),
                system_audio_mode: Mutex::new(Some(true)),
                acts: true,
                nak_press: false,
            }
        }

        fn sent(&self) -> Vec<(Message, LogicalAddress)> {
            self.sent.lock().unwrap().clone()
        }
    }

    #[async_trait::async_trait]
    impl Transmitter for RecordingTransmitter {
        async fn send(&self, message: &Message, destination: LogicalAddress) -> Result<(), String> {
            self.sent.lock().unwrap().push((*message, destination));
            if let Message::UserControlPressed { ui_command } = message {
                if self.nak_press {
                    return Err("the bus NAKed the press".to_string());
                }
                if self.acts {
                    let mut audio = self.audio.lock().unwrap();
                    if let Some(status) = audio.as_mut() {
                        match ui_command {
                            UiCommand::VolumeUp => {
                                *status = status.with_volume((status.volume() + 1).min(100));
                            }
                            UiCommand::VolumeDown => {
                                *status = status.with_volume(status.volume().saturating_sub(1));
                            }
                            UiCommand::Mute => *status = status.with_mute(!status.mute()),
                            _ => {}
                        }
                    }
                }
            }
            Ok(())
        }

        async fn request(
            &self,
            message: &Message,
            destination: LogicalAddress,
            _reply: Opcode,
        ) -> Result<Message, String> {
            self.sent.lock().unwrap().push((*message, destination));
            match message {
                Message::GiveAudioStatus => self
                    .audio
                    .lock()
                    .unwrap()
                    .map(|status| Message::ReportAudioStatus { status })
                    .ok_or_else(|| "no reply within the timeout".to_string()),
                Message::GiveSystemAudioModeStatus => self
                    .system_audio_mode
                    .lock()
                    .unwrap()
                    .map(|status| Message::SystemAudioModeStatus { status })
                    .ok_or_else(|| "no reply within the timeout".to_string()),
                Message::SystemAudioModeRequest { .. } => {
                    *self.system_audio_mode.lock().unwrap() = Some(true);
                    Ok(Message::SetSystemAudioMode { status: true })
                }
                other => Err(format!("nothing answers {other:?}")),
            }
        }
    }

    /// The whole volume message table, asserted with no device present.
    ///
    /// Everything is addressed to the audio system: broadcasting a volume key
    /// would offer it to every device on a bus that also carries an Apple TV and
    /// a PS5.
    #[test]
    fn every_volume_intent_maps_to_its_messages_and_destination() {
        let ours = addr("2.5.0.0");
        assert_eq!(
            wire_for(VolumeTx::SystemAudioModeQuery),
            Wire::Request {
                message: Message::GiveSystemAudioModeStatus,
                destination: LogicalAddress::AudioSystem,
                reply: Opcode::SystemAudioModeStatus,
            }
        );
        assert_eq!(
            wire_for(VolumeTx::SystemAudioModeRequest(ours)),
            Wire::Request {
                message: Message::SystemAudioModeRequest {
                    physical_address: phys(ours),
                },
                destination: LogicalAddress::AudioSystem,
                // The AVR answers a request with <Set System Audio Mode>.
                reply: Opcode::SetSystemAudioMode,
            }
        );
        assert_eq!(
            wire_for(VolumeTx::AudioStatusQuery),
            Wire::Request {
                message: Message::GiveAudioStatus,
                destination: LogicalAddress::AudioSystem,
                reply: Opcode::ReportAudioStatus,
            }
        );
        for (key, ui) in [
            (VolumeKey::VolumeUp, UiCommand::VolumeUp),
            (VolumeKey::VolumeDown, UiCommand::VolumeDown),
            // The toggle, deliberately — see `volume`'s module docs.
            (VolumeKey::MuteToggle, UiCommand::Mute),
        ] {
            assert_eq!(
                wire_for(VolumeTx::KeyPressAndRelease(key)),
                Wire::KeyPair {
                    press: Message::UserControlPressed { ui_command: ui },
                    release: Message::UserControlReleased,
                    destination: LogicalAddress::AudioSystem,
                },
                "{key:?}"
            );
        }
    }

    /// **THE PAIRING RULE: a `<User Control Pressed>` is ALWAYS followed by a
    /// `<User Control Released>`.**
    ///
    /// An unreleased press auto-repeats on the AVR — the volume runs away until
    /// something sends the release. The pairing is structural: `Wire::KeyPair`
    /// carries both messages in one value, so there is no way to spell a press
    /// on its own, and the bus sends both.
    ///
    /// Mutation-check (run 2026-09-14): drop the `release` from
    /// `wire_for`'s `KeyPressAndRelease` arm (or the second `send` from
    /// `DeviceVolumeBus::perform`) and this fails on every key, together with
    /// `a_naked_press_still_gets_its_release`.
    #[tokio::test]
    async fn a_key_press_is_always_followed_by_its_release() {
        for (key, ui) in [
            (VolumeKey::VolumeUp, UiCommand::VolumeUp),
            (VolumeKey::VolumeDown, UiCommand::VolumeDown),
            (VolumeKey::MuteToggle, UiCommand::Mute),
        ] {
            let transmitter = RecordingTransmitter::new(40, false);
            let observations = Mutex::new(Observations::default());
            let bus = DeviceVolumeBus::new(&transmitter, &observations);
            assert_eq!(
                bus.perform(VolumeTx::KeyPressAndRelease(key)).await,
                Ok(VolumeReply::None)
            );
            assert_eq!(
                transmitter.sent(),
                vec![
                    (
                        Message::UserControlPressed { ui_command: ui },
                        LogicalAddress::AudioSystem
                    ),
                    (Message::UserControlReleased, LogicalAddress::AudioSystem),
                ],
                "{key:?}"
            );
        }
    }

    /// **And the release goes out even when the press was NOT accepted.**
    ///
    /// The asymmetry is deliberate: if the NAK was spurious the AVR is now
    /// auto-repeating, while an extra `<User Control Released>` for a key that
    /// was never pressed is a no-op at every receiver. One of those errors
    /// is a runaway volume and the other is nothing at all.
    ///
    /// Mutation-check (run 2026-09-14): make `DeviceVolumeBus::perform` return
    /// early on a failed press (`self.transmitter.send(&press, …).await?;`) and
    /// this fails.
    #[tokio::test]
    async fn a_naked_press_still_gets_its_release() {
        let transmitter = RecordingTransmitter {
            nak_press: true,
            ..RecordingTransmitter::new(40, false)
        };
        let observations = Mutex::new(Observations::default());
        let bus = DeviceVolumeBus::new(&transmitter, &observations);
        let result = bus
            .perform(VolumeTx::KeyPressAndRelease(VolumeKey::VolumeUp))
            .await;
        assert!(result.is_err(), "a NAKed press must still be an error");
        assert!(
            transmitter
                .sent()
                .contains(&(Message::UserControlReleased, LogicalAddress::AudioSystem)),
            "the release must go out anyway: {:?}",
            transmitter.sent()
        );
    }

    /// The whole sequence, in `linux-cec` messages, against an AVR that is out
    /// of system-audio mode and then acts.
    ///
    /// This is the message-level twin of the `volume` module's sequence test:
    /// the request precedes the press, and the press precedes the read-back.
    #[tokio::test]
    async fn the_volume_sequence_reaches_the_bus_in_order() {
        let transmitter = RecordingTransmitter::new(40, false);
        *transmitter.system_audio_mode.lock().unwrap() = Some(false);
        let observations = Mutex::new(Observations::default());
        let bus = DeviceVolumeBus::new(&transmitter, &observations);
        let plan = volume::plan(VolumeAction::Up, Observation::Known(addr("2.5.0.0"))).unwrap();

        assert_eq!(volume::execute(&bus, plan).await, ActionOutcome::Done);
        assert_eq!(
            transmitter
                .sent()
                .into_iter()
                .map(|(m, _)| m)
                .collect::<Vec<_>>(),
            vec![
                Message::GiveSystemAudioModeStatus,
                Message::SystemAudioModeRequest {
                    physical_address: phys(addr("2.5.0.0")),
                },
                Message::GiveAudioStatus,
                Message::UserControlPressed {
                    ui_command: UiCommand::VolumeUp,
                },
                Message::UserControlReleased,
                Message::GiveAudioStatus,
            ]
        );
        // And the read-back reached the published state, which is how
        // `av-state`'s `volume` stops being null: our own request/reply
        // exchanges never come past the receive loop.
        assert_eq!(
            observations.lock().unwrap().volume(),
            Observation::Known(41)
        );
    }

    /// **The non-selected-input case, at the message seam: every frame is
    /// accepted and the action still reports a FAILURE.**
    ///
    /// Mutation-check (run 2026-09-14): make `volume::perform_level` report
    /// `Done` without consulting the read-back and this fails.
    #[tokio::test]
    async fn an_accepted_but_ignored_volume_command_is_a_failure_at_the_message_seam() {
        let transmitter = RecordingTransmitter {
            acts: false,
            ..RecordingTransmitter::new(40, false)
        };
        let observations = Mutex::new(Observations::default());
        let bus = DeviceVolumeBus::new(&transmitter, &observations);
        let plan = volume::plan(VolumeAction::Up, Observation::Known(addr("2.5.0.0"))).unwrap();

        let outcome = volume::execute(&bus, plan).await;
        let ActionOutcome::Failed(why) = outcome else {
            panic!("must report a failure, got {outcome:?}");
        };
        assert!(why.contains("non-selected input"), "{why}");
        // Every frame WAS accepted by the bus — this is a judged failure, not a
        // transmit error.
        assert!(transmitter.sent().iter().any(|(m, _)| matches!(
            m,
            Message::UserControlPressed {
                ui_command: UiCommand::VolumeUp
            }
        )));
    }

    /// A `<Report Audio Status>` payload becomes this crate's own report, with
    /// the unknown encoding preserved as unknown.
    #[test]
    fn an_audio_status_payload_keeps_its_unknowns() {
        assert_eq!(
            audio_report(AudioStatus::new().with_volume(37).with_mute(true)),
            AudioReport {
                level: Observation::Known(37),
                muted: true,
            }
        );
        assert_eq!(
            audio_report(
                AudioStatus::new()
                    .with_volume(usize::from(volume::LEVEL_UNKNOWN))
                    .with_mute(false),
            ),
            AudioReport {
                level: Observation::Unknown,
                muted: false,
            }
        );
    }
}
