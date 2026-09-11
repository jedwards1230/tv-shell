//! The screen watcher: reads what is on screen and pushes the owner decision
//! into the input thread.
//!
//! # Why it is here and not in the input loop
//!
//! **The input thread must never do an X round trip.** Pad forwarding is
//! latency-sensitive and constant; an X read blocks for milliseconds and, in the
//! bad cases this core exists to survive, for a great deal longer. Putting the
//! screen read on the pad path would make controller latency a function of how
//! busy the compositor is — and would make a hung X server a dead controller.
//!
//! So the watcher runs on a thread of its own, reads the screen there, computes
//! the owner with [`super::routing`] (pure), and **pushes** the result down an
//! `mpsc` into the input runtime's `select!` loop. The input thread only ever
//! receives a decision already made.
//!
//! # Two clocks, deliberately
//!
//! * A **poll** (~[`routing::POLL`]) is the sensor of record, for V2_DESIGN
//!   §10's reason: the core publishes no event stream yet, and a listener that
//!   silently stops processing is v1's residual defect. A poll that stops has no
//!   equivalent quiet failure mode — the owner simply stops changing, which is
//!   visible in `input-state`.
//! * A **nudge** is the core saying it just wrote the base layer itself
//!   (`show`, `launch`, `home` — and the Guide escape's `home`). It recomputes
//!   at once and applies without the settle window, because those verbs verify
//!   their own switch before returning. This is what makes the escape usable:
//!   holding Guide returns the screen to the shell AND the pad follows it,
//!   rather than the shell sitting inert until a poll happens to notice.
//!
//! Observations are debounced ([`routing::Settle`]); assertions are not.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, SyncSender, TrySendError};
use std::sync::Arc;
use std::time::{Duration, Instant};

use super::routing::{self, Facts, InputOwner, Settle};
use crate::atoms::AppId;

/// Where the watcher reads "what is on screen" from.
///
/// A trait so the whole loop is testable with no X server: the rules it applies
/// are [`routing`]'s, and a rule whose only test needs a compositor is a rule
/// with no test.
pub trait ScreenSource {
    /// The base window's app id. `None` when nothing is mapped, and
    /// [`crate::boot::SCREEN_UNREADABLE`] when the read failed — the fail-closed
    /// answer [`routing::owner_of`] folds into `Unknown`.
    fn on_screen(&self) -> Option<AppId>;
}

impl ScreenSource for Arc<dyn crate::ipc::Compositor> {
    fn on_screen(&self) -> Option<AppId> {
        crate::ipc::Compositor::on_screen_app(&**self)
    }
}

/// Where a settled owner decision goes.
///
/// A trait for the same reason [`ScreenSource`] is one, and with the same shape
/// as [`super::escape::EscapeSink`]: the session's half of the contract is
/// testable against a recorder with no device.
pub trait OwnerSink {
    /// Deliver a decision. `false` means the receiver is gone.
    fn set_owner(&self, owner: InputOwner) -> bool;
    /// Is the receiver still there? Checked even when nothing changed, so a
    /// watcher outliving a dead input thread stops instead of spinning.
    fn alive(&self) -> bool;
}

/// The shell's overlay declaration, shared with the IPC surface.
///
/// An atomic rather than a message because it is **state, not an event**: the
/// question the decision asks is "does the shell currently have an overlay up",
/// and the last write is the whole answer. A queue would let a `take`/`release`
/// pair arrive out of order relative to a screen read and leave the flag
/// disagreeing with reality until the next one.
#[derive(Clone, Default)]
pub struct OverlayFlag(Arc<AtomicBool>);

impl OverlayFlag {
    pub fn new() -> OverlayFlag {
        OverlayFlag::default()
    }

    /// `input-focus take` / `release`.
    pub fn set(&self, taken: bool) {
        self.0.store(taken, Ordering::Relaxed);
    }

    pub fn get(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }
}

/// The sending half of the nudge channel: "the core just wrote the base layer".
///
/// Cloneable and inert by default, so [`crate::compositor::GamescopeCompositor`]
/// and the IPC surface can hold one whether or not an input layer was ever
/// started.
#[derive(Clone, Default)]
pub struct Nudge(Option<SyncSender<()>>);

impl Nudge {
    /// Ask the watcher to recompute now.
    ///
    /// Never blocks and never fails loudly. The queue holds ONE request because
    /// depth is not throughput here: every nudge says the same thing ("look
    /// again"), so a backlog would only re-read a screen that has not moved
    /// since the read that is already queued.
    pub fn now(&self) {
        let Some(tx) = &self.0 else { return };
        match tx.try_send(()) {
            Ok(()) | Err(TrySendError::Full(())) => {}
            Err(TrySendError::Disconnected(())) => {
                tracing::debug!("no screen watcher is listening for nudges");
            }
        }
    }
}

/// The receiving half, held by the watcher thread.
pub struct Nudges(Receiver<()>);

impl Nudges {
    /// Take a queued nudge without waiting.
    ///
    /// Public so the sites that SEND one — the escape worker, the IPC surface —
    /// can be tested for it. "The core recomputes after writing the base layer"
    /// is the rule this whole phase turns on, and a rule with no observable is a
    /// rule no test can invert.
    pub fn try_recv(&self) -> Result<(), std::sync::mpsc::TryRecvError> {
        self.0.try_recv()
    }
}

/// A nudge channel. The sender is cloned to every site that writes the base
/// layer; the receiver goes to the watcher.
pub fn nudge_channel() -> (Nudge, Nudges) {
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    (Nudge(Some(tx)), Nudges(rx))
}

/// The watcher's control surface, as the IPC layer holds it.
///
/// One handle rather than two parameters threaded side by side, because the two
/// are never useful apart: setting the overlay flag without recomputing leaves
/// the declaration inert until the next poll, and that is a drawer that opens
/// and does not take the pad for a third of a second.
///
/// [`WatchHandle::detached`] is the inert one, for a core with no input layer.
#[derive(Clone, Default)]
pub struct WatchHandle {
    overlay: OverlayFlag,
    nudge: Nudge,
}

impl WatchHandle {
    pub fn new(overlay: OverlayFlag, nudge: Nudge) -> WatchHandle {
        WatchHandle { overlay, nudge }
    }

    /// The handle a core with no screen watcher hands to IPC: every method is a
    /// no-op, and nothing downstream has to ask whether input is enabled.
    pub fn detached() -> WatchHandle {
        WatchHandle::default()
    }

    /// `input-focus take` / `release`, and recompute now.
    pub fn set_overlay(&self, taken: bool) {
        self.overlay.set(taken);
        self.nudge.now();
    }

    /// The core just wrote the base layer — `show`, `launch`, `home`.
    /// Recompute now rather than at the next poll.
    pub fn wrote_base_layer(&self) {
        self.nudge.now();
    }

    /// The sender, for the escape worker, which writes the base layer on a
    /// thread of its own and never goes through the IPC surface.
    pub fn nudge(&self) -> Nudge {
        self.nudge.clone()
    }
}

/// How long the watcher waits, in both of its modes.
///
/// A parameter rather than the constants directly so the loop's own tests run in
/// milliseconds instead of seconds. Production uses [`Timing::default`], which
/// IS [`routing::POLL`] and [`routing::SETTLE`].
#[derive(Debug, Clone, Copy)]
pub struct Timing {
    pub poll: Duration,
    pub settle: Duration,
}

impl Default for Timing {
    fn default() -> Timing {
        Timing {
            poll: routing::POLL,
            settle: routing::SETTLE,
        }
    }
}

/// Why the loop woke.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Wake {
    /// The poll interval elapsed, or the settle deadline came due. Debounced.
    Poll,
    /// The core wrote the base layer itself. Applied at once.
    Nudge,
}

/// Watch the screen until the input layer goes away.
///
/// Blocking, and meant for a thread of its own — see the module docs. Returns
/// when the sink reports its receiver is gone, or when every [`Nudge`] has been
/// dropped, both of which mean the core is shutting down.
pub fn run(
    source: &impl ScreenSource,
    sink: &impl OwnerSink,
    shell_app_id: AppId,
    overlay: &OverlayFlag,
    nudges: &Nudges,
    timing: Timing,
) {
    let mut settle = Settle::new(timing.settle, InputOwner::Unknown);
    // The first pass is an observation, not an assertion: nothing has been
    // written yet, so the very first owner still earns its settle window.
    let mut wake = Wake::Poll;
    loop {
        let now = Instant::now();
        let owner = routing::owner_of(Facts {
            on_screen: source.on_screen(),
            shell_app_id,
            shell_overlay: overlay.get(),
        });
        let applied = match wake {
            Wake::Nudge => settle.asserted(owner),
            Wake::Poll => {
                settle.observed(owner, now);
                settle.due(now)
            }
        };
        if let Some(owner) = applied {
            tracing::info!(?owner, route = ?owner.route(), "input owner changed");
            if !sink.set_owner(owner) {
                tracing::debug!("screen watcher stopping: the input layer is gone");
                return;
            }
        }

        // Checked HERE, before the wait, rather than at the top of the loop.
        // At the top it is only reached after a full poll interval has already
        // elapsed, so a watcher whose input layer is gone would sit sleeping
        // for one more interval before noticing — and a test that ends by
        // going dead would pay that interval too, which is how this loop's own
        // latency assertion came to measure the wrong thing.
        if !sink.alive() {
            tracing::debug!("screen watcher stopping: the input layer is gone");
            return;
        }

        // Sleep until the poll is due, or until a pending change settles —
        // whichever is sooner — and wake early for a nudge.
        let wait = match settle.deadline() {
            Some(at) => at
                .saturating_duration_since(Instant::now())
                .min(timing.poll),
            None => timing.poll,
        };
        wake = match nudges.0.recv_timeout(wait) {
            Ok(()) => Wake::Nudge,
            Err(RecvTimeoutError::Timeout) => Wake::Poll,
            Err(RecvTimeoutError::Disconnected) => {
                tracing::debug!("screen watcher stopping: nothing can nudge it any more");
                return;
            }
        };
    }
}

/// Start the watcher on its own thread.
pub fn spawn(
    source: Arc<dyn crate::ipc::Compositor>,
    sink: super::InputControl,
    shell_app_id: AppId,
    overlay: OverlayFlag,
    nudges: Nudges,
) -> std::io::Result<std::thread::JoinHandle<()>> {
    std::thread::Builder::new()
        .name("tv-shell-screenwatch".into())
        .spawn(move || {
            run(
                &source,
                &sink,
                shell_app_id,
                &overlay,
                &nudges,
                Timing::default(),
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::sync::Mutex;

    const SHELL: AppId = AppId::new(9001);
    const MOONLIGHT: AppId = AppId::new(9003);

    /// A screen that reads a scripted sequence, holding its last value once the
    /// script runs out — which is what a real screen does between changes.
    struct Script(Mutex<Vec<Option<AppId>>>, Mutex<Option<AppId>>);

    impl Script {
        fn new(mut frames: Vec<Option<AppId>>) -> Script {
            frames.reverse();
            Script(Mutex::new(frames), Mutex::new(None))
        }
    }

    impl ScreenSource for Script {
        fn on_screen(&self) -> Option<AppId> {
            let mut frames = self.0.lock().unwrap();
            let mut last = self.1.lock().unwrap();
            if let Some(next) = frames.pop() {
                *last = next;
            }
            *last
        }
    }

    /// A recording sink that also BOUNDS the loop.
    ///
    /// `alive` goes false once `stop_after` decisions have landed — and also
    /// after `max_passes` trips round, whatever happened. The second bound is
    /// not tidiness: a `Settle` whose window is renewed on every observation
    /// never settles, so `run` spins forever and the suite HANGS. A hang and a
    /// pass are indistinguishable to an unattended CI run, so a loop that is
    /// not converging has to end as a failed assertion instead. Measured, not
    /// imagined: that is exactly what mutation 14 did before this bound existed.
    struct Recorder {
        seen: RefCell<Vec<InputOwner>>,
        stop_after: usize,
        passes: std::cell::Cell<usize>,
        max_passes: usize,
    }

    impl Recorder {
        fn new(stop_after: usize) -> Recorder {
            Recorder {
                seen: RefCell::new(Vec::new()),
                stop_after,
                passes: std::cell::Cell::new(0),
                // Generous against the fast timings below (2 ms poll, 10 ms
                // settle): a converging loop needs a handful of passes, and a
                // non-converging one blows through this in well under a second.
                max_passes: 500,
            }
        }

        /// Assert the run ended because it finished, not because it gave up.
        fn converged(&self) {
            assert!(
                self.passes.get() < self.max_passes,
                "the watcher never settled: {} passes without reaching {} decisions",
                self.passes.get(),
                self.stop_after
            );
        }
    }

    impl OwnerSink for Recorder {
        fn set_owner(&self, owner: InputOwner) -> bool {
            self.seen.borrow_mut().push(owner);
            true
        }
        fn alive(&self) -> bool {
            self.passes.set(self.passes.get() + 1);
            self.seen.borrow().len() < self.stop_after && self.passes.get() <= self.max_passes
        }
    }

    fn fast() -> Timing {
        Timing {
            poll: Duration::from_millis(2),
            settle: Duration::from_millis(10),
        }
    }

    /// **Rule: the poll is a real sensor — a change nothing announced is found.**
    ///
    /// No nudge is ever sent here. The screen simply starts empty and then shows
    /// Moonlight, which is what an app mapping on its own looks like, and the
    /// owner has to follow. The session starts at `Unknown` too, so the empty
    /// frames are correctly a no-op rather than a delivery.
    ///
    /// **Mutation note.** Make the loop recompute only on a nudge (drop the
    /// `RecvTimeoutError::Timeout => Wake::Poll` arm) and nothing is ever
    /// delivered — the run blocks and the test times out.
    #[test]
    fn the_poll_finds_a_change_nothing_announced() {
        let script = Script::new(vec![None, None, Some(MOONLIGHT)]);
        let sink = Recorder::new(1);
        let (_nudge, nudges) = nudge_channel();
        run(&script, &sink, SHELL, &OverlayFlag::new(), &nudges, fast());
        sink.converged();
        assert_eq!(*sink.seen.borrow(), vec![InputOwner::App { id: MOONLIGHT }]);
    }

    /// **Rule: a nudge applies at once, WITHOUT the settle window.**
    ///
    /// This is the escape's path, end to end through the loop: the base-layer
    /// write happens, the nudge arrives, and the owner follows immediately.
    ///
    /// **The assertion is the LATENCY, not the outcome, and that distinction was
    /// measured rather than reasoned about.** An earlier version asserted only
    /// the recorded owner, with a settle window of 10 s to make the point — and
    /// the mutation below SURVIVED it, because the loop still applied the owner
    /// once the window finally elapsed. The test passed, ten seconds later,
    /// having asserted the exact opposite of its own name. So the window is
    /// large (5 s) and the deadline is small (1 s): the only way to finish
    /// inside the deadline is to have bypassed the debounce.
    ///
    /// **Mutation note.** Treat `Wake::Nudge` as an observation (delegate to the
    /// `Wake::Poll` arm) and the run takes the full 5 s window, failing the
    /// elapsed assertion.
    #[test]
    fn a_nudge_applies_immediately() {
        let script = Script::new(vec![Some(SHELL)]);
        let sink = Recorder::new(1);
        let (nudge, nudges) = nudge_channel();
        nudge.now();
        let started = Instant::now();
        run(
            &script,
            &sink,
            SHELL,
            &OverlayFlag::new(),
            &nudges,
            Timing {
                poll: Duration::from_secs(5),
                settle: Duration::from_secs(5),
            },
        );
        let elapsed = started.elapsed();
        assert_eq!(*sink.seen.borrow(), vec![InputOwner::Shell]);
        assert!(
            elapsed < Duration::from_secs(1),
            "a nudge must not wait out the 5 s settle window; this took {elapsed:?}"
        );
    }

    /// **Rule: the shell's overlay declaration reaches the decision.**
    ///
    /// Through the real path: the IPC verb sets [`OverlayFlag`], and the
    /// watcher reads it on the next recompute. Nothing here pokes an owner.
    ///
    /// **Mutation note.** Drop `shell_overlay` from the `Facts` the loop builds
    /// (hardcode `false`) and the recorded owner is `Shell`, not `ShellOverlay`.
    #[test]
    fn the_overlay_flag_is_read_on_every_recompute() {
        let script = Script::new(vec![Some(SHELL)]);
        let sink = Recorder::new(1);
        let overlay = OverlayFlag::new();
        overlay.set(true);
        let (nudge, nudges) = nudge_channel();
        nudge.now();
        run(&script, &sink, SHELL, &overlay, &nudges, fast());
        sink.converged();
        assert_eq!(*sink.seen.borrow(), vec![InputOwner::ShellOverlay]);
    }

    /// A watcher whose input layer has gone stops, rather than spinning on a
    /// screen nobody is listening to.
    #[test]
    fn it_stops_when_the_input_layer_is_gone() {
        struct Dead;
        impl OwnerSink for Dead {
            fn set_owner(&self, _: InputOwner) -> bool {
                false
            }
            fn alive(&self) -> bool {
                false
            }
        }
        let (_nudge, nudges) = nudge_channel();
        run(
            &Script::new(vec![Some(SHELL)]),
            &Dead,
            SHELL,
            &OverlayFlag::new(),
            &nudges,
            fast(),
        );
    }
}
