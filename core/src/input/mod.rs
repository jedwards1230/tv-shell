//! The pad fleet: discovery, exclusive grab, and the permanent per-player
//! uinput presenters (V2_DESIGN §7).
//!
//! # Scope
//!
//! This is the *foundation* of §7 and deliberately not all of it. What is here:
//! DB-match-or-reject discovery, stable per-player slots, hot join and leave,
//! `EVIOCGRAB`, presenters that exist for the life of the session, a 1:1
//! passthrough, and the read-only `input-state` verb. On its own it is
//! **behaviourally invisible**: the pad is grabbed and re-presented, and what
//! reads the presenter sees the same input it saw from the physical pad.
//!
//! Since phase 1 of `docs/V2_GAMEPAD_HANDOFF.md` there is a **second route**:
//! pad events are translated by [`keymap`] onto one permanent uinput
//! **keyboard** and reach the v2 shell through gamescope, instead of crossing
//! onto the presenters.
//!
//! Since phase 2 the choice between them is a **decision, not a setting**.
//! [`routing`] computes an [`InputOwner`] from what is on screen; [`watcher`]
//! recomputes it on a poll and immediately after the core's own base-layer
//! writes, and pushes it into the input thread over [`InputControl`]. Phase 1
//! pinned the route for the life of the session, and hardware showed what that
//! costs: a held Guide returned the screen to the shell and the home screen was
//! completely inert, because the pad was still forwarding to the app.
//! `[input].shell_keys` now **pins** the owner to the shell — the operator
//! override for a measurement, under which **no app or game receives pad input
//! at all**.
//!
//! There is also a **third path that is not a route at all**: a held Guide
//! makes the core write the base layer back to the shell itself
//! ([`escape`], jedwards1230/tv-shell#496). It is active on both routes,
//! because it is the way back from a running app, and it deliberately involves
//! the shell in nothing — v1 delivered this as a message the shell acted on and
//! it failed whenever the shell was wedged, which is the only time it matters.
//!
//! What is NOT here, each a follow-up: **masking** across a route change and the
//! safety combos (phase 3) — a transition quiesces the target it leaves, but the
//! physical release that arrives afterwards still crosses to the new one, which
//! is jedwards1230/tv-shell#295's shape; the `gamepad`/`keyboard` per-app
//! contracts (phase 4, an `[[app]]` change in `core.toml`); rumble/battery/LED;
//! and the companion touchpad/motion-node inhibition §7 calls for (SteamOS's
//! `ds-inhibit` shape).
//!
//! # Default off
//!
//! `[input].enabled` is `false` unless an operator sets it. With it off
//! [`start`] returns `None` before constructing anything, so there is no code
//! path from a disabled core to `/dev/input` or `/dev/uinput`. See
//! [`config::InputConfig`].
//!
//! # Module layout
//!
//! | Module | Owns | Testable in CI |
//! |---|---|---|
//! | [`identity`] | SDL GUID, controller DB, wire id, slot allocation | yes |
//! | [`discovery`] | The claim-or-refuse gate, and devnode ownership | yes |
//! | [`presenter`] | The canonical profile, rescaling, translation, quiesce | yes |
//! | [`keymap`] | Pad → key translation, the keyboard profile, stick repeat | yes |
//! | [`escape`] | The Guide tap/hold rule, and the hand-off that writes the base layer | yes |
//! | [`routing`] | The owner truth table, the transition plan, the settle debounce | yes |
//! | [`fleet`] | Membership, slot stability, the join/leave plan | yes |
//! | [`session`] | The lifecycle: create once, claim, forward, retire, re-route | yes (recording double) |
//! | [`backend`] | The hardware seam | n/a (a trait) |
//! | [`watcher`] | The screen poll, the nudge, and the push into the input thread | yes (its loop; its X source is a trait) |
//! | `evdev_backend` | evdev/uinput syscalls | **no** — needs a seat |
//! | `runtime` | The poll/read loop and the thread it runs on | **no** — needs a backend |

pub mod backend;
pub mod config;
pub mod discovery;
pub mod escape;
pub mod fleet;
pub mod identity;
pub mod keymap;
pub mod presenter;
pub mod routing;
pub mod session;
pub mod watcher;

/// Public so `core/tests/input_uinput.rs` can drive it against a real kernel.
/// That file is the ONLY place the hardware claims are checked, so the module
/// has to be reachable from outside the crate.
#[cfg(target_os = "linux")]
pub mod evdev_backend;
#[cfg(target_os = "linux")]
mod runtime;

pub use config::{InputConfig, ResolvedInput};
pub use escape::EscapeSink;
pub use routing::{InputOwner, Route};
pub use session::InputReport;

use tokio::sync::watch;

/// A handle on the running input layer, held by the IPC surface.
///
/// Reads are snapshots off a `watch` channel rather than a request/reply round
/// trip. That is deliberate: `input-state` is a diagnostic an operator reaches
/// for when something is wrong, so it must answer even if the input loop is
/// wedged. A round trip would hang exactly when it was most needed; a snapshot
/// answers, and the report carries [`InputReport::last_poll_unix_ms`] so a
/// stopped loop is visible as a timestamp that stops advancing rather than as a
/// plausible-looking but stale answer.
///
/// Only `runtime` (Linux) ever constructs one, so on any other host every field
/// here is written by nothing — hence the `allow`. The type itself stays
/// unconditional so `start`'s signature does not change per platform, which is
/// what lets the pure modules and their tests build and run anywhere.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub struct InputHandle {
    reports: watch::Receiver<InputReport>,
    control: InputControl,
    /// Dropped on shutdown; the runtime's loop selects on its closure.
    shutdown: Option<tokio::sync::oneshot::Sender<()>>,
    join: Option<std::thread::JoinHandle<()>>,
}

impl InputHandle {
    /// A cloneable read-only view, for the IPC surface.
    pub fn reports(&self) -> InputReports {
        InputReports(Some(self.reports.clone()))
    }

    /// A cloneable WRITE path into the input thread, for the screen watcher.
    ///
    /// The counterpart of [`Self::reports`], and the reason phase 2 needed a
    /// second channel at all: `watch` is read-only, so the owner decision had
    /// nowhere to go. See [`InputControl`].
    pub fn control(&self) -> InputControl {
        self.control.clone()
    }

    /// The most recent report.
    pub fn report(&self) -> InputReport {
        self.reports.borrow().clone()
    }

    /// Stop the input loop and wait for it to release every pad.
    ///
    /// Best-effort and bounded by the loop noticing: a core that is killed
    /// instead releases everything anyway, because a grab lives on a file
    /// descriptor the kernel closes (see `evdev_backend`).
    pub fn shutdown(&mut self) {
        drop(self.shutdown.take());
        if let Some(join) = self.join.take() {
            if join.join().is_err() {
                tracing::warn!("the input runtime thread panicked on shutdown");
            }
        }
    }
}

/// The read side of the input layer, held by the IPC surface.
///
/// `None` inside means the layer is disabled or failed to start, and
/// [`InputReports::report`] then answers with [`InputReport::disabled`] — an
/// honest empty report rather than an error, because "input is off" is a state
/// the verb exists to report, not a failure to answer.
#[derive(Clone)]
pub struct InputReports(Option<watch::Receiver<InputReport>>);

impl InputReports {
    /// The view a core with no input layer hands to IPC.
    pub fn disabled() -> InputReports {
        InputReports(None)
    }

    /// The most recent report, or the disabled one.
    pub fn report(&self) -> InputReport {
        match &self.0 {
            Some(rx) => rx.borrow().clone(),
            None => InputReport::disabled(),
        }
    }
}

/// A message into the input thread.
///
/// Deliberately tiny, and deliberately one-way. Nothing here asks the input
/// thread a question: a request/reply channel would let the compositor half
/// block on the pad path, which is the one thing this split exists to prevent.
#[derive(Debug, Clone, Copy)]
pub enum Control {
    /// Route the pad according to this owner. Computed by
    /// [`watcher`] from a screen read that happened on another thread.
    SetOwner(InputOwner),
}

/// The write side of the input layer, held by the screen watcher.
///
/// `None` inside means the layer is disabled or failed to start; every send is
/// then a no-op and [`InputControl::is_closed`] answers `true`, so a watcher
/// handed a disabled control stops instead of computing decisions for nobody.
#[derive(Clone)]
pub struct InputControl(Option<tokio::sync::mpsc::UnboundedSender<Control>>);

impl InputControl {
    /// The control a core with no input layer hands out.
    pub fn disabled() -> InputControl {
        InputControl(None)
    }

    /// Has the input thread gone?
    pub fn is_closed(&self) -> bool {
        match &self.0 {
            Some(tx) => tx.is_closed(),
            None => true,
        }
    }
}

impl watcher::OwnerSink for InputControl {
    fn set_owner(&self, owner: InputOwner) -> bool {
        match &self.0 {
            Some(tx) => tx.send(Control::SetOwner(owner)).is_ok(),
            None => false,
        }
    }

    fn alive(&self) -> bool {
        !self.is_closed()
    }
}

/// What [`start`] should do, decided **without doing any of it**.
///
/// The gate is a value rather than an early `return` inside `start` for one
/// reason: `start` spawns a thread that opens `/dev/uinput`, so it cannot run in
/// a test, and a safety rule whose only expression is inside an untestable
/// function is undefended. This is testable everywhere, including on a host with
/// no seat.
#[derive(Debug)]
pub enum StartDecision {
    /// `[input].enabled` is off. Nothing is enumerated, opened or created —
    /// and nothing has been *read*, either (see the module docs).
    Disabled,
    /// Enabled, and the config resolved.
    Start(ResolvedInput),
    /// Enabled, but the config could not be resolved. Named so the operator
    /// learns which key was wrong.
    Misconfigured(String),
}

/// Decide whether the input layer runs, and settle its config if it does.
///
/// **The `enabled` check comes first and short-circuits everything.** Not as an
/// optimisation: `resolve` reads a file from disk, so a gate placed after it
/// would do observable work on behalf of a layer that is switched off. That
/// ordering is what
/// [`a_disabled_config_does_no_work_at_all`](self#tests) pins, using the file
/// read as the probe — a disabled config pointing at an unreadable database must
/// come back `Disabled`, not `Misconfigured`.
pub fn decide(config: &InputConfig) -> StartDecision {
    if !config.enabled {
        return StartDecision::Disabled;
    }
    match config.resolve() {
        Ok(resolved) => StartDecision::Start(resolved),
        Err(e) => StartDecision::Misconfigured(e.to_string()),
    }
}

/// Start the input layer if — and only if — it is enabled.
///
/// Returns `None` when `[input].enabled` is false, **before** anything is
/// enumerated, opened or created. That is the whole safety contract: a core
/// running with the default config is byte-identical, at the device layer, to
/// one built without this module.
///
/// An input layer that fails to start is logged and returns `None` rather than
/// taking the core down: the compositor half — the base layer, launching, the
/// escape hatches — is what keeps a television showing something, and it must
/// not be hostage to `/dev/uinput` permissions.
///
/// `escape` builds the sink a held Guide fires into. A **thunk**, not a value,
/// so the gate below still short-circuits everything: with the flag off the
/// escape worker is never constructed either, and the promise stays "a disabled
/// core does no work at all" rather than "no *device* work". It is a parameter
/// because what performs the base-layer write is the compositor half of the
/// core, which this module must not know about — and because a session's escape
/// has to be substitutable for the recording double its tests use.
#[cfg(target_os = "linux")]
pub fn start(
    config: &InputConfig,
    escape: impl FnOnce() -> anyhow::Result<Box<dyn EscapeSink + Send>>,
) -> Option<InputHandle> {
    let resolved = match decide(config) {
        StartDecision::Disabled => {
            tracing::info!("[input] is disabled; no input device will be enumerated or opened");
            return None;
        }
        StartDecision::Misconfigured(e) => {
            tracing::error!("input layer not started: {e}");
            return None;
        }
        StartDecision::Start(resolved) => resolved,
    };
    let escape = match escape() {
        Ok(sink) => sink,
        Err(e) => {
            // The layer does not start rather than starting with no way back to
            // the shell: a grabbed pad and no escape is exactly the trap
            // jedwards1230/tv-shell#496 exists to remove.
            tracing::error!("input layer not started: building the escape sink: {e:#}");
            return None;
        }
    };
    match runtime::spawn(resolved, escape) {
        Ok(handle) => Some(handle),
        Err(e) => {
            tracing::error!("input layer not started: {e}");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A config that is disabled but would fail loudly if anything acted on it.
    fn disabled_but_would_fail() -> InputConfig {
        InputConfig {
            enabled: false,
            // Resolving this is a file read that ERRORS. It is the probe: if the
            // gate ran after `resolve`, this config could not come back
            // `Disabled`.
            controller_db: "/nonexistent/gamecontrollerdb.txt".into(),
            ..InputConfig::default()
        }
    }

    /// **Rule: with the flag off, the layer does no work at all — not even a
    /// file read.**
    ///
    /// The safety property the whole PR rests on, and the one that is easiest to
    /// test vacuously. "`start` returns `None`" would pass against a gate placed
    /// anywhere, including after the work; so would asserting the flag parses as
    /// `false`. Neither says the layer was inert.
    ///
    /// This uses an **observable side effect as the probe**. `resolve` reads
    /// `controller_db` from disk, and the path here does not exist, so the same
    /// config differs by outcome depending on whether that read happened:
    /// `Disabled` if the gate short-circuited, `Misconfigured` if it did not.
    /// The second half proves the probe is live — that the file really would
    /// have been read — so the first half cannot pass because the path was
    /// harmless.
    #[test]
    fn a_disabled_config_does_no_work_at_all() {
        let off = disabled_but_would_fail();
        assert!(
            matches!(decide(&off), StartDecision::Disabled),
            "a disabled layer must not even resolve its config"
        );

        // The probe is live: flip ONLY `enabled`, and the very same config now
        // reaches the file and fails on it.
        let on = InputConfig {
            enabled: true,
            ..disabled_but_would_fail()
        };
        match decide(&on) {
            StartDecision::Misconfigured(e) => assert!(e.contains("controller_db"), "{e}"),
            other => panic!("the probe is dead — enabling changed nothing: {other:?}"),
        }
    }

    /// The default config — what a box nobody reconfigured runs — is `Disabled`.
    #[test]
    fn the_default_config_starts_nothing() {
        assert!(matches!(
            decide(&InputConfig::default()),
            StartDecision::Disabled
        ));
    }

    /// An enabled, valid config resolves and carries its settings through, so
    /// the gate is not simply refusing everything.
    #[test]
    fn an_enabled_config_resolves_and_carries_its_settings() {
        let config = InputConfig {
            enabled: true,
            players: 2,
            ..InputConfig::default()
        };
        match decide(&config) {
            StartDecision::Start(resolved) => {
                assert_eq!(resolved.players, 2);
                assert!(
                    resolved.db.is_known(0x045e, 0x028e),
                    "the baseline db loaded"
                );
            }
            other => panic!("an enabled valid config must start: {other:?}"),
        }
    }
}

/// Non-Linux hosts have no evdev or uinput, so there is nothing to start.
///
/// The crate still compiles and its rules are still tested there — the point of
/// keeping every decision out of the backend.
#[cfg(not(target_os = "linux"))]
pub fn start(
    config: &InputConfig,
    _escape: impl FnOnce() -> anyhow::Result<Box<dyn EscapeSink + Send>>,
) -> Option<InputHandle> {
    if config.enabled {
        tracing::warn!("[input].enabled is set, but evdev and uinput are Linux-only");
    }
    None
}
