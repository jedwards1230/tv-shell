//! `/dev/cecN` lifecycle: open, configure, read the topology back.
//!
//! # The OPEN sequence is transmit-free, and the ordering below is what makes
//! it so
//!
//! The living-room bus carries an Apple TV and a PS5 as well as the television
//! and the AVR, so a stray transmit from this daemon is a real-world side effect
//! on someone else's evening. Starting this daemon must therefore put **nothing**
//! on the bus: every message it sends comes from a client asking for one, via
//! [`crate::action::plan`] and [`crate::kernel::ops`].
//!
//! One call in the sequence would break that if it were moved:
//! **`set_osd_name` must be called BEFORE `set_logical_addresses`.**
//! `linux-cec`'s implementation sends a `<Set OSD Name>` message to the TV if a
//! logical address has already been claimed (`device.rs`: `if
//! self.tx_logical_address != LogicalAddress::Unregistered { … tx_message(…) }`),
//! and does not if one has not. Before the logical addresses are set we are
//! `Unregistered`, so the call is pure configuration — which is the difference
//! between "starting the daemon is silent on the bus" and "starting the daemon
//! announces itself to the television". The crate's own docs
//! require the same order for a different reason — the kernel only advertises
//! the OSD name on query if it was set first — so the two agree, but the
//! transmit is the one that matters here and it is why the order is pinned by a
//! comment rather than left to chance.
//!
//! **The one bus interaction that is not ours.** `CEC_ADAP_S_LOG_ADDRS` makes
//! the *kernel* poll the bus to allocate a logical address. That is the kernel's
//! own address-claim traffic, performed by every CEC device that attaches, and
//! it is unavoidable for an adapter that is going to have an address at all. No
//! message of this daemon's is transmitted. Stated here rather than left for a
//! reader to discover, because "no transmits" should mean what it says.
//!
//! # Capabilities are READ, never assumed
//!
//! `get_capabilities()` runs before anything is configured, and the result gates
//! the two calls that need it (`CEC_CAP_PHYS_ADDR`, `CEC_CAP_LOG_ADDRS`) and is
//! published verbatim in `av-state`. Whether `pulse8-cec` implements
//! `CEC_CAP_MONITOR_PIN` is UNVERIFIED — it could not be checked without a
//! device — so nothing here assumes the pin monitor exists. Step 6's `av-health`
//! reads [`crate::state::Topology::monitor_pin`] to name which health signal is
//! actually in force.

use std::sync::{Arc, Mutex};

use anyhow::{anyhow, Context, Result};
use linux_cec::device::{AsyncDevice, Capabilities};
use linux_cec::{FollowerMode, InitiatorMode, LogicalAddressType, PhysicalAddress};

use crate::action::Action;
use crate::backend::{ActionOutcome, AvBackend};
use crate::config::CecConfig;
use crate::health::{Health, HealthReport};
use crate::kernel::ops;
use crate::state::{now_ms, AvState, Observation, Observations, PhysAddr, Topology};
use crate::volume::{VolumeAction, VolumeBus, VolumeState};

/// An open kernel CEC adapter, its topology, and the observations folded out of
/// its receive queue.
pub struct KernelBackend {
    device: Arc<AsyncDevice>,
    topology: Topology,
    observations: Arc<Mutex<Observations>>,
    /// The four observed health facts. Shared with the receive loop, which
    /// records what it hears, and with the watchdog probe, which records facts
    /// 1 and 2. **Never held across an `.await`** — the same rule
    /// `observations` follows, and for the same reason: `av-health` must keep
    /// answering while the device is the thing being diagnosed.
    health: Arc<Mutex<Health>>,
}

impl KernelBackend {
    /// Open and configure the adapter named by `config`, then read its topology
    /// back.
    ///
    /// See the module docs for the ordering rules. Every step is logged, and the
    /// physical address is logged **as set and as read back** — a wrong
    /// `phys_addr` is otherwise silent, and silence is the whole problem with
    /// it.
    pub async fn open(config: &CecConfig) -> Result<KernelBackend> {
        let path = config.device.path.clone();
        let configured = config.phys_addr()?;

        let device = AsyncDevice::open(&path)
            .await
            .with_context(|| format!("opening the CEC device at {path}"))?;

        // 1. READ the capabilities, before configuring anything that depends on
        //    one. `get_capabilities` is also the daemon's liveness probe (the
        //    plan's fact 1): a wedged USB device fails it, a healthy idle bus
        //    does not.
        let caps = device
            .get_capabilities()
            .await
            .with_context(|| format!("reading CEC_ADAP_G_CAPS on {path}"))?;
        let capabilities = capability_names(caps);
        let monitor_pin = caps.contains(Capabilities::MONITOR_PIN);
        let driver = device
            .get_driver_name()
            .await
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|_| "<unknown>".to_string());
        tracing::info!(
            "opened {path}: driver {driver}, capabilities [{}], monitor_pin={monitor_pin}",
            capabilities.join(" ")
        );

        // 2. Initiator mode. Required by CEC_ADAP_S_LOG_ADDRS — the kernel
        //    refuses the address claim from a process that is not an initiator.
        //    It PERMITS transmitting; it does not transmit. This step sends
        //    nothing.
        device
            .set_initiator_mode(InitiatorMode::Enabled)
            .await
            .context("entering initiator mode (needed to configure logical addresses)")?;

        // 3. The physical address. Gated on the capability rather than attempted
        //    blindly, so a driver that cannot take one says so in a message
        //    naming the capability instead of an opaque ioctl errno.
        if !caps.contains(Capabilities::PHYS_ADDR) {
            return Err(anyhow!(
                "{path} does not report CEC_CAP_PHYS_ADDR, so its physical address cannot be \
                 set from userspace; capabilities are [{}]",
                capabilities.join(" ")
            ));
        }
        // `From<u16>`, not `TryFrom`: the 16-bit encoding IS a physical
        // address, including `f.f.f.f`. Validation of the operator's text form
        // happened in `config::validate`, where the error can name the key.
        let wanted = PhysicalAddress::from(configured.raw());
        device
            .set_physical_address(wanted)
            .await
            .with_context(|| format!("setting the physical address to {configured}"))?;

        // 4. The OSD name — BEFORE the logical addresses. See the module docs:
        //    after them, this call transmits.
        device
            .set_osd_name(&config.device.osd_name)
            .await
            .with_context(|| format!("setting the OSD name to {:?}", config.device.osd_name))?;

        // 5. The logical address. `Playback` is what an HTPC is on a CEC bus.
        if !caps.contains(Capabilities::LOG_ADDRS) {
            return Err(anyhow!(
                "{path} does not report CEC_CAP_LOG_ADDRS, so it cannot claim a logical \
                 address; capabilities are [{}]",
                capabilities.join(" ")
            ));
        }
        device
            .set_logical_addresses(&[LogicalAddressType::Playback])
            .await
            .context("claiming the Playback logical address")?;

        // 6. Follower mode, so the receive loop sees traffic addressed to us and
        //    broadcasts. `Enabled`, not `Exclusive`: an exclusive follower locks
        //    every other process out of the adapter for as long as this one
        //    holds it open, and the point of the kernel driver is that it is a
        //    shared, inspectable owner — `cec-ctl --monitor` has to keep working
        //    for anyone diagnosing the bus.
        device
            .set_follower_mode(FollowerMode::Enabled)
            .await
            .context("entering follower mode")?;

        // 7. Read the topology BACK. This is the whole answer to "a wrong
        //    phys_addr fails silently": the value set and the value read are
        //    both logged and both published.
        let read_back: Observation<PhysAddr> = match device.get_physical_address().await {
            Ok(a) => Observation::Known(PhysAddr::from_raw(u16::from(a))),
            Err(e) => {
                tracing::warn!("reading CEC_ADAP_G_PHYS_ADDR back on {path}: {e}");
                Observation::Unknown
            }
        };
        match read_back {
            Observation::Known(got) if got == configured => {
                tracing::info!("physical address {configured} set and read back unchanged");
            }
            Observation::Known(got) => {
                // Loud, because this is the failure that is otherwise invisible:
                // a later `<Active Source>` would address a port that does not
                // exist and the bus would report nothing about it.
                tracing::warn!(
                    "physical address MISMATCH on {path}: set {configured}, adapter reports \
                     {got}. `2.5.0.0` is the pre-2026-08-07 value and is unverified against \
                     the current rack — check `cec-ctl -d {path} --show-topology`"
                );
            }
            Observation::Unknown => {
                tracing::warn!(
                    "physical address {configured} was set but could not be read back; \
                     av-state will report physAddr as unknown"
                );
            }
        }

        let log_addrs: Vec<String> = match device.get_logical_addresses().await {
            Ok(addrs) => addrs.iter().map(ToString::to_string).collect(),
            Err(e) => {
                tracing::warn!("reading the logical addresses back on {path}: {e}");
                Vec::new()
            }
        };
        if log_addrs.is_empty() {
            tracing::warn!(
                "{path} holds no logical address; the adapter is attached but unaddressed, so \
                 nothing on the bus is addressed to us"
            );
        } else {
            tracing::info!("logical addresses: {}", log_addrs.join(" "));
        }

        // The health machine's opening facts: the fd answered (that is what
        // produced `caps`), and the addressing exactly as read back above. No
        // extra ioctl, and no assumption — `addressed` is `unknown` when the
        // read-back itself failed, which is a different thing from "no address".
        let addressed = addressing_from(read_back, &log_addrs);
        let health = Health::at_open(monitor_pin, addressed, now_ms());
        tracing::info!(
            "health at open: {} ({})",
            health.state().as_str(),
            health.report(now_ms()).reason
        );

        Ok(KernelBackend {
            device: Arc::new(device),
            health: Arc::new(Mutex::new(health)),
            topology: Topology {
                backend: "cec",
                device: path,
                phys_addr_configured: configured,
                phys_addr_read_back: read_back,
                log_addrs,
                capabilities,
                monitor_pin,
            },
            observations: Arc::new(Mutex::new(Observations::default())),
        })
    }

    /// The device handle, for the receive loop.
    #[must_use]
    pub fn device(&self) -> Arc<AsyncDevice> {
        Arc::clone(&self.device)
    }

    /// The shared observation store, for the receive loop.
    #[must_use]
    pub fn observations(&self) -> Arc<Mutex<Observations>> {
        Arc::clone(&self.observations)
    }

    /// The shared health facts, for the receive loop.
    #[must_use]
    pub fn health(&self) -> Arc<Mutex<Health>> {
        Arc::clone(&self.health)
    }

    /// Observe **facts 1 and 2**, record them, and answer the one question the
    /// watchdog loop asks: should `WATCHDOG=1` be sent?
    ///
    /// Both facts are pure ioctls on our own file descriptor —
    /// `CEC_ADAP_G_CAPS`, `CEC_ADAP_G_PHYS_ADDR`, `CEC_ADAP_G_LOG_ADDRS`. They
    /// touch the bus not at all, so unlike v1's `cec-health` — which inferred
    /// adapter health from the outcome of our own transmits — probing has no
    /// side effect on anyone's television. A wedged USB device fails fact 1; a
    /// bus where everything is switched off does not.
    ///
    /// The **decision** is [`Health::should_feed_watchdog`], not this function:
    /// the gate is fact 1 alone, and the reasoning for that lives with the state
    /// machine rather than inline here.
    pub async fn probe(&self) -> bool {
        let alive = match self.device.get_capabilities().await {
            Ok(_) => true,
            Err(e) => {
                tracing::warn!("CEC_ADAP_G_CAPS failed; the adapter fd is not answering: {e}");
                false
            }
        };
        // Fact 2 only when fact 1 held: with a dead fd the addressing reads
        // would fail too, and recording that as "unaddressed" would attribute
        // one fault to two facts.
        let addressed = if alive {
            Some(read_addressing(&self.device).await)
        } else {
            None
        };
        let now = now_ms();
        let mut health = self
            .health
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let before = health.state();
        health.record_fd_probe(alive, now);
        if let Some(addressed) = addressed {
            health.record_addressing(addressed, now);
        }
        if health.state() != before {
            // One line per transition, naming the observation. Not one per
            // probe: this runs every few seconds, and the journal on this box
            // retains ~21h (jedwards1230/tv-shell#509).
            tracing::info!(
                "health {} -> {}: {}",
                before.as_str(),
                health.state().as_str(),
                health.report(now).reason
            );
        }
        health.should_feed_watchdog()
    }

    /// Record a transmit the bus accepted (**fact 4**).
    fn note_tx(&self, outcome: &ActionOutcome) {
        if !matches!(outcome, ActionOutcome::Done) {
            return;
        }
        self.health
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .record_tx_ok(now_ms());
    }
}

/// **Fact 2**, read from the adapter: a valid physical address AND at least one
/// logical address.
///
/// `Observation::Unknown` when the read itself failed — a device going away
/// mid-read is not the same fact as an adapter that holds no address, and
/// collapsing the two is how a transport error becomes a verdict.
///
/// Both ioctls are pure gets. Neither puts anything on the bus.
pub async fn read_addressing(device: &AsyncDevice) -> Observation<bool> {
    let phys_ok = match device.get_physical_address().await {
        Ok(a) => PhysAddr::from_raw(u16::from(a)).is_valid(),
        Err(e) => {
            tracing::debug!("CEC_ADAP_G_PHYS_ADDR: {e}");
            return Observation::Unknown;
        }
    };
    let log_ok = match device.get_logical_addresses().await {
        Ok(addrs) => !addrs.is_empty(),
        Err(e) => {
            tracing::debug!("CEC_ADAP_G_LOG_ADDRS: {e}");
            return Observation::Unknown;
        }
    };
    Observation::Known(phys_ok && log_ok)
}

/// **Fact 2** from what the open sequence already read back, with no extra
/// ioctl.
fn addressing_from(phys: Observation<PhysAddr>, log_addrs: &[String]) -> Observation<bool> {
    match phys {
        Observation::Known(a) => Observation::Known(a.is_valid() && !log_addrs.is_empty()),
        // The read-back failed at open. Unknown, never false.
        Observation::Unknown => Observation::Unknown,
    }
}

#[async_trait::async_trait]
impl AvBackend for KernelBackend {
    /// Decide, then transmit.
    ///
    /// The decision is [`crate::action::plan`] — a pure function of the action
    /// and the two observed addresses — so every gate is covered by CI on a
    /// runner with no adapter. This method's whole job is to read the two
    /// addresses, hand them to the planner, and either put the plan on the bus
    /// or return the refusal verbatim.
    ///
    /// **A refusal happens before anything is built, so it transmits nothing.**
    /// The observation lock is taken and released before the first `.await` on
    /// the device, so an action can never block `av-state`.
    async fn act(&self, action: Action) -> ActionOutcome {
        let owner = {
            let observations = self
                .observations
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            observations.active_source()
        };
        let plan = match crate::action::plan(action, self.topology.phys_addr_read_back, owner) {
            Ok(plan) => plan,
            Err(refusal) => {
                // At info, not warn: a refusal is the gate working. The journal
                // should read as "declined, here is why", not as a fault.
                tracing::info!("{action:?} refused: {}", refusal.reason);
                return ActionOutcome::Refused(refusal.reason);
            }
        };
        tracing::info!("{action:?}: transmitting {:?}", plan.transmits);
        let transmitter = ops::DeviceTransmitter::new(self.device());
        let outcome = ops::execute(&transmitter, &self.observations, plan).await;
        self.note_tx(&outcome);
        outcome
    }

    /// Decide, then run the volume sequence.
    ///
    /// Same division as [`AvBackend::act`]: [`crate::volume::plan`] is the gate
    /// (a refusal here transmits nothing), [`crate::volume::execute`] is the
    /// sequence, and this method only supplies the wire.
    async fn volume(&self, action: VolumeAction) -> ActionOutcome {
        let plan = match crate::volume::plan(action, self.topology.phys_addr_read_back) {
            Ok(plan) => plan,
            Err(refusal) => {
                tracing::info!("volume {} refused: {}", action.as_str(), refusal.reason);
                return ActionOutcome::Refused(refusal.reason);
            }
        };
        tracing::info!("volume {}: starting", action.as_str());
        let transmitter = ops::DeviceTransmitter::new(self.device());
        let bus = ops::DeviceVolumeBus::new(&transmitter, &self.observations);
        let outcome = crate::volume::execute(&bus, plan).await;
        self.note_tx(&outcome);
        outcome
    }

    /// Ask the AVR, and fall back to what the receive loop last heard.
    ///
    /// The fallback is not a consolation prize: it is a different, honestly
    /// labelled answer. `source` says which one this is, so a caller can tell a
    /// reading taken just now from one heard at some point in the past — and
    /// when neither exists, every field is `null` rather than a plausible zero.
    async fn volume_state(&self) -> VolumeState {
        let transmitter = ops::DeviceTransmitter::new(self.device());
        let bus = ops::DeviceVolumeBus::new(&transmitter, &self.observations);
        match bus.perform(crate::volume::VolumeTx::AudioStatusQuery).await {
            Ok(crate::volume::VolumeReply::Audio(report)) => {
                VolumeState::from_report(report, crate::state::now_ms())
            }
            other => {
                if let Err(e) = other {
                    tracing::debug!("the AVR did not answer <Give Audio Status> ({e})");
                }
                let observations = self
                    .observations
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                VolumeState::from_observations(
                    observations.volume(),
                    observations.muted(),
                    observations.observed_at(),
                )
            }
        }
    }

    /// The recorded facts, with no device access at all.
    ///
    /// Deliberately does NOT probe: see [`AvBackend::health`]. The facts are
    /// refreshed by [`KernelBackend::probe`] on the watchdog interval and by the
    /// receive loop on every bus event, and every time in the reply is an age —
    /// so a stale answer reads as stale rather than as fresh.
    async fn health(&self) -> HealthReport {
        let health = match self.health.lock() {
            Ok(h) => h.clone(),
            Err(poisoned) => poisoned.into_inner().clone(),
        };
        health.report(now_ms())
    }

    async fn snapshot(&self) -> AvState {
        // The lock is taken and released inside this expression and is never
        // held across an `.await`: the rx loop must not be able to make a
        // diagnostic read of `av-state` block.
        let observations = match self.observations.lock() {
            Ok(g) => g.clone(),
            // A poisoned lock means a fold panicked. The observations are still
            // structurally valid (the fold is total), and refusing to answer
            // `av-state` at exactly the moment something went wrong is the worst
            // available option for a diagnostic verb.
            Err(poisoned) => poisoned.into_inner().clone(),
        };
        AvState::assemble(&self.topology, &observations)
    }
}

/// The capability flags actually present, by name.
///
/// Read off the bitflags rather than hardcoded, so a flag this crate has never
/// heard of still reaches `av-state` instead of being silently dropped.
fn capability_names(caps: Capabilities) -> Vec<String> {
    caps.iter_names()
        .map(|(name, _)| name.to_string())
        .collect()
}
