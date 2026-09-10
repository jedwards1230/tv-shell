//! The Guide escape: a held Guide button returns the screen to the shell, and
//! **the core performs that base-layer write itself**.
//!
//! # Why the core, and not the shell
//!
//! v1 delivered this as `intent home-hold` — a message *to the shell*, which
//! then acted. That path failed in exactly the situation it existed for: when
//! the shell was wedged, so was the escape (`docs/V2_GAMEPAD_HANDOFF.md` §5,
//! "Shell-delivered escape — **Drop**"). Here the pad hold is routed into
//! [`crate::baselayer::home`], which is one `GAMESCOPECTRL_BASELAYER_APPID`
//! write plus one bounded verify. Nothing on the path touches the shell, so the
//! shell being dead, hung or never started changes nothing about whether the
//! escape works.
//!
//! It rests on two measurements, both in §2: **M1** settled option A, *grab
//! always*, so the core still sees the pad once an app is on screen; and **M3**
//! measured `EVIOCGRAB` genuinely taking the pad from a running app (4015
//! events in 12 s that Moonlight did not get). Without either, a held Guide over
//! a running game would never reach this code.
//!
//! # A hold, not a tap
//!
//! Ported from v1 (`daemon/src/input/pad.rs::handle_meta`), including the
//! constant: [`DEFAULT_HOLD_MS`] is v1's `DEFAULT_META_HOLD_MS`, tuned on this
//! hardware. The Guide button is **buffered** on press — never forwarded live,
//! so there is no window in which a partial press leaks while we discriminate —
//! and the release decides:
//!
//! * **released before the threshold** → a TAP, delivered per route (the app
//!   sees a real Guide press+release; the shell sees its drawer key);
//! * **held past it** → the escape fires and the release is swallowed entirely.
//!
//! # Two seams, because they fail differently
//!
//! [`GuideWatch`] is pure and takes `now: Instant` from its caller — no clock is
//! read inside routing code, so "did this hold fire" is decided deterministically
//! in CI. [`EscapeSink`] is the hand-off to whatever performs the write; the
//! session counts what it returns, so a fired escape that could not write the
//! base layer is a number in `input-state` rather than a silence.
//!
//! The production sink deliberately does **not** write on the input thread. A
//! `home` whose target has no mapped window waits the *map* bound (tens of
//! seconds) — and "the shell has no mapped window" is precisely the case this
//! escape exists for. Blocking the pad loop for that long would trade a wedged
//! shell for a dead controller. So [`ChannelEscape`] hands the request to
//! [`run_worker`] on a thread of its own and returns immediately.

use std::sync::mpsc::{Receiver, SyncSender, TrySendError};
use std::time::{Duration, Instant};

use super::watcher::Nudge;

/// v1's `DEFAULT_META_HOLD_MS`, ported unchanged.
///
/// 500 ms is a comfortable press-and-hold that a deliberate long-press clears
/// and a quick tap does not. Ported rather than re-picked: it was tuned on this
/// hardware, with this controller, by using it.
pub const DEFAULT_HOLD_MS: u64 = 500;

/// Bounds on the configured hold, so a typo cannot make the escape unreachable
/// (an hour-long hold) or indistinguishable from a tap (zero).
pub const MIN_HOLD_MS: u64 = 100;
pub const MAX_HOLD_MS: u64 = 5_000;

/// What a Guide RELEASE means, once the press it ends is known.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Release {
    /// Released before the threshold: deliver the buffered Guide per route.
    Tap,
    /// The hold already fired. Swallow it — the app must not see the release of
    /// a button it never saw pressed.
    Swallow,
    /// A release for a press this pad never showed us (it joined mid-hold, or a
    /// `SYN_DROPPED` ate the press). Nothing to deliver, and emphatically not a
    /// tap: replaying one would be a Guide press the user did not make.
    Ignore,
}

/// One pad's Guide state.
///
/// **Per pad, which is the whole of the per-pad-complete rule here.** v1 keeps
/// its hold timer on the pad and re-checks *that pad's* held set at fire time,
/// so two pads each holding half a chord never complete it between them
/// (`docs/V2_GAMEPAD_HANDOFF.md` §5). With a single-button escape that
/// degenerates to a sharper statement: the pad that armed the hold is the pad
/// that must still be holding when it fires, and one pad's release can never
/// satisfy — or cancel — another's.
///
/// Pure: every method takes the clock from its caller.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct GuideWatch {
    /// When THIS pad's Guide went down, while it is down.
    pressed_at: Option<Instant>,
    /// This pad's hold has fired; its release is owed a swallow.
    fired: bool,
}

impl GuideWatch {
    /// Guide went down on this pad.
    pub fn press(&mut self, now: Instant) {
        self.pressed_at = Some(now);
        self.fired = false;
    }

    /// Guide came up on this pad.
    pub fn release(&mut self) -> Release {
        let was_down = self.pressed_at.take().is_some();
        let fired = std::mem::take(&mut self.fired);
        match (was_down, fired) {
            (true, false) => Release::Tap,
            (true, true) => Release::Swallow,
            (false, _) => Release::Ignore,
        }
    }

    /// When this pad's hold is due, if it is armed and has not fired.
    pub fn deadline(&self, hold: Duration) -> Option<Instant> {
        match (self.pressed_at, self.fired) {
            (Some(at), false) => Some(at + hold),
            _ => None,
        }
    }

    /// Fire this pad's hold if it is due at `now`.
    ///
    /// **Fires at most once per press.** The second call for the same press
    /// returns `false` because `fired` latches, so a timer that runs again — a
    /// repeat tick, a re-entered loop — cannot escape twice off one hold.
    pub fn due(&mut self, now: Instant, hold: Duration) -> bool {
        match self.deadline(hold) {
            Some(at) if now >= at => {
                self.fired = true;
                true
            }
            _ => false,
        }
    }

    /// Is this pad holding Guide right now (fired or not)?
    ///
    /// What the fleet latch clears on, and deliberately not [`Self::armed`]:
    /// this tracks the physical button, so the latch's lifetime is the gesture's
    /// rather than the timer's. At today's call sites — a release, and a retire
    /// — the two are equivalent, because a pad that has fired cannot fire again
    /// without a fresh press. Measured, not assumed: swapping this for `armed`
    /// in `clear_escape_latch` is a mutation that **survives** the suite, and
    /// the reason it survives is that equivalence rather than a missing test.
    pub fn holding(&self) -> bool {
        self.pressed_at.is_some()
    }

    /// Is this pad armed — Guide down, threshold not yet reached?
    ///
    /// Reported as `escape.armed` so a hardware session reads that the core saw
    /// the press, rather than inferring it from whether the escape happened.
    pub fn armed(&self) -> bool {
        self.pressed_at.is_some() && !self.fired
    }
}

/// Why an escape could not be handed off.
#[derive(Debug, thiserror::Error)]
#[error("the escape could not be delivered: {0}")]
pub struct EscapeError(pub String);

/// The seam between "a Guide hold fired" and "the base layer was written".
///
/// A trait so the session's rules are testable without a compositor, and so the
/// **failure** is testable at all: a fired escape that could not write must show
/// up in `input-state`, and a rule whose only expression is inside a real X
/// round trip is a rule no test can invert.
pub trait EscapeSink {
    /// Return the screen to the shell. Called from the input thread, so an
    /// implementation must not block on anything unbounded.
    fn fire(&mut self) -> Result<(), EscapeError>;
}

/// Whatever performs the actual base-layer write.
///
/// Narrow on purpose: the worker below needs one operation, and a trait with
/// one method is one a test can implement in three lines. `Arc<dyn Compositor>`
/// satisfies it, which is how the real core wires this to
/// [`crate::baselayer::home`] with its verify intact.
pub trait HomeWriter: Send {
    /// The core's `home`: one base-layer write plus one bounded verify.
    /// Returns the IPC reply — `ok`, or a line beginning `error:`.
    fn home(&self) -> String;
}

impl HomeWriter for std::sync::Arc<dyn crate::ipc::Compositor> {
    fn home(&self) -> String {
        crate::ipc::Compositor::home(&**self)
    }
}

/// The production sink: hand the escape to [`run_worker`] and return.
pub struct ChannelEscape {
    tx: SyncSender<()>,
}

impl EscapeSink for ChannelEscape {
    fn fire(&mut self) -> Result<(), EscapeError> {
        match self.tx.try_send(()) {
            Ok(()) => Ok(()),
            // An escape is already queued. Coalescing is correct rather than
            // merely convenient: two `home` writes back to back say the same
            // thing, and the second would serialise behind the first's verify
            // for no gain.
            Err(TrySendError::Full(())) => {
                tracing::debug!("an escape is already in flight; coalescing this one");
                Ok(())
            }
            Err(TrySendError::Disconnected(())) => Err(EscapeError(
                "the escape worker is gone, so no base-layer write can happen".into(),
            )),
        }
    }
}

/// Start the escape worker and return the sink that feeds it.
///
/// The queue holds ONE request. Depth is not throughput here: every request is
/// identical ("put the shell on screen"), so a backlog would only replay a
/// switch that already happened.
pub fn spawn(writer: impl HomeWriter + 'static, nudge: Nudge) -> std::io::Result<ChannelEscape> {
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    std::thread::Builder::new()
        .name("tv-shell-escape".into())
        .spawn(move || run_worker(&rx, &writer, &nudge))?;
    Ok(ChannelEscape { tx })
}

/// Serve escape requests until the sink is dropped.
///
/// Separate from [`spawn`] so the "a fired escape writes the base layer" rule is
/// testable against a recording [`HomeWriter`] with no thread, no compositor and
/// no X server.
///
/// # The nudge is not optional
///
/// Every write is followed by a [`Nudge`], which is what makes the escape flip
/// **routing** and not merely what is on screen. Without it, holding Guide over
/// an app returns you to the shell and leaves the pad forwarding to the app —
/// which is precisely what was measured on hardware before this phase: the home
/// screen came back and was completely inert. The nudge is sent here, at the
/// site of the write, rather than left to the watcher's poll to notice, because
/// a third of a second of dead controller after asking to get out of an app is
/// the symptom, not an acceptable margin.
///
/// It is sent whether the write succeeded or not, deliberately. A failed `home`
/// may still have moved the screen (the write can land and the verify time out),
/// and a recompute costs one X read; refusing to look after a failure is how a
/// core ends up confidently routing to a shell that is not there.
pub fn run_worker(rx: &Receiver<()>, writer: &impl HomeWriter, nudge: &Nudge) {
    while rx.recv().is_ok() {
        let reply = writer.home();
        if reply.starts_with("error") {
            // Loud, and then keep serving: the next press must still be able to
            // try. A worker that exited on the first failure would make one bad
            // moment permanent.
            tracing::error!("the Guide escape could not return to the shell: {reply}");
        } else {
            tracing::info!("Guide escape: returned to the shell ({reply})");
        }
        nudge.now();
    }
    tracing::debug!("escape worker stopping; its sink was dropped");
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    const HOLD: Duration = Duration::from_millis(DEFAULT_HOLD_MS);

    fn t0() -> Instant {
        Instant::now()
    }

    /// **Rule: a TAP never fires the escape.**
    ///
    /// v1 lets a Guide tap through to the game, and a tap that returned the box
    /// to the shell would make every accidental brush of the button a game
    /// interruption.
    ///
    /// **Mutation note.** Make `due` fire at `pressed_at` rather than
    /// `pressed_at + hold` (a zero threshold), or have `release` report
    /// `Swallow` unconditionally, and this fails.
    #[test]
    fn a_tap_is_a_tap_and_fires_nothing() {
        let t = t0();
        let mut w = GuideWatch::default();
        w.press(t);
        // At the press, and one millisecond short of the threshold.
        assert!(!w.due(t, HOLD));
        assert!(!w.due(t + HOLD - Duration::from_millis(1), HOLD));
        assert_eq!(w.release(), Release::Tap);
        // And releasing disarmed it: a later tick cannot fire a press that ended.
        assert!(!w.due(t + Duration::from_secs(9), HOLD));
        assert_eq!(w.deadline(HOLD), None);
    }

    /// **Rule: a HOLD fires exactly once, and its release is swallowed.**
    ///
    /// The swallow is the leak rule: the app never saw the press (it is
    /// buffered), so it must not see the release either.
    ///
    /// **Mutation note.** Drop the `fired` latch in `due` and the "exactly once"
    /// assertion fails; make `release` return `Tap` after a fire and the swallow
    /// assertion does.
    #[test]
    fn a_hold_fires_once_and_swallows_its_release() {
        let t = t0();
        let mut w = GuideWatch::default();
        w.press(t);
        assert_eq!(w.deadline(HOLD), Some(t + HOLD));
        assert!(w.due(t + HOLD, HOLD), "due at exactly the threshold");
        assert!(
            !w.due(t + HOLD + Duration::from_secs(1), HOLD),
            "a second tick on the same hold must not escape again"
        );
        assert_eq!(w.deadline(HOLD), None, "a fired hold is no longer armed");
        assert_eq!(w.release(), Release::Swallow);
    }

    /// **Rule: a release we never saw the press for delivers nothing.**
    ///
    /// Reachable for real: a pad claimed while the user is already holding Guide
    /// sends only the release. Replaying a tap there would be a Guide press
    /// nobody made.
    ///
    /// **Mutation note.** Make `release` return `Tap` when `pressed_at` is
    /// `None` and this fails.
    #[test]
    fn a_release_with_no_press_is_ignored() {
        let mut w = GuideWatch::default();
        assert_eq!(w.release(), Release::Ignore);
    }

    /// **Rule: `armed` is true only between the press and the fire.**
    ///
    /// It is what `input-state` reports, so it must distinguish "the core saw
    /// your press" from "the core acted on it".
    #[test]
    fn armed_spans_the_press_and_ends_at_the_fire() {
        let t = t0();
        let mut w = GuideWatch::default();
        assert!(!w.armed());
        w.press(t);
        assert!(w.armed() && w.holding());
        assert!(w.due(t + HOLD, HOLD));
        assert!(!w.armed(), "it fired, so it is no longer waiting to");
        assert!(w.holding(), "but the button is still physically down");
        w.release();
        assert!(!w.holding());
    }

    /// A recording [`HomeWriter`], and a switch to make it fail.
    #[derive(Default)]
    struct RecordingHome {
        calls: RefCell<usize>,
        fail: bool,
    }

    impl HomeWriter for RecordingHome {
        fn home(&self) -> String {
            *self.calls.borrow_mut() += 1;
            if self.fail {
                "error: base layer did not take".into()
            } else {
                "ok".into()
            }
        }
    }

    /// **Rule: a request becomes a base-layer write, and nothing else does.**
    ///
    /// The worker's whole job. Two requests, two `home` calls — no shell, no
    /// IPC, nothing a dead shell could stop.
    ///
    /// **Mutation note.** Make `run_worker` drain without calling `home` and
    /// this fails on the count.
    #[test]
    fn every_request_performs_one_home_write() {
        let (tx, rx) = std::sync::mpsc::sync_channel(4);
        tx.send(()).unwrap();
        tx.send(()).unwrap();
        drop(tx);
        let writer = RecordingHome::default();
        let (nudge, _nudges) = super::super::watcher::nudge_channel();
        run_worker(&rx, &writer, &nudge);
        assert_eq!(*writer.calls.borrow(), 2);
    }

    /// **Rule: every escape write NUDGES the screen watcher.**
    ///
    /// This is what makes a held Guide flip **routing** and not merely what is
    /// on screen. Measured on hardware before phase 2: the escape returned the
    /// screen to the shell and the home screen was completely inert, because the
    /// pad was still forwarding to the app. The write and the recompute have to
    /// be the same act.
    ///
    /// **Mutation note — the "escape does not trigger a recompute" mutation.**
    /// Delete the `nudge.now()` from `run_worker` and this fails: the channel is
    /// empty. The first assertion proves the probe is live rather than the
    /// channel being pre-filled by construction.
    #[test]
    fn every_escape_write_asks_the_watcher_to_recompute() {
        let (tx, rx) = std::sync::mpsc::sync_channel(4);
        let (nudge, nudges) = super::super::watcher::nudge_channel();
        assert!(
            nudges.try_recv().is_err(),
            "the probe is live: nothing has nudged yet"
        );
        tx.send(()).unwrap();
        drop(tx);
        let writer = RecordingHome::default();
        run_worker(&rx, &writer, &nudge);
        assert_eq!(*writer.calls.borrow(), 1);
        assert!(nudges.try_recv().is_ok(), "the write asked for a recompute");
    }

    /// **Rule: a FAILED write still nudges.**
    ///
    /// A `home` can land and still fail its verify, so the screen may have moved
    /// even though the reply says `error:`. Not looking after a failure is how a
    /// core ends up confidently routing to a shell that is not there.
    ///
    /// **Mutation note.** Move `nudge.now()` inside the success branch and this
    /// fails.
    #[test]
    fn a_failed_write_still_asks_the_watcher_to_recompute() {
        let (tx, rx) = std::sync::mpsc::sync_channel(4);
        let (nudge, nudges) = super::super::watcher::nudge_channel();
        tx.send(()).unwrap();
        drop(tx);
        let writer = RecordingHome {
            fail: true,
            ..RecordingHome::default()
        };
        run_worker(&rx, &writer, &nudge);
        assert!(nudges.try_recv().is_ok());
    }

    /// **Rule: a failed write does not stop the worker.**
    ///
    /// One unwritable moment must not make the escape permanently dead — the
    /// next press has to be able to try.
    ///
    /// **Mutation note.** `break` on an `error:` reply and the second call
    /// disappears.
    #[test]
    fn a_failed_write_leaves_the_worker_serving() {
        let (tx, rx) = std::sync::mpsc::sync_channel(4);
        tx.send(()).unwrap();
        tx.send(()).unwrap();
        drop(tx);
        let writer = RecordingHome {
            fail: true,
            ..RecordingHome::default()
        };
        let (nudge, _nudges) = super::super::watcher::nudge_channel();
        run_worker(&rx, &writer, &nudge);
        assert_eq!(*writer.calls.borrow(), 2);
    }

    /// **Rule: the sink reports a worker that is gone, rather than swallowing.**
    ///
    /// A sink that returned `Ok` with nothing behind it would hold
    /// `escape.failures` at zero while the escape was silently dead.
    #[test]
    fn a_dead_worker_is_an_error_not_a_silence() {
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        let mut sink = ChannelEscape { tx };
        drop(rx);
        assert!(sink.fire().is_err());
    }

    /// A second fire while one is queued coalesces rather than failing: both
    /// requests say the same thing.
    #[test]
    fn a_queued_escape_coalesces() {
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        let mut sink = ChannelEscape { tx };
        assert!(sink.fire().is_ok());
        assert!(sink.fire().is_ok(), "the second coalesces into the first");
        assert!(rx.try_recv().is_ok());
        assert!(rx.try_recv().is_err(), "and only one request was queued");
    }
}
