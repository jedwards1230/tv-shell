//! The receive loop — the daemon's only ear on the bus.
//!
//! It **listens and folds**, and it transmits nothing. Every `linux-cec` type
//! stops in [`observation_for`]; what leaves this module is
//! [`crate::state::BusObservation`], this crate's own vocabulary, which is what
//! lets the whole "what does this mean for the published state" decision live in
//! a pure function CI can cover with no adapter.
//!
//! # Why a poller rather than `rx_message`
//!
//! `rx_message` returns messages only. `PollResult` also carries
//! `StateChange` — the kernel telling us the adapter gained or lost its
//! physical/logical address, with no probe of ours — and `LostMessages`, which
//! says the snapshot may have missed an edge. Both are observations worth
//! having, and `StateChange` is the event step 7's un-failover is built on
//! ("recovery is an event, not a poll").

use std::sync::{Arc, Mutex};
use std::time::Duration;

use linux_cec::device::{AsyncDevice, MessageData, PollResult, PollTimeout};
use linux_cec::message::Message;
use linux_cec::LogicalAddress;

use crate::state::{now_ms, AvDevice, BusObservation, Observations, PhysAddr, PowerState};

/// How long one poll waits before coming back empty.
///
/// Bounded rather than infinite so a stop is prompt: `AsyncDevicePoller`'s
/// `Drop` joins its thread, and a thread parked in an unbounded `poll(2)` would
/// make shutdown wait for the next bus event — which on an idle bus is never.
const POLL_INTERVAL: Duration = Duration::from_secs(1);

/// Poll the adapter forever, folding what it hears into `observations`.
///
/// Returns only if the device becomes unusable. The caller decides what that
/// means; today `main` logs it and the watchdog stops being fed, so systemd
/// restarts the unit on a bounded timer.
pub async fn run(device: Arc<AsyncDevice>, observations: Arc<Mutex<Observations>>) {
    let timeout = match PollTimeout::try_from(POLL_INTERVAL) {
        Ok(t) => t,
        // Unreachable for a one-second constant, and still not a panic: this is
        // a long-running daemon, and `PollTimeout::ZERO` degrades to a busy-ish
        // loop rather than taking the process down.
        Err(e) => {
            tracing::error!("{POLL_INTERVAL:?} is not a valid poll timeout ({e}); using zero");
            PollTimeout::ZERO
        }
    };

    let poller = match device.get_poller().await {
        Ok(p) => p,
        Err(e) => {
            tracing::error!("cannot create a poller for the CEC device: {e}");
            return;
        }
    };

    tracing::info!("receive loop running (listening only — this daemon transmits nothing)");
    loop {
        let status = match poller.poll(timeout).await {
            Ok(s) => s,
            Err(e) => {
                tracing::error!("polling the CEC device failed: {e}");
                return;
            }
        };
        let results = match device.handle_status(status).await {
            Ok(r) => r,
            Err(e) => {
                // One bad read is not a reason to stop listening — a
                // malformed frame from a third device on a shared bus is
                // exactly the kind of thing that happens. A poll failure
                // above is different: that is our own fd.
                tracing::debug!("reading the CEC queues: {e}");
                continue;
            }
        };
        for result in results {
            let Some(observation) = fold(&result) else {
                continue;
            };
            tracing::debug!("bus observation: {observation:?}");
            // The lock is taken per observation and released immediately; it is
            // never held across an `.await`, so `av-state` cannot block behind
            // this loop.
            match observations.lock() {
                Ok(mut o) => o.apply(observation, now_ms()),
                Err(poisoned) => poisoned.into_inner().apply(observation, now_ms()),
            }
        }
    }
}

/// Translate one `linux-cec` poll result into this crate's vocabulary.
///
/// `None` means "nothing this daemon publishes changed" — a message from a third
/// device on the shared bus, or one whose opcode we do not track.
fn fold(result: &PollResult) -> Option<BusObservation> {
    match result {
        PollResult::Message(envelope) => match &envelope.message {
            MessageData::Valid(message) => observation_for(message, envelope.initiator),
            // A frame the crate could not parse. Logged by `linux-cec` itself;
            // it tells us nothing about the state we publish.
            MessageData::Invalid(_) => None,
        },
        PollResult::StateChange => Some(BusObservation::StateChange),
        PollResult::LostMessages(n) => Some(BusObservation::LostMessages(*n)),
        // The pin monitor is not enabled in this step (follower mode is
        // `Enabled`, not `MonitorPin`), and whether `pulse8-cec` even offers
        // `CEC_CAP_MONITOR_PIN` is unverified. Step 6 turns pin events into a
        // health signal; there is nothing for them to mean yet.
        PollResult::PinEvent(_) => None,
        // `PollResult` is `#[non_exhaustive]`: a future variant is something
        // this daemon has not been taught to read, which is exactly "no change
        // to what we publish" rather than an error.
        _ => None,
    }
}

/// What a received message means for the published state.
///
/// Separated from [`fold`] so it is unit-testable: `Envelope` carries a kernel
/// timestamp with no public constructor, but a [`Message`] and a
/// [`LogicalAddress`] are both ordinary values.
fn observation_for(message: &Message, initiator: LogicalAddress) -> Option<BusObservation> {
    match message {
        Message::ActiveSource { address } => Some(BusObservation::ActiveSource(
            PhysAddr::from_raw(u16::from(*address)),
        )),
        Message::InactiveSource { address } => Some(BusObservation::InactiveSource(
            PhysAddr::from_raw(u16::from(*address)),
        )),
        Message::ReportPowerStatus { status } => Some(BusObservation::PowerStatus {
            device: av_device(initiator),
            state: power_state(*status)?,
        }),
        Message::ReportAudioStatus { status } => {
            // Only the AVR's audio status is the AVR's audio status. On a shared
            // bus another device could report one, and folding that in would
            // publish somebody else's volume as ours.
            if av_device(initiator) != AvDevice::AudioSystem {
                return None;
            }
            Some(BusObservation::AudioStatus {
                // `AudioStatus::volume` is the 7-bit field; the CEC range is
                // 0-100 and `0x7F` means "no change". `u8::try_from` cannot fail
                // for a 7-bit field, and a value out of the 0-100 range is
                // reported as observed rather than clamped — clamping would
                // publish a number the AVR never sent.
                volume: u8::try_from(status.volume()).unwrap_or(u8::MAX),
                muted: status.mute(),
            })
        }
        _ => None,
    }
}

/// Which of the devices this daemon tracks an address belongs to.
fn av_device(address: LogicalAddress) -> AvDevice {
    match address {
        LogicalAddress::Tv => AvDevice::Tv,
        LogicalAddress::AudioSystem => AvDevice::AudioSystem,
        _ => AvDevice::Other,
    }
}

/// The four power states CEC defines.
///
/// `PowerStatus` is `#[non_exhaustive]`, and a status this daemon does not
/// recognise yields `None` rather than being folded into the nearest of the
/// four. Reporting an unrecognised value as `on` or `standby` would be exactly
/// the confident-and-wrong answer this crate publishes `unknown` to avoid.
fn power_state(status: linux_cec::operand::PowerStatus) -> Option<PowerState> {
    use linux_cec::operand::PowerStatus;
    Some(match status {
        PowerStatus::On => PowerState::On,
        PowerStatus::Standby => PowerState::Standby,
        PowerStatus::ToOn => PowerState::ToOn,
        PowerStatus::ToStandby => PowerState::ToStandby,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use linux_cec::operand::{AudioStatus, PowerStatus};
    use linux_cec::PhysicalAddress;

    fn phys(s: &str) -> PhysicalAddress {
        s.parse().unwrap()
    }

    /// The four messages this step listens for reach the published state.
    #[test]
    fn the_tracked_messages_translate() {
        assert_eq!(
            observation_for(
                &Message::ActiveSource {
                    address: phys("2.5.0.0")
                },
                LogicalAddress::PlaybackDevice1,
            ),
            Some(BusObservation::ActiveSource("2.5.0.0".parse().unwrap()))
        );
        assert_eq!(
            observation_for(
                &Message::InactiveSource {
                    address: phys("1.0.0.0")
                },
                LogicalAddress::PlaybackDevice1,
            ),
            Some(BusObservation::InactiveSource("1.0.0.0".parse().unwrap()))
        );
        assert_eq!(
            observation_for(
                &Message::ReportPowerStatus {
                    status: PowerStatus::Standby
                },
                LogicalAddress::Tv,
            ),
            Some(BusObservation::PowerStatus {
                device: AvDevice::Tv,
                state: PowerState::Standby,
            })
        );
        assert_eq!(
            observation_for(
                &Message::ReportAudioStatus {
                    status: AudioStatus::new().with_volume(37).with_mute(true),
                },
                LogicalAddress::AudioSystem,
            ),
            Some(BusObservation::AudioStatus {
                volume: 37,
                muted: true,
            })
        );
    }

    /// **The rule: a power or audio report is attributed to its initiator, never
    /// assumed to be the television's or the AVR's.**
    ///
    /// The living-room bus carries an Apple TV and a PS5. Folding their reports
    /// in would publish another device's power state as the television's.
    #[test]
    fn a_third_party_report_is_not_attributed_to_the_tv_or_the_avr() {
        assert_eq!(
            observation_for(
                &Message::ReportPowerStatus {
                    status: PowerStatus::On
                },
                LogicalAddress::PlaybackDevice2,
            ),
            Some(BusObservation::PowerStatus {
                device: AvDevice::Other,
                state: PowerState::On,
            })
        );
        // And an audio status from anything but the audio system is dropped
        // outright — there is no "other device's volume" field to put it in.
        assert_eq!(
            observation_for(
                &Message::ReportAudioStatus {
                    status: AudioStatus::new().with_volume(99).with_mute(false),
                },
                LogicalAddress::PlaybackDevice2,
            ),
            None
        );
    }

    /// A message this step does not track changes nothing.
    #[test]
    fn an_untracked_message_yields_no_observation() {
        assert_eq!(
            observation_for(&Message::ImageViewOn, LogicalAddress::Tv),
            None
        );
        assert_eq!(observation_for(&Message::Standby, LogicalAddress::Tv), None);
    }

    /// The non-message poll results that do mean something.
    #[test]
    fn state_changes_and_lost_messages_are_observations() {
        assert_eq!(
            fold(&PollResult::StateChange),
            Some(BusObservation::StateChange)
        );
        assert_eq!(
            fold(&PollResult::LostMessages(5)),
            Some(BusObservation::LostMessages(5))
        );
    }

    /// The poll timeout is bounded, so a stop does not wait on an idle bus.
    #[test]
    fn the_poll_interval_is_bounded() {
        assert!(PollTimeout::try_from(POLL_INTERVAL).is_ok());
        assert!(POLL_INTERVAL <= Duration::from_secs(5));
    }
}
