//! Who owns the pad, decided from what is on screen — pure, no syscalls.
//!
//! Phase 2 of `docs/V2_GAMEPAD_HANDOFF.md` §3. Phase 1 pinned the route for the
//! life of the session, and using it on hardware showed exactly what that costs:
//! with `shell_keys = false` a held Guide correctly returned the screen to the
//! shell (jedwards1230/tv-shell#498) and **the home screen was then completely
//! inert**, because the pad was still being forwarded to the app's presenter.
//! You could have a drivable shell or a working app, never both.
//!
//! So the route stops being a setting and becomes a decision: [`owner_of`] reads
//! the base window's app id, and [`plan`] says what a change from one owner to
//! the next must *do*. Both are values, computed here and executed elsewhere —
//! the module states rules and performs none of them, which is what lets the
//! whole truth table be checked in CI with no compositor, no seat and no device.
//!
//! # The safe default, and it is not negotiable
//!
//! **[`InputOwner::Unknown`] routes to the APP, never to the shell.** Trapping
//! the pad in an invisible shell is the worse failure by a wide margin: the user
//! sees a game and a controller that does nothing, with no way to tell whether
//! the box is wedged. Routing to the app when we are unsure costs, at worst, a
//! shell that ignores the pad while something is visibly on screen — a state the
//! Guide escape ([`super::escape`]) always gets you out of, because that escape
//! is a base-layer write the core performs itself and is active on both routes.
//!
//! An unreadable screen is folded into the same answer: [`crate::compositor`]
//! fails closed by reporting [`crate::boot::SCREEN_UNREADABLE`] rather than
//! `None`, and [`owner_of`] maps that sentinel to `Unknown`. A failed X read
//! must never look like "the shell is up".
//!
//! # Debounce, ported rather than re-picked
//!
//! [`SETTLE`] is v1's `FOCUS_SETTLE_MS` (`daemon/src/input/mod.rs`), 300 ms,
//! tuned on this hardware: a launch flaps focus several times over a fraction of
//! a second, and applying every flap would switch the route back and forth
//! underneath a user who pressed one button. [`Settle`] collapses a flap into
//! **one net transition**.
//!
//! A core-initiated write — `show`, `launch`, `home`, and the Guide escape's
//! `home` — is **not** debounced. It goes through [`Settle::asserted`] and
//! applies at once. That is v1's split too ("only follow-focus is debounced;
//! explicit IPC applies instantly"), and here it is what makes the escape usable:
//! the write that returns the screen to the shell must flip routing *with* it,
//! not 300 ms later after a poll happens to notice.

use std::time::{Duration, Instant};

use crate::atoms::AppId;

/// v1's `FOCUS_SETTLE_MS`, ported unchanged. See the module docs.
pub const SETTLE: Duration = Duration::from_millis(300);

/// How often the watcher re-reads the screen when nothing nudged it.
///
/// A poll rather than an event subscription for V2_DESIGN §10's reason: the core
/// publishes no event stream yet, and a listener that silently stops processing
/// is v1's residual defect. 250 ms is fast enough that a change nothing announced
/// is corrected before anyone reaches for a second button, and slow enough that
/// it is four X round trips a second on an idle box.
pub const POLL: Duration = Duration::from_millis(250);

/// Who the core believes owns the pad right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "kebab-case", tag = "kind")]
pub enum InputOwner {
    /// The v2 shell owns the screen and the pad drives it as keys.
    Shell,
    /// The shell owns the screen AND has declared an input-taking overlay (a
    /// drawer, the QAM) via `input-focus take`.
    ///
    /// It routes exactly as [`InputOwner::Shell`] does — the difference is not
    /// where events go but that *entering* it is a transition, so the keyboard
    /// is quiesced and a button held while the drawer opens cannot instantly
    /// activate the item under the cursor.
    ShellOverlay,
    /// An app owns the screen; the pad reaches it through its presenter.
    App { id: AppId },
    /// Not known — nothing is mapped, or the screen could not be read.
    /// **Routes to the app.** See the module docs.
    Unknown,
}

/// Where pad events actually go.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Route {
    /// Translated to keys on the core's uinput keyboard, which gamescope routes
    /// to the focused client.
    Shell,
    /// Forwarded 1:1 onto the player's presenter.
    App,
}

impl InputOwner {
    /// The route this owner implies.
    ///
    /// Total, and deliberately so: there is no owner for which "where do events
    /// go" is undecided, because an undecided route is a dead controller.
    pub fn route(self) -> Route {
        match self {
            InputOwner::Shell | InputOwner::ShellOverlay => Route::Shell,
            // The safe default lives HERE, in one arm, so it cannot be
            // reintroduced differently at a second site.
            InputOwner::App { .. } | InputOwner::Unknown => Route::App,
        }
    }
}

/// The facts an owner decision is made from.
///
/// A struct rather than three positional arguments because two of the three are
/// `AppId`-shaped: `owner_of(on_screen, shell)` and `owner_of(shell, on_screen)`
/// both compile, and swapping them inverts the entire decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Facts {
    /// The base window's app id, as [`crate::screen::ScreenState::on_screen_app`]
    /// reports it — `None` when nothing is mapped, and
    /// [`crate::boot::SCREEN_UNREADABLE`] when the read failed.
    pub on_screen: Option<AppId>,
    /// `[session] shell_app_id` from `core.toml`.
    pub shell_app_id: AppId,
    /// The shell has declared an input-taking overlay (`input-focus take`).
    ///
    /// **A declaration of the shell's own state, never a command about
    /// routing.** The core folds it into a decision it makes itself, so a shell
    /// that dies without sending `release` self-heals the moment the watcher
    /// sees something else on screen — which is v1's `set_overlay_focus` with
    /// its failure mode removed. It is ignored outright unless the shell is what
    /// is on screen, for the same reason.
    pub shell_overlay: bool,
}

/// Decide who owns the pad.
pub fn owner_of(facts: Facts) -> InputOwner {
    match facts.on_screen {
        // Fail-closed sentinel: the compositor could not read the screen. Not
        // evidence of anything, and emphatically not evidence the shell is up.
        Some(id) if id == crate::boot::SCREEN_UNREADABLE => InputOwner::Unknown,
        Some(id) if id == facts.shell_app_id => {
            if facts.shell_overlay {
                InputOwner::ShellOverlay
            } else {
                InputOwner::Shell
            }
        }
        Some(id) => InputOwner::App { id },
        None => InputOwner::Unknown,
    }
}

/// What a change of owner must DO, as data.
///
/// Produced here and executed by [`super::session::InputSession::set_owner`],
/// so "quiesce before you switch" is a value a test can read rather than an
/// ordering buried in the one function that also opens devices.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Transition {
    pub from: InputOwner,
    pub to: InputOwner,
    /// The route to **quiesce** — release everything the target it is losing
    /// still believes is held.
    ///
    /// This is the route being LEFT, always. A target left holding a button has
    /// nothing downstream that will ever correct it: a game sees no disconnect,
    /// and the shell sees no key-up.
    pub quiesce: Route,
    /// The route to send events on once the switch is done.
    pub route: Route,
}

/// Plan a change of owner, or `None` when there is nothing to change.
///
/// **An owner change with no route change is still a transition.** App 9003 →
/// app 9004 keeps `Route::App`, and the button held when the switch happened
/// must still be released, or the new app inherits it. Same for
/// `Shell → ShellOverlay`: the drawer opening over the home screen keeps the
/// keyboard route and must not inherit a held D-pad.
pub fn plan(from: InputOwner, to: InputOwner) -> Option<Transition> {
    if from == to {
        return None;
    }
    Some(Transition {
        from,
        to,
        quiesce: from.route(),
        route: to.route(),
    })
}

/// The debounce: observations settle, assertions apply at once.
///
/// Holds the owner currently in force and, when an observation disagrees with
/// it, the candidate and when it was first seen. Pure — every method takes the
/// clock from its caller, so "did this flap collapse" is decided
/// deterministically in CI rather than by sleeping.
#[derive(Debug, Clone, Copy)]
pub struct Settle {
    settle: Duration,
    applied: InputOwner,
    pending: Option<(InputOwner, Instant)>,
}

impl Settle {
    /// Start with `initial` in force and nothing pending.
    pub fn new(settle: Duration, initial: InputOwner) -> Settle {
        Settle {
            settle,
            applied: initial,
            pending: None,
        }
    }

    /// The owner currently in force.
    pub fn applied(&self) -> InputOwner {
        self.applied
    }

    /// A poll observed `owner`.
    ///
    /// Never applies anything by itself — that is [`Settle::due`]'s job. An
    /// observation matching what is already in force *cancels* a pending change,
    /// which is how a flap out and back collapses to nothing at all.
    pub fn observed(&mut self, owner: InputOwner, now: Instant) {
        if owner == self.applied {
            self.pending = None;
            return;
        }
        match self.pending {
            // Still the same candidate: keep its ORIGINAL timestamp, so the
            // window measures how long it has been stable and not how recently
            // we looked.
            Some((pending, _)) if pending == owner => {}
            _ => self.pending = Some((owner, now)),
        }
    }

    /// When the pending change becomes due, if there is one.
    pub fn deadline(&self) -> Option<Instant> {
        self.pending.map(|(_, at)| at + self.settle)
    }

    /// Apply the pending change if it has been stable for the settle window.
    ///
    /// Returns the new owner exactly once per change.
    pub fn due(&mut self, now: Instant) -> Option<InputOwner> {
        let (owner, at) = self.pending?;
        if now < at + self.settle {
            return None;
        }
        self.pending = None;
        self.applied = owner;
        Some(owner)
    }

    /// The core itself just put something on screen — apply now.
    ///
    /// Not debounced, and that is the point: `show`, `launch`, `home` and the
    /// Guide escape's `home` each verify their own switch before returning, so
    /// there is nothing to wait out. Waiting would mean the escape returns you
    /// to a shell that ignores the controller for a third of a second, which is
    /// the exact symptom this phase exists to remove.
    pub fn asserted(&mut self, owner: InputOwner) -> Option<InputOwner> {
        self.pending = None;
        if owner == self.applied {
            return None;
        }
        self.applied = owner;
        Some(owner)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHELL: AppId = AppId::new(9001);
    const MOONLIGHT: AppId = AppId::new(9003);
    const STEAM: AppId = AppId::new(769);

    fn facts(on_screen: Option<AppId>) -> Facts {
        Facts {
            on_screen,
            shell_app_id: SHELL,
            shell_overlay: false,
        }
    }

    fn t0() -> Instant {
        Instant::now()
    }

    /// **Rule: `Unknown` routes to the APP, never to the shell.**
    ///
    /// The safety rule of the whole phase. A shell that ignores the pad while a
    /// game is visible is recoverable — the Guide escape is a core-side write
    /// that works on both routes. A pad routed into an invisible shell is not:
    /// the user sees a game and a dead controller, with nothing to press.
    ///
    /// **Mutation note.** Make `InputOwner::Unknown.route()` return
    /// `Route::Shell` and this fails. Both halves matter: the second assertion
    /// pins that `Shell` really does route elsewhere, so the first cannot pass
    /// because every owner returns `App`.
    #[test]
    fn unknown_routes_to_the_app() {
        assert_eq!(InputOwner::Unknown.route(), Route::App);
        assert_eq!(InputOwner::Shell.route(), Route::Shell);
    }

    /// **Rule: an unreadable screen is `Unknown`, not the shell.**
    ///
    /// Reachable through the real path: `Compositor::on_screen_app` fails closed
    /// by answering [`crate::boot::SCREEN_UNREADABLE`] rather than `None`, so
    /// this is the value a failed X read genuinely produces here.
    ///
    /// **Mutation note.** Drop the sentinel arm from `owner_of` and an
    /// unreadable screen becomes `App { id: 4294967295 }` — which routes the
    /// same way, so the route assertion alone would NOT catch it. The owner
    /// assertion is what does, and it matters because the owner is what
    /// `input-state` reports to a person at the television.
    #[test]
    fn an_unreadable_screen_is_unknown() {
        let owner = owner_of(facts(Some(crate::boot::SCREEN_UNREADABLE)));
        assert_eq!(owner, InputOwner::Unknown);
        assert_eq!(owner.route(), Route::App);
    }

    /// **Rule: the owner is the base window's app id against the shell's.**
    ///
    /// **Mutation note.** Swap the two ids in `owner_of`'s comparison and every
    /// arm here inverts.
    #[test]
    fn the_owner_follows_what_is_on_screen() {
        assert_eq!(owner_of(facts(Some(SHELL))), InputOwner::Shell);
        assert_eq!(
            owner_of(facts(Some(MOONLIGHT))),
            InputOwner::App { id: MOONLIGHT }
        );
        assert_eq!(owner_of(facts(None)), InputOwner::Unknown);
    }

    /// **Rule: the overlay declaration is ignored unless the shell is on screen.**
    ///
    /// A shell that declared an overlay and then lost the screen — it crashed
    /// mid-drawer, or an app was launched over it — must not keep the pad. This
    /// is the self-heal that v1's `set_overlay_focus` lacked: the declaration is
    /// folded into a decision the core makes, so a missing `release` costs
    /// nothing.
    ///
    /// **Mutation note.** Hoist the `shell_overlay` check above the app-id
    /// match — i.e. make the declaration decide on its own — and the second
    /// assertion fails.
    #[test]
    fn an_overlay_declaration_only_counts_while_the_shell_is_on_screen() {
        let declared = Facts {
            shell_overlay: true,
            ..facts(Some(SHELL))
        };
        assert_eq!(owner_of(declared), InputOwner::ShellOverlay);
        assert_eq!(InputOwner::ShellOverlay.route(), Route::Shell);

        let stale = Facts {
            shell_overlay: true,
            ..facts(Some(MOONLIGHT))
        };
        assert_eq!(
            owner_of(stale),
            InputOwner::App { id: MOONLIGHT },
            "a stale overlay declaration must not hold the pad away from a live app"
        );
    }

    /// **Rule: every transition quiesces the route it is LEAVING.**
    ///
    /// Leaving a target without releasing what it holds leaves a button nothing
    /// will ever correct — a game sees no disconnect, and the shell sees no
    /// key-up.
    ///
    /// **Mutation note.** Set `quiesce: to.route()` instead of `from.route()`
    /// and the first two assertions fail.
    #[test]
    fn a_transition_quiesces_what_it_leaves() {
        let t = plan(InputOwner::App { id: MOONLIGHT }, InputOwner::Shell).unwrap();
        assert_eq!(t.quiesce, Route::App);
        assert_eq!(t.route, Route::Shell);

        let back = plan(InputOwner::Shell, InputOwner::App { id: MOONLIGHT }).unwrap();
        assert_eq!(back.quiesce, Route::Shell);
        assert_eq!(back.route, Route::App);
    }

    /// **Rule: an owner change with no route change is still a transition.**
    ///
    /// Both cases are real. App→app is a launch from inside an app; the button
    /// held at the moment of the switch must not be inherited by the new one
    /// (jedwards1230/tv-shell#295's shape). Shell→overlay is the drawer opening
    /// over the home screen, which must not inherit a held D-pad.
    ///
    /// **Mutation note.** Return `None` when `from.route() == to.route()` and
    /// both unwraps here panic.
    #[test]
    fn a_same_route_owner_change_still_transitions() {
        let switch = plan(
            InputOwner::App { id: MOONLIGHT },
            InputOwner::App { id: STEAM },
        )
        .unwrap();
        assert_eq!(switch.quiesce, Route::App);
        assert_eq!(switch.route, Route::App);

        let drawer = plan(InputOwner::Shell, InputOwner::ShellOverlay).unwrap();
        assert_eq!(drawer.quiesce, Route::Shell);
        assert_eq!(drawer.route, Route::Shell);
    }

    /// No change is not a transition — otherwise every poll would quiesce, and
    /// holding a button would be impossible.
    #[test]
    fn an_unchanged_owner_plans_nothing() {
        assert!(plan(InputOwner::Shell, InputOwner::Shell).is_none());
        assert!(plan(
            InputOwner::App { id: MOONLIGHT },
            InputOwner::App { id: MOONLIGHT }
        )
        .is_none());
    }

    /// **Rule: a launch flap collapses to ONE net transition.**
    ///
    /// v1 learned this the hard way: a launch flaps the focused window several
    /// times over a fraction of a second, and applying each flap switches the
    /// route back and forth underneath a user who pressed one button.
    ///
    /// **Mutation note — this is the debounce mutation.** Construct with
    /// `Duration::ZERO` (or make `due` ignore the window) and the first
    /// assertion fails immediately: the intermediate `Unknown` applies.
    #[test]
    fn a_launch_flap_collapses_to_one_transition() {
        let t = t0();
        let mut s = Settle::new(SETTLE, InputOwner::Shell);
        let app = InputOwner::App { id: MOONLIGHT };

        // The flap: app, nothing, app, nothing, app — inside the window.
        s.observed(app, t);
        assert_eq!(s.due(t + Duration::from_millis(20)), None);
        s.observed(InputOwner::Unknown, t + Duration::from_millis(20));
        assert_eq!(s.due(t + Duration::from_millis(40)), None);
        s.observed(app, t + Duration::from_millis(40));
        assert_eq!(s.due(t + Duration::from_millis(60)), None);
        s.observed(InputOwner::Unknown, t + Duration::from_millis(60));
        s.observed(app, t + Duration::from_millis(80));

        // Nothing has applied yet, and the shell is still in force.
        assert_eq!(s.applied(), InputOwner::Shell);

        // Once it holds still for the window, exactly one change lands.
        assert_eq!(
            s.due(t + Duration::from_millis(80) + SETTLE),
            Some(app),
            "the settled owner applies"
        );
        assert_eq!(s.applied(), app);
        assert_eq!(
            s.due(t + Duration::from_secs(9)),
            None,
            "and it applies exactly once"
        );
    }

    /// **Rule: a candidate's window measures stability, not recency.**
    ///
    /// Re-observing the SAME candidate must not push its deadline out, or a
    /// 250 ms poll against a 300 ms window would renew it forever and nothing
    /// would ever apply.
    ///
    /// **Mutation note.** Overwrite the timestamp on every observation (drop
    /// the `pending == owner` arm) and this never applies.
    #[test]
    fn re_observing_the_same_candidate_does_not_renew_it() {
        let t = t0();
        let mut s = Settle::new(SETTLE, InputOwner::Shell);
        let app = InputOwner::App { id: MOONLIGHT };
        for ms in [0, 250, 500, 750] {
            s.observed(app, t + Duration::from_millis(ms));
        }
        assert_eq!(
            s.deadline(),
            Some(t + SETTLE),
            "measured from the first sighting"
        );
        assert_eq!(s.due(t + Duration::from_millis(750)), Some(app));
    }

    /// **Rule: a flap back to what is already in force cancels outright.**
    ///
    /// The other half of the collapse: an app that flickers away and back must
    /// produce no transition at all, not one at each edge.
    #[test]
    fn returning_to_the_applied_owner_cancels_the_pending_change() {
        let t = t0();
        let mut s = Settle::new(SETTLE, InputOwner::Shell);
        s.observed(InputOwner::Unknown, t);
        assert!(s.deadline().is_some());
        s.observed(InputOwner::Shell, t + Duration::from_millis(10));
        assert_eq!(s.deadline(), None);
        assert_eq!(s.due(t + Duration::from_secs(9)), None);
    }

    /// **Rule: a core-initiated write applies at once, and clears the debounce.**
    ///
    /// This is the escape's path. Holding Guide writes the base layer back to
    /// the shell and the owner must follow *with* it — a route that waits out
    /// the settle window is a home screen that ignores the controller for a
    /// third of a second after you asked to get back to it.
    ///
    /// **Mutation note — this is the "escape does not recompute" mutation.**
    /// Make `asserted` merely record a pending change (i.e. delegate to
    /// `observed`) and the first assertion returns `None`.
    #[test]
    fn an_asserted_owner_bypasses_the_debounce() {
        let t = t0();
        let mut s = Settle::new(SETTLE, InputOwner::App { id: MOONLIGHT });
        assert_eq!(s.asserted(InputOwner::Shell), Some(InputOwner::Shell));
        assert_eq!(s.applied(), InputOwner::Shell);
        assert_eq!(
            s.asserted(InputOwner::Shell),
            None,
            "and asserting what is already in force changes nothing"
        );

        // A stale observation queued before the assertion does not resurrect.
        let mut s = Settle::new(SETTLE, InputOwner::Shell);
        s.observed(InputOwner::App { id: MOONLIGHT }, t);
        assert_eq!(
            s.asserted(InputOwner::ShellOverlay),
            Some(InputOwner::ShellOverlay)
        );
        assert_eq!(
            s.due(t + Duration::from_secs(9)),
            None,
            "the pending observation was dropped, not merely postponed"
        );
    }
}
