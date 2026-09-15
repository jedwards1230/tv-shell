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
//!
//! # It also feeds three of the four health facts
//!
//! Everything here is passive, which is the point: the daemon learns whether it
//! is still hearing the bus WITHOUT asking the bus anything.
//!
//! * every received message is **fact 4** (`last_rx`) — any message, including
//!   one this daemon does not otherwise track, because the question is "did we
//!   hear anything";
//! * a `PinEvent` is **fact 3** (`bus_activity`), the line-level signal that
//!   separates a quiet bus from a deaf adapter — see [`crate::health`] for why
//!   it is not in force on this deployment;
//! * a `StateChange` invalidates **fact 2** and is immediately followed by a
//!   re-read of the addressing, so the `unknown` it produces lasts milliseconds
//!   rather than until the next watchdog probe.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use linux_cec::device::{AsyncDevice, MessageData, PollResult, PollTimeout};
use linux_cec::message::Message;
use linux_cec::LogicalAddress;

use crate::failover::{Failover, Observed};
use crate::health::Health;
use crate::kernel::device::{log_transition, read_addressing};
use crate::state::{now_ms, AvDevice, BusObservation, Observations, PhysAddr};

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
pub async fn run(
    device: Arc<AsyncDevice>,
    observations: Arc<Mutex<Observations>>,
    health: Arc<Mutex<Health>>,
    failover: Arc<Mutex<Failover>>,
) {
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

    tracing::info!("receive loop running (this loop listens and folds; it transmits nothing)");
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
            note_health(&device, &health, &failover, &result).await;
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

/// Fold one poll result into the health facts.
///
/// Separate from [`fold`] because the two answer different questions: `fold`
/// asks "what does this mean for the published AV state", this asks "what does
/// it tell us about whether we can still hear". A `<Report Power Status>` from
/// the PS5 changes nothing in the first and is proof of life in the second.
///
/// **No lock is held across the `.await`.** The `StateChange` arm re-reads the
/// adapter's addressing, and holding the health lock across that ioctl would let
/// a slow device block `av-health` — the one verb that has to answer when the
/// device is what is being diagnosed.
async fn note_health(
    device: &AsyncDevice,
    health: &Mutex<Health>,
    failover: &Mutex<Failover>,
    result: &PollResult,
) {
    let lock = |f: &mut dyn FnMut(&mut Health)| match health.lock() {
        Ok(mut h) => f(&mut h),
        Err(poisoned) => f(&mut poisoned.into_inner()),
    };
    match result {
        // Fact 4. Any message at all — this is "we are still hearing".
        PollResult::Message(_) => {
            lock(&mut |h| h.record_rx(now_ms()));
            // …and the same fact is what clears the transmit-side failover rule:
            // a bus we can still hear is not a deaf adapter, so a run of
            // transmit failures alongside live traffic says something about the
            // device that did not answer, not about us.
            note_failover(failover, Observed::Rx);
        }
        // Fact 3. Line-level activity, observed passively.
        PollResult::PinEvent(event) => {
            tracing::debug!("CEC pin event: {event:?}");
            lock(&mut |h| h.record_bus_activity(now_ms()));
        }
        // Fact 2 is stale from this instant. Marked first so a concurrent
        // `av-health` between the two statements reports `unknown` rather than
        // an addressing the kernel has just told us not to trust.
        PollResult::StateChange => {
            lock(&mut |h| h.record_state_change(now_ms()));
            let addressed = read_addressing(device).await;
            tracing::info!("adapter state change; addressing re-read as {addressed:?}");
            let mut state = None;
            lock(&mut |h| {
                h.record_addressing(addressed, now_ms());
                state = Some(h.state());
            });
            // **This is the un-failover path, and it is an EVENT.** The kernel
            // tells us the adapter regained its address; the addressing is
            // re-read on the spot and the resulting verdict — not a timer —
            // is what lets the warm-path decision come back to CEC. The device
            // was never closed, so there is nothing to re-open.
            if let Some(state) = state {
                note_failover(failover, Observed::Health(state));
            }
        }
        // A dropped message is not evidence either way about hearing: the
        // kernel dropped it, we did not fail to receive it. `state::Observations`
        // counts it.
        _ => {}
    }
}

/// Fold one observation into the warm-path decision, logging a change if it
/// caused one.
///
/// One line per change, never one per observation: this runs on every message
/// heard on the bus.
fn note_failover(failover: &Mutex<Failover>, observed: Observed) {
    let transition = match failover.lock() {
        Ok(mut f) => f.observe(observed, now_ms()),
        Err(poisoned) => poisoned.into_inner().observe(observed, now_ms()),
    };
    if let Some(t) = transition {
        log_transition(&t);
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
        // A pin event says nothing about the AV state — it is a voltage on a
        // wire, not a message. It IS a health fact, and `note_health` above is
        // where it lands.
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
            state: super::ops::power_state(*status)?,
        }),
        Message::ReportAudioStatus { status } => {
            // Only the AVR's audio status is the AVR's audio status. On a shared
            // bus another device could report one, and folding that in would
            // publish somebody else's volume as ours.
            if av_device(initiator) != AvDevice::AudioSystem {
                return None;
            }
            Some(BusObservation::AudioStatus {
                // `AudioStatus::volume` is the 7-bit field. CEC defines only
                // `0..=100` as levels and reserves `0x7F` for "audio volume
                // status unknown", which is what a receiver sends when it has
                // dropped out of system-audio mode — so the out-of-range case
                // is a real wire value, not a hypothetical. It becomes
                // `unknown`, never a clamped number and never `0`:
                // `volume::level_observation` is the one place that rule lives.
                // (`u8::try_from` cannot fail for a 7-bit field; `u8::MAX` is
                // out of range and so folds to `unknown` too.)
                volume: crate::volume::level_observation(
                    u8::try_from(status.volume()).unwrap_or(u8::MAX),
                ),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::PowerState;
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
                volume: crate::state::Observation::Known(37),
                muted: true,
            })
        );
    }

    /// **The rule: an `<Report Audio Status>` carrying CEC's "volume unknown"
    /// encoding folds to `unknown`, never to a number.**
    ///
    /// `0x7F` is the value a receiver sends when it does not know its own
    /// volume — a real wire value, which is why the fold has to handle it rather
    /// than clamping it into the 0-100 range. The mute bit is still meaningful
    /// and survives.
    ///
    /// Mutation-check (run 2026-09-14): drop the `level_observation` call and
    /// fold the raw byte through, and this fails (it becomes `Known(127)`);
    /// coerce it to `Known(0)` and it fails too.
    #[test]
    fn a_volume_the_avr_does_not_know_folds_to_unknown() {
        use crate::state::Observation;
        for raw in [crate::volume::LEVEL_UNKNOWN, 101, 126] {
            assert_eq!(
                observation_for(
                    &Message::ReportAudioStatus {
                        status: AudioStatus::new()
                            .with_volume(usize::from(raw))
                            .with_mute(true),
                    },
                    LogicalAddress::AudioSystem,
                ),
                Some(BusObservation::AudioStatus {
                    volume: Observation::Unknown,
                    muted: true,
                }),
                "{raw:#x}"
            );
        }
        // …and a real level still comes through as itself, so the rule is not
        // vacuously passing by making everything unknown.
        assert_eq!(
            observation_for(
                &Message::ReportAudioStatus {
                    status: AudioStatus::new().with_volume(100).with_mute(false),
                },
                LogicalAddress::AudioSystem,
            ),
            Some(BusObservation::AudioStatus {
                volume: Observation::Known(100),
                muted: false,
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

    /// **Reachability: every ownership state the gates are tested against is
    /// produced by THIS path, not poked into the store.**
    ///
    /// A test that asserts an unreachable state defends nothing. So this walks
    /// the whole chain once — a `linux-cec` `Message` off the wire, through
    /// `observation_for`, through the fold, to the verdict the transmit gates
    /// read — for each of the three verdicts and for the malformed broadcast
    /// that makes `f.f.f.f` a real input rather than a hypothetical one.
    #[test]
    fn the_receive_path_can_produce_every_ownership_verdict() {
        use crate::ownership::Ownership;
        use crate::state::Observation;

        let ours = Observation::Known("2.5.0.0".parse::<PhysAddr>().unwrap());
        let mut obs = Observations::default();

        // 1. Nothing heard yet.
        assert_eq!(obs.ownership(ours).state, Ownership::Unknown);

        // 2. A real broadcast naming somebody else.
        let heard = observation_for(
            &Message::ActiveSource {
                address: phys("1.0.0.0"),
            },
            LogicalAddress::PlaybackDevice2,
        )
        .expect("an <Active Source> is an observation");
        obs.apply(heard, 10);
        assert_eq!(obs.ownership(ours).state, Ownership::OwnedByOther);

        // 3. A real broadcast naming us.
        let heard = observation_for(
            &Message::ActiveSource {
                address: phys("2.5.0.0"),
            },
            LogicalAddress::PlaybackDevice1,
        )
        .expect("an <Active Source> is an observation");
        obs.apply(heard, 20);
        assert_eq!(obs.ownership(ours).state, Ownership::OwnedByUs);

        // 4. `f.f.f.f` really can arrive: the payload is folded verbatim rather
        //    than being filtered, so `ownership::is_addressable`'s false branch
        //    is reachable from the bus and not only from a unit test.
        let heard = observation_for(
            &Message::ActiveSource {
                address: phys("f.f.f.f"),
            },
            LogicalAddress::PlaybackDevice2,
        )
        .expect("a malformed <Active Source> is still an observation");
        assert_eq!(
            heard,
            BusObservation::ActiveSource(PhysAddr::INVALID),
            "the payload must reach the store unfiltered"
        );
        obs.apply(heard, 30);
        assert_eq!(obs.ownership(ours).state, Ownership::Unknown);

        // 5. And a release returns it to unknown.
        let heard = observation_for(
            &Message::InactiveSource {
                address: phys("1.0.0.0"),
            },
            LogicalAddress::PlaybackDevice2,
        )
        .expect("an <Inactive Source> is an observation");
        obs.apply(heard, 40);
        assert_eq!(obs.ownership(ours).state, Ownership::Unknown);
        assert!(obs.ownership(ours).ever_observed);
    }

    /// The poll timeout is bounded, so a stop does not wait on an idle bus.
    #[test]
    fn the_poll_interval_is_bounded() {
        assert!(PollTimeout::try_from(POLL_INTERVAL).is_ok());
        assert!(POLL_INTERVAL <= Duration::from_secs(5));
    }
}
