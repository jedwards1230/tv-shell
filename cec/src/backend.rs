//! The backend seam — everything the IPC layer needs from an AV backend.
//!
//! # Why there is a trait here at all, this early
//!
//! Two reasons, and both are load-bearing rather than tidiness:
//!
//! 1. **`linux-cec` stops here.** It is version 0.2.1 with one author and ~2.7k
//!    downloads; Valve provenance and a complete API make it the right bet, but
//!    it is not widely exercised (plan §7 item 9). Keeping its types behind this
//!    trait — so [`crate::ipc`] never sees a `PhysicalAddress`, a `Message` or a
//!    `PollResult` — is what makes a swap to `cec_linux` or to hand-rolled
//!    `<linux/cec.h>` ioctls a contained change instead of a rewrite.
//! 2. **CI has no adapter.** With the snapshot behind a trait, the whole
//!    request/reply surface is exercised end-to-end on a runner with no
//!    `/dev/cecN` — the role `core/`'s `Compositor` trait plays for X, and v1's
//!    `fake_runtime` for evdev.
//!
//! # Why [`AvBackend::snapshot`] cannot block on the bus
//!
//! It answers from a **cached snapshot**, updated by the rx loop. It reads no
//! device and takes no lock the rx loop holds across an `.await`, so `av-state`
//! still answers when the rx loop is the thing being diagnosed. That is the
//! property v1's `cec-health` did not have: it inferred adapter health from the
//! outcome of *our own transmits*, so a probe of it was itself a bus interaction
//! — and the watchdog above it then inferred health a second time from IPC
//! reachability, which is how a deliberately stopped daemon read as a wedged
//! adapter and got "recovered" three times.
//!
//! The trait is `#[async_trait]` rather than a plain sync trait because steps
//! 4-7 add verbs that genuinely await the device (`wake`, `standby`,
//! `input-select`, the `volume` family), and they must be able to land here
//! without re-shaping the seam every caller holds.

use crate::action::Action;
use crate::state::AvState;
use crate::volume::{VolumeAction, VolumeState};

/// What became of an action.
///
/// **Three outcomes, not two, and the third is the point.** A refusal is not a
/// failure: nothing is broken, nothing was attempted, and zero messages reached
/// the bus. v1 replied `ok` to a skipped transmit, which made "we deliberately
/// declined to power off your television" indistinguishable from "we powered off
/// your television" — and a caller cannot tell those apart after the fact
/// either, because on a shared bus the television may well have gone off for
/// some other reason. Reported as `error:` it would instead read as a fault and
/// send an operator looking for a broken adapter, which is precisely the
/// `cec-health` failure shape this design is removing, one layer down.
///
/// So [`ActionOutcome::Refused`] gets its own reply token. See
/// [`crate::protocol::resp_refused`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ActionOutcome {
    /// Every message in the plan was accepted on the bus.
    ///
    /// **This means more than "it left the adapter".** `linux-cec`'s
    /// `tx_message` returns an error unless the kernel's `tx_status` carries
    /// `OK`, i.e. the frame was ACKed (or broadcast without a NAK). So `Done` is
    /// a bus-level acknowledgement, not a queue receipt — the distinction the
    /// plan for jedwards1230/tv-shell#504 insists on.
    Done,
    /// A gate declined. **Zero transmits**, by construction — see
    /// [`crate::action::Refusal`].
    Refused(String),
    /// A transmit was attempted and the bus did not accept it.
    Failed(String),
}

/// Everything the IPC layer needs from an AV backend.
///
/// Held as `Arc<dyn AvBackend>`, which is why it is `#[async_trait]`: native
/// `async fn` in traits is not yet dyn-compatible.
#[async_trait::async_trait]
pub trait AvBackend: Send + Sync + 'static {
    /// The latest snapshot of what has been observed.
    ///
    /// **Infallible by design.** There is no error case, because "nothing has
    /// told us" is not a failure — it is a state this reply exists to report,
    /// and it is reported as `unknown` per field. An `av-state` that could
    /// return `error:` would collapse "the daemon is broken" and "the bus has
    /// been quiet" into one answer.
    async fn snapshot(&self) -> AvState;

    /// Perform one power or input-switching action.
    ///
    /// The **decision** is not here: it is [`crate::action::plan`], a pure
    /// function of the action plus the two observed addresses, so the gates are
    /// covered by CI on a runner with no adapter. An implementation of this
    /// method reads its own observations, calls that planner, and either
    /// transmits the plan or returns the refusal verbatim.
    ///
    /// This is fallible-by-value rather than by `Result` because all three
    /// outcomes are real answers a caller acts on differently; see
    /// [`ActionOutcome`].
    async fn act(&self, action: Action) -> ActionOutcome;

    /// Step the AVR's volume, or set its mute flag.
    ///
    /// Same three outcomes and the same division of labour: the decision is
    /// [`crate::volume::plan`] and the sequence is [`crate::volume::execute`],
    /// both of which are covered by CI with no adapter. **`Done` means the AVR's
    /// own `<Report Audio Status>` showed the change**, not that a frame was
    /// accepted — a receiver ignores CEC from a non-selected input, so a
    /// transmit proves nothing on its own.
    async fn volume(&self, action: VolumeAction) -> ActionOutcome;

    /// The AVR's level and mute flag, with the source of each named.
    ///
    /// **Unlike [`AvBackend::snapshot`], this one may touch the bus.** It asks
    /// the AVR directly (`<Give Audio Status>`, bounded) and falls back to
    /// whatever the receive loop last heard, publishing `source` so a caller can
    /// tell a fresh answer from an old one. The two verbs differ deliberately:
    /// `av-state` is the diagnostic read that must still answer when the device
    /// is the thing being diagnosed, while `volume-state` is a question about
    /// the AVR, which only the AVR can answer. Callers bound their timeouts
    /// either way — see [`crate::ipc`].
    async fn volume_state(&self) -> VolumeState;
}

#[cfg(test)]
pub(crate) mod testing {
    use super::*;
    use crate::action::{plan, CecTx, LocalRecord};
    use crate::state::{
        AvDevice, AvState, BusObservation, Observation, Observations, PhysAddr, PowerState,
        Topology,
    };
    use crate::volume::{self, AudioReport, VolumeBus, VolumeKey, VolumeReply, VolumeTx};
    use std::sync::Mutex;

    /// A stand-in receiver, so the volume surface is exercised with no adapter
    /// and no AVR.
    pub(crate) struct FakeAvr {
        /// What `<Report Audio Status>` answers. `None` models a receiver that
        /// does not answer at all.
        pub(crate) report: Option<AudioReport>,
        pub(crate) system_audio_mode: bool,
        /// Whether it ACTS on a key press. `false` is the non-selected-input
        /// case: the frame is ACKed and the command ignored.
        pub(crate) acts: bool,
    }

    impl Default for FakeAvr {
        fn default() -> FakeAvr {
            FakeAvr {
                report: Some(AudioReport {
                    level: Observation::Known(40),
                    muted: false,
                }),
                system_audio_mode: true,
                acts: true,
            }
        }
    }

    /// A stand-in backend, so the IPC layer is exercised with no adapter — the
    /// role `FakeCompositor` plays in `core/src/ipc.rs`'s tests.
    ///
    /// **It fakes the wire, not the decision.** [`FakeBackend::act`] calls the
    /// same [`crate::action::plan`] the kernel backend does and records the
    /// [`CecTx`] list it produced; only the transmit itself is replaced. That is
    /// what makes "standby refused and transmitted nothing" an assertion about
    /// the real gate rather than about a second implementation of it.
    pub(crate) struct FakeBackend {
        topology: Topology,
        observations: Mutex<Observations>,
        /// Every message the backend would have put on the bus, in order.
        transmits: Mutex<Vec<CecTx>>,
        /// What a `<Give Device Power Status>` read-back returns. `None` — the
        /// default — is the honest common case: a device that does not answer.
        power_reply: Mutex<Option<PowerState>>,
        /// Make the next transmit fail, to exercise the `Failed` outcome.
        fail_transmits: Mutex<bool>,
        /// The receiver on the other end of the volume verbs.
        avr: Mutex<FakeAvr>,
        /// Every volume intent this backend would have put on the bus.
        volume_transmits: Mutex<Vec<VolumeTx>>,
    }

    impl FakeBackend {
        pub(crate) fn new() -> FakeBackend {
            FakeBackend {
                topology: Topology {
                    backend: "cec",
                    device: "/dev/cec0".into(),
                    phys_addr_configured: "2.5.0.0".parse::<PhysAddr>().unwrap(),
                    phys_addr_read_back: Observation::Known("2.5.0.0".parse::<PhysAddr>().unwrap()),
                    log_addrs: vec!["playback-device1".into()],
                    capabilities: vec!["PHYS_ADDR".into(), "LOG_ADDRS".into()],
                    monitor_pin: false,
                },
                observations: Mutex::new(Observations::default()),
                transmits: Mutex::new(Vec::new()),
                power_reply: Mutex::new(None),
                fail_transmits: Mutex::new(false),
                avr: Mutex::new(FakeAvr::default()),
                volume_transmits: Mutex::new(Vec::new()),
            }
        }

        /// A backend whose own physical address could not be read back, so
        /// every gate that needs one refuses.
        pub(crate) fn without_our_address() -> FakeBackend {
            let mut backend = FakeBackend::new();
            backend.topology.phys_addr_read_back = Observation::Unknown;
            backend
        }

        /// Put a differently-behaved receiver on the bus.
        pub(crate) fn set_avr(&self, avr: FakeAvr) {
            *self.avr.lock().unwrap() = avr;
        }

        /// Every volume intent this backend has put on the bus so far.
        pub(crate) fn volume_transmits(&self) -> Vec<VolumeTx> {
            self.volume_transmits.lock().unwrap().clone()
        }

        /// Fold an observation in, as the real rx loop would.
        pub(crate) fn observe(&self, obs: BusObservation, now_ms: u64) {
            self.observations.lock().unwrap().apply(obs, now_ms);
        }

        /// Everything this backend has put on the bus so far.
        pub(crate) fn transmits(&self) -> Vec<CecTx> {
            self.transmits.lock().unwrap().clone()
        }

        /// What the TV answers a power query with.
        pub(crate) fn set_power_reply(&self, state: Option<PowerState>) {
            *self.power_reply.lock().unwrap() = state;
        }

        /// Make transmits fail, as a NAKing bus does.
        pub(crate) fn fail_transmits(&self) {
            *self.fail_transmits.lock().unwrap() = true;
        }
    }

    #[async_trait::async_trait]
    impl AvBackend for FakeBackend {
        async fn snapshot(&self) -> AvState {
            AvState::assemble(&self.topology, &self.observations.lock().unwrap())
        }

        async fn act(&self, action: Action) -> ActionOutcome {
            let owner = self.observations.lock().unwrap().active_source();
            let plan = match plan(action, self.topology.phys_addr_read_back, owner) {
                Ok(p) => p,
                // Verbatim, and BEFORE anything touches `transmits`: a refusal
                // is zero bus traffic.
                Err(refusal) => return ActionOutcome::Refused(refusal.reason),
            };
            if *self.fail_transmits.lock().unwrap() {
                return ActionOutcome::Failed("fake bus NAKed the transmit".into());
            }
            self.transmits.lock().unwrap().extend(plan.transmits);
            let now = 1_700_000_000_000;
            let mut observations = self.observations.lock().unwrap();
            match plan.record {
                Some(LocalRecord::Claimed(a)) => observations.record_our_claim(a, now),
                Some(LocalRecord::Released) => observations.record_our_release(now),
                None => {}
            }
            if plan.then_read.is_some() {
                if let Some(state) = *self.power_reply.lock().unwrap() {
                    observations.apply(
                        BusObservation::PowerStatus {
                            device: AvDevice::Tv,
                            state,
                        },
                        now,
                    );
                }
            }
            ActionOutcome::Done
        }

        /// The **real** sequence, against the stand-in receiver.
        ///
        /// [`crate::volume::execute`] has exactly one implementation and this
        /// drives it, so "the AVR ignored it and we said so" is an assertion
        /// about the shipped decision rather than about a second copy of it.
        async fn volume(&self, action: VolumeAction) -> ActionOutcome {
            let plan = match volume::plan(action, self.topology.phys_addr_read_back) {
                Ok(p) => p,
                Err(refusal) => return ActionOutcome::Refused(refusal.reason),
            };
            volume::execute(&FakeVolumeBus(self), plan).await
        }

        async fn volume_state(&self) -> VolumeState {
            // Same order as the kernel backend: ask the AVR, and fall back to
            // what was heard.
            let bus = FakeVolumeBus(self);
            if let Ok(VolumeReply::Audio(report)) = bus.perform(VolumeTx::AudioStatusQuery).await {
                return VolumeState::from_report(report, 1_700_000_000_000);
            }
            let observations = self.observations.lock().unwrap();
            VolumeState::from_observations(
                observations.volume(),
                observations.muted(),
                observations.observed_at(),
            )
        }
    }

    /// The stand-in wire the fake backend's volume verbs run over.
    struct FakeVolumeBus<'a>(&'a FakeBackend);

    #[async_trait::async_trait]
    impl VolumeBus for FakeVolumeBus<'_> {
        async fn perform(&self, tx: VolumeTx) -> Result<VolumeReply, String> {
            self.0.volume_transmits.lock().unwrap().push(tx);
            let mut avr = self.0.avr.lock().unwrap();
            match tx {
                VolumeTx::SystemAudioModeQuery => {
                    Ok(VolumeReply::SystemAudioMode(avr.system_audio_mode))
                }
                VolumeTx::SystemAudioModeRequest(_) => {
                    avr.system_audio_mode = true;
                    Ok(VolumeReply::SystemAudioMode(true))
                }
                VolumeTx::AudioStatusQuery => match avr.report {
                    Some(report) => {
                        // As the real bus does: a read-back reaches the
                        // published state, so `av-state` reflects it.
                        self.0.observations.lock().unwrap().apply(
                            BusObservation::AudioStatus {
                                volume: report.level,
                                muted: report.muted,
                            },
                            1_700_000_000_000,
                        );
                        Ok(VolumeReply::Audio(report))
                    }
                    None => Err("no reply within the timeout".to_string()),
                },
                VolumeTx::KeyPressAndRelease(key) => {
                    if !avr.acts {
                        return Ok(VolumeReply::None);
                    }
                    if let Some(report) = avr.report.as_mut() {
                        match (key, report.level) {
                            (VolumeKey::VolumeUp, Observation::Known(l)) => {
                                report.level = Observation::Known(
                                    l.saturating_add(1).min(crate::volume::MAX_LEVEL),
                                );
                            }
                            (VolumeKey::VolumeDown, Observation::Known(l)) => {
                                report.level = Observation::Known(l.saturating_sub(1));
                            }
                            (VolumeKey::MuteToggle, _) => report.muted = !report.muted,
                            _ => {}
                        }
                    }
                    Ok(VolumeReply::None)
                }
            }
        }
    }
}
