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

use crate::state::AvState;

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
}

#[cfg(test)]
pub(crate) mod testing {
    use super::*;
    use crate::state::{AvState, Observation, Observations, PhysAddr, Topology};
    use std::sync::Mutex;

    /// A stand-in backend, so the IPC layer is exercised with no adapter — the
    /// role `FakeCompositor` plays in `core/src/ipc.rs`'s tests.
    pub(crate) struct FakeBackend {
        topology: Topology,
        observations: Mutex<Observations>,
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
            }
        }

        /// Fold an observation in, as the real rx loop would.
        pub(crate) fn observe(&self, obs: crate::state::BusObservation, now_ms: u64) {
            self.observations.lock().unwrap().apply(obs, now_ms);
        }
    }

    #[async_trait::async_trait]
    impl AvBackend for FakeBackend {
        async fn snapshot(&self) -> AvState {
            AvState::assemble(&self.topology, &self.observations.lock().unwrap())
        }
    }
}
