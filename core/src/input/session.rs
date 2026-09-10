//! The input session: presenters created once, pads claimed and released as
//! they come and go, and events forwarded 1:1 in between.
//!
//! Synchronous by design. Every rule lives here, and the async loop that feeds
//! it lives in [`super::runtime`], so the rules are testable without a reactor,
//! a seat or a device.
//!
//! # The lifecycle, and the one thing that must never happen
//!
//! ```text
//! start()   -> create presenter 0..players   <- ONCE, for the life of the session
//!              register their devnodes as ours
//! poll()    -> enumerate -> plan -> claim (open + EVIOCGRAB) -> admit to a slot
//!                                -> leave  (quiesce the presenter, then release)
//! forward() -> translate one physical event onto the pad's presenter
//! ```
//!
//! `poll` and `forward` never create or destroy a presenter. V2_DESIGN §7:
//! create/destroy is a hotplug event every game and Moonlight forward to the
//! streaming host (jedwards1230/tv-shell#402), so a pad's unplug must be
//! invisible to whatever is reading the presenter. What the game sees instead is
//! a controller that stops moving — which is why a leave *quiesces* rather than
//! simply going quiet.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::Serialize;

use super::backend::{InputBackend, InputError};
use super::config::ResolvedInput;
use super::discovery::{OwnedNodes, Pin, Refusal};
use super::escape::{EscapeSink, GuideWatch, Release};
use super::fleet::{Fleet, FleetFull};
use super::identity::ControllerDb;
use super::keymap::{key, KeyEmit, KeyMap, KeyboardProfile};
use super::presenter::{btn, ev, quiesce, translate, DropReason, Forward, PadProfile};
use super::routing::{self, InputOwner, Route};

/// A pad newly taken into the fleet.
///
/// Informational: the backend already opened, grabbed and began reading it. The
/// stream is deliberately NOT handed out — the file descriptor is the grab, so
/// splitting the two would make `release` a request rather than a fact.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Joined {
    pub path: PathBuf,
    pub slot: u8,
}

/// The shell keyboard, as reported.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct KeyboardReport {
    pub name: String,
    pub devnodes: Vec<String>,
}

/// One presenter, as reported.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct PresenterReport {
    pub slot: u8,
    pub name: String,
    pub devnodes: Vec<String>,
}

/// One claimed pad, as reported.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct PadReport {
    pub slot: u8,
    pub wire_id: String,
    pub name: String,
    pub path: String,
    pub vendor: String,
    pub product: String,
    /// Always `true` for a pad in the fleet: the core does not hold a pad it did
    /// not grab. Reported anyway because "is my controller grabbed" is the
    /// question this verb exists to answer, and answering it by omission is how
    /// a reader ends up guessing.
    pub grabbed: bool,
}

/// One device the gate refused, and why.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct RefusedReport {
    pub path: String,
    pub name: String,
    pub vendor: String,
    pub product: String,
    pub guid: String,
    pub reason: Refusal,
    pub explanation: String,
}

/// The Guide escape, as reported.
///
/// In the report rather than left to be inferred, for the reason `owner` and
/// `route` are: the acceptance for this is a person at a television, and
/// "did the core see my press, and did the write happen" must be readable
/// there instead of guessed from whether the screen changed.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct EscapeReport {
    /// The configured tap-vs-hold threshold.
    pub hold_ms: u64,
    /// A pad is holding Guide and its threshold has not been reached yet.
    /// This is what distinguishes "the core never saw the button" from "the
    /// core saw it and the write did not take".
    pub armed: bool,
    /// Holds that fired and were handed off for a base-layer write.
    pub fires: u64,
    /// Holds that fired and could NOT be handed off. Non-zero means a user
    /// asked to leave an app and the core could not make it happen.
    pub failures: u64,
    /// When the last fire was handed off, in Unix milliseconds.
    pub last_fire_unix_ms: Option<u64>,
}

/// The `input-state` payload.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct InputReport {
    /// False when `[input].enabled` is off, in which case every list below is
    /// empty because nothing was ever opened.
    pub enabled: bool,
    pub players: u8,
    pub presenters: Vec<PresenterReport>,
    pub pads: Vec<PadReport>,
    pub refused: Vec<RefusedReport>,
    /// Events that did not cross onto a presenter, by reason. Present so a pad
    /// losing a button is a number an operator can read rather than a mystery.
    pub drops: BTreeMap<DropReason, u64>,
    /// When the last discovery pass completed, in Unix milliseconds.
    ///
    /// **This is how a stopped input loop becomes visible.** `input-state`
    /// answers from a snapshot so it cannot hang when the loop is wedged, and
    /// the price of that is a report which looks plausible whether the loop is
    /// running or dead. A timestamp that stops advancing distinguishes them.
    /// `None` before the first poll, and while disabled.
    pub last_poll_unix_ms: Option<u64>,
    /// Events the backend REFUSED to emit.
    ///
    /// Distinct from `drops`, which are events this crate decided not to
    /// forward. These are events it did forward and the device rejected -- a
    /// uinput node in a bad state, or one that went away.
    ///
    /// It exists because `retire` documents that it returns a presenter to rest,
    /// and a failed emit makes that claim false with nothing else able to
    /// notice: the pad is gone, so no later event corrects the stuck button, and
    /// from a game's side no device disconnected. A non-zero count here means a
    /// player may be holding a button nothing will release.
    pub emit_failures: u64,
    /// Who the core believes owns the pad. See [`InputOwner`].
    pub owner: InputOwner,
    /// Where pad events are going right now. See [`Route`].
    pub route: Route,
    /// The shell keyboard, when this session created one. `None` when
    /// `[input].shell_keys` is off — the device is then never created at all.
    pub keyboard: Option<KeyboardReport>,
    /// Buttons held across a route change, whose press/release must not reach
    /// the new target (jedwards1230/tv-shell#295).
    ///
    /// **Still always empty.** Phase 2 makes the route a decision and quiesces
    /// each transition, which releases what the OLD target holds — but nothing
    /// yet suppresses the physical release that lands on the NEW one. That is
    /// phase 3. Reported rather than omitted so the gap is readable.
    pub masked_keys: Vec<u16>,
    /// Axes held across a route change. Empty, as above.
    pub masked_axes: Vec<u16>,
    /// The Guide escape (jedwards1230/tv-shell#496). See [`EscapeReport`].
    pub escape: EscapeReport,
    /// How many discovery passes have COMPLETED.
    ///
    /// Beside the timestamp rather than instead of it, for two different
    /// readers. A person wants the wall clock ("it last looked a minute ago");
    /// a test — and a metric — needs something that changes on every pass, and
    /// a millisecond stamp does not: two polls in the same tick carry the same
    /// value, so an assertion built on the timestamp alone cannot tell "it did
    /// not run" from "it ran again quickly". This can.
    pub polls_completed: u64,
}

impl InputReport {
    /// The report for a core running with input disabled.
    ///
    /// Structurally empty, because with the flag off there is no session: no
    /// enumeration has happened, no device has been opened and no presenter
    /// exists. This is the value the IPC verb returns in that case, and it is
    /// built without touching hardware.
    pub fn disabled() -> InputReport {
        InputReport {
            enabled: false,
            players: 0,
            presenters: Vec::new(),
            pads: Vec::new(),
            refused: Vec::new(),
            drops: BTreeMap::new(),
            emit_failures: 0,
            last_poll_unix_ms: None,
            polls_completed: 0,
            // Nothing is grabbed, so the pad reaches whatever is on screen
            // directly. That is honestly `Unknown` — the core is deciding
            // nothing — and `Unknown` routes to the app, which is exactly what
            // is happening.
            owner: InputOwner::Unknown,
            route: Route::App,
            keyboard: None,
            masked_keys: Vec::new(),
            masked_axes: Vec::new(),
            // No pad is grabbed, so no Guide press ever reaches the core and
            // the escape cannot arm. Reported as the configured default rather
            // than zero, so the number does not read as "the hold is disabled".
            escape: EscapeReport {
                hold_ms: super::escape::DEFAULT_HOLD_MS,
                armed: false,
                fires: 0,
                failures: 0,
                last_fire_unix_ms: None,
            },
        }
    }
}

/// Milliseconds since the Unix epoch, saturating rather than panicking on a
/// clock set before 1970.
fn unix_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// The running input layer.
pub struct InputSession<B: InputBackend> {
    backend: B,
    profile: PadProfile,
    fleet: Fleet,
    owned: OwnedNodes,
    db: ControllerDb,
    pin: Pin,
    players: u8,
    presenters: Vec<PresenterReport>,
    drops: BTreeMap<DropReason, u64>,
    /// The most recent poll's refusals, so `input-state` explains what the core
    /// is currently declining rather than everything it ever declined.
    refused: Vec<RefusedReport>,
    emit_failures: u64,
    last_poll_unix_ms: Option<u64>,
    polls_completed: u64,
    /// Who the core currently believes owns the pad.
    ///
    /// **A decision, recomputed by [`super::watcher`] and pushed in over the
    /// control channel** — never read from the compositor on this thread. See
    /// [`Self::set_owner`].
    owner: InputOwner,
    route: Route,
    /// `[input].shell_keys`: the owner is PINNED to the shell and arbitration is
    /// refused outright.
    ///
    /// The operator override, kept from phase 1. It exists so a session can be
    /// forced onto the shell route for a measurement even when the screen says
    /// otherwise — and so the flag someone already has in `core.toml` keeps
    /// doing what its documentation said it did.
    pinned: bool,
    keyboard: Option<KeyboardReport>,
    /// One translator per claimed pad, so two players keep independent repeat
    /// timers. Present only on the shell route.
    keymaps: BTreeMap<PathBuf, KeyMap>,
    /// Where a fired Guide hold goes. Boxed so the session's rules are testable
    /// against a recording double with no compositor — and so the escape's own
    /// failure is a thing a test can produce.
    escape: Box<dyn EscapeSink>,
    /// The tap-vs-hold threshold, from `[input].guide_hold_ms`.
    guide_hold: std::time::Duration,
    /// One Guide state machine per claimed pad. **Per pad is the rule, not an
    /// implementation detail** — see [`GuideWatch`].
    guides: BTreeMap<PathBuf, GuideWatch>,
    /// Fleet-level dedup latch, ported from v1's `home_hold_active`. Set on the
    /// first pad's fire and cleared only when NO pad holds Guide, so two pads
    /// held down together escape once rather than twice.
    escape_latched: bool,
    escape_fires: u64,
    escape_failures: u64,
    escape_last_fire_unix_ms: Option<u64>,
}

impl<B: InputBackend> InputSession<B> {
    /// Create every presenter, then return a session ready to poll.
    ///
    /// **Presenter creation is here and nowhere else.** It happens once, before
    /// any pad is looked at, and a failure is fatal to the session rather than
    /// degraded: a core that came up with three of four presenters would hand
    /// player four's input nowhere, and the `players` count it reports would be
    /// a lie.
    pub fn start(
        mut backend: B,
        config: &ResolvedInput,
        escape: Box<dyn EscapeSink>,
    ) -> Result<InputSession<B>, InputError> {
        let profile = PadProfile::canonical();
        let mut owned = OwnedNodes::new();
        let mut presenters = Vec::new();

        for slot in 0..config.players {
            let devnodes = backend.create_presenter(slot, &profile)?;
            if devnodes.is_empty() {
                return Err(InputError::Presenter {
                    slot,
                    detail: "the backend reported no devnode, so discovery could not \
                             recognise this presenter as ours and would grab it"
                        .into(),
                });
            }
            for node in &devnodes {
                owned.register(node.clone());
            }
            presenters.push(PresenterReport {
                slot,
                name: PadProfile::device_name(slot),
                devnodes: devnodes.iter().map(|p| p.display().to_string()).collect(),
            });
        }

        tracing::info!(
            players = config.players,
            "input presenters created; they persist for the life of this session"
        );

        // The shell keyboard, created HERE and nowhere else — the same
        // permanence rule the presenters follow (§7 / jedwards1230/tv-shell#402):
        // a device that appeared and vanished with a route change would be a
        // hotplug event apps forward to the streaming host.
        //
        // **It is created unconditionally now, and that is a phase-2 change.**
        // Phase 1 created it only under `shell_keys`, which was correct while
        // the route was fixed for the life of the session. It is not correct
        // once the owner is arbitrated: any session may be handed the shell
        // route at any moment, and the one thing that must never happen is
        // creating the device at that moment. The default-off promise is
        // unaffected — `[input].enabled` gates `start` itself, so a session that
        // creates a keyboard is one an operator asked for.
        let devnodes = backend.create_keyboard(&KeyboardProfile)?;
        if devnodes.is_empty() {
            return Err(InputError::Keyboard(
                "the backend reported no devnode for the keyboard, so discovery could \
                 not be taught to skip it"
                    .into(),
            ));
        }
        for node in &devnodes {
            owned.register(node.clone());
        }
        let keyboard = Some(KeyboardReport {
            name: KeyboardProfile::device_name(),
            devnodes: devnodes.iter().map(|p| p.display().to_string()).collect(),
        });

        // The starting owner. With `shell_keys` the session is pinned to the
        // shell for its whole life, exactly as phase 1 behaved. Without it the
        // owner is `Unknown` — the honest answer before anything has looked at
        // the screen — which routes to the app, so a core that comes up while a
        // game is running does not steal the pad in the window before the
        // watcher's first decision arrives.
        let (owner, pinned) = if config.shell_keys {
            tracing::warn!(
                "[input].shell_keys is ON: the owner is PINNED to the SHELL and arbitration \
                 is disabled, so no app or game receives pad input while this core runs"
            );
            (InputOwner::Shell, true)
        } else {
            (InputOwner::Unknown, false)
        };
        let route = owner.route();

        Ok(InputSession {
            backend,
            profile,
            fleet: Fleet::new(config.players),
            owned,
            db: config.db.clone(),
            pin: config.pin,
            players: config.players,
            presenters,
            drops: BTreeMap::new(),
            refused: Vec::new(),
            emit_failures: 0,
            last_poll_unix_ms: None,
            polls_completed: 0,
            owner,
            route,
            pinned,
            keyboard,
            keymaps: BTreeMap::new(),
            escape,
            guide_hold: config.guide_hold,
            guides: BTreeMap::new(),
            escape_latched: false,
            escape_fires: 0,
            escape_failures: 0,
            escape_last_fire_unix_ms: None,
        })
    }

    /// One discovery pass: claim what is new, retire what is gone.
    ///
    /// Returns the pads that joined. An enumeration failure is reported and changes nothing —
    /// notably it does **not** retire the whole fleet, because "we could not
    /// read the device list" is not evidence that every pad was unplugged.
    pub fn poll(&mut self) -> Vec<Joined> {
        let candidates = match self.backend.enumerate() {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!("input discovery: {e}");
                return Vec::new();
            }
        };
        let plan = self
            .fleet
            .plan(&candidates, &self.db, &self.owned, self.pin);

        self.refused = plan
            .refuse
            .iter()
            .map(|(c, reason)| RefusedReport {
                path: c.path.display().to_string(),
                name: c.name.clone(),
                vendor: format!("{:04x}", c.vendor),
                product: format!("{:04x}", c.product),
                guid: c.guid(),
                reason: *reason,
                explanation: reason.explain().to_string(),
            })
            .collect();

        for path in plan.leave {
            self.retire(&path);
        }

        let mut joined = Vec::new();
        for candidate in plan.claim {
            let source_axes = match self.backend.claim(&candidate.path) {
                Ok(a) => a,
                Err(e) => {
                    // A pad we cannot open is a pad we do not hold. Left out of
                    // the fleet, it is re-tried on the next poll — which is the
                    // right behaviour for a device still settling after a plug.
                    tracing::warn!("{e}");
                    continue;
                }
            };
            match self.fleet.admit(&candidate, source_axes) {
                Ok(slot) => {
                    tracing::info!(
                        slot,
                        pad = %candidate.name,
                        path = %candidate.path.display(),
                        "pad claimed"
                    );
                    joined.push(Joined {
                        path: candidate.path.clone(),
                        slot,
                    });
                }
                Err(FleetFull) => {
                    // We grabbed it and then found no slot: give it straight
                    // back rather than holding a pad we will never present.
                    // Otherwise the pad is exclusively ours and dead to
                    // everything, which is worse than not claiming it.
                    tracing::warn!(
                        players = self.players,
                        pad = %candidate.name,
                        "no free player slot; releasing the pad rather than holding it unpresented"
                    );
                    self.backend.release(&candidate.path);
                }
            }
        }
        self.last_poll_unix_ms = Some(unix_millis());
        self.polls_completed += 1;
        joined
    }

    /// Forward one physical event onto its pad's presenter.
    ///
    /// The 1:1 passthrough. An event from a pad the fleet does not hold is
    /// ignored — that is a stream draining after a retire, not something to
    /// route at a slot which may already belong to another player.
    ///
    /// Returns `true` when the event changed something the report carries — in
    /// practice, a Guide press or release. The runtime republishes on that, so
    /// `escape.armed` is observable *while* a hold is in progress rather than
    /// only after it resolves, which is the whole reason the field exists.
    ///
    /// **`now` is passed in, never read here.** Two time-dependent rules hang
    /// off this call — the stick auto-repeat and the Guide tap-vs-hold — and a
    /// clock read inside routing code makes both untestable except by sleeping.
    /// The runtime passes `Instant::now()`; a test passes the instant it means.
    pub fn forward(
        &mut self,
        path: &Path,
        event_type: u16,
        code: u16,
        value: i32,
        now: std::time::Instant,
    ) -> bool {
        let Some(pad) = self.fleet.get(path) else {
            return false;
        };
        let slot = pad.slot;
        let source_axis = if event_type == ev::ABS {
            pad.source_axes.get(&code).copied()
        } else {
            None
        };

        // Guide is the core's, on EVERY route, and it never crosses live.
        //
        // Before the route split rather than inside it: the escape is the way
        // back from a running app, so it cannot be a property of the shell
        // route — with `shell_keys` off the pad reaches the app and this is the
        // only thing that can end that. v1 buffers the press for the same
        // reason it is buffered here: forwarding it and retracting later is not
        // possible, so nothing is forwarded until the release says which
        // gesture it was.
        if event_type == ev::KEY && code == btn::MODE {
            self.on_guide(path, slot, value, now);
            return true;
        }

        if self.route == Route::Shell {
            // The pad drives the shell: nothing reaches the presenter, so a game
            // behind the shell sees a controller sitting still rather than one
            // being used by someone else.
            let emits = self
                .keymaps
                .entry(path.to_path_buf())
                .or_default()
                .on_event(event_type, code, value, source_axis, now);
            for emit in emits {
                self.emit_key(emit);
            }
            return false;
        }

        let forward = translate(event_type, code, value, source_axis, &self.profile);
        if let Forward::Drop(reason) = forward {
            *self.drops.entry(reason).or_insert(0) += 1;
            return false;
        }
        // Track the button BEFORE emitting: if the emit fails we still know what
        // the pad is holding, and quiesce stays correct.
        if let Forward::Key { code, value } = forward {
            self.fleet.note_key(path, code, value);
        }
        self.emit(slot, forward);
        false
    }

    /// One Guide event, on the pad at `path`.
    ///
    /// The press arms; the release delivers a tap or is swallowed; the hold
    /// itself fires from [`Self::tick`], because a threshold that elapses while
    /// the user holds perfectly still produces no event to hang it off.
    fn on_guide(&mut self, path: &Path, slot: u8, value: i32, now: std::time::Instant) {
        match value {
            1 => {
                self.guides
                    .entry(path.to_path_buf())
                    .or_default()
                    .press(now);
            }
            0 => {
                let release = self.guides.entry(path.to_path_buf()).or_default().release();
                match release {
                    // A tap belongs to whatever is on screen. On the app route
                    // that is a real Guide press+release on the presenter (v1's
                    // `ReplayToPad`, so Steam Big Picture still opens); on the
                    // shell route it is the drawer, which is v1's `HomeTap` and
                    // reaches the v2 shell as Tab — the key measured to arrive
                    // (§2.1), not KEY_MENU, which gamescope drops.
                    Release::Tap => match self.route {
                        Route::App => {
                            for v in [1, 0] {
                                self.emit(
                                    slot,
                                    Forward::Key {
                                        code: btn::MODE,
                                        value: v,
                                    },
                                );
                            }
                        }
                        Route::Shell => {
                            for v in [1, 0] {
                                self.emit_key(KeyEmit {
                                    code: key::TAB,
                                    value: v,
                                });
                            }
                        }
                    },
                    // The hold already fired. Swallowed on purpose: the target
                    // never saw the press, so a release would be the only edge
                    // it ever got — exactly the leak jedwards1230/tv-shell#295
                    // is about, in the one direction this phase can produce.
                    Release::Swallow | Release::Ignore => {}
                }
                self.clear_escape_latch();
            }
            // A kernel autorepeat on a held Guide is not a new press, and must
            // not restart the hold timer.
            _ => {}
        }
    }

    /// Clear the fleet latch once NO pad is holding Guide.
    ///
    /// v1's rule, and the reason it is "no pad holding" rather than "no pad
    /// armed": a fired hold stays `holding` until the button comes up, so the
    /// latch cannot clear underneath the very press that set it.
    fn clear_escape_latch(&mut self) {
        if self.escape_latched && !self.guides.values().any(|g| g.holding()) {
            self.escape_latched = false;
        }
    }

    /// Hand a fired hold to the escape sink, and count what comes back.
    ///
    /// **The core does this itself.** Not a message to the shell — v1's
    /// `intent home-hold` was exactly that, and it failed whenever the shell was
    /// wedged, which is the only time anyone needs it. See
    /// [`super::escape`].
    fn fire_escape(&mut self, slot: u8) {
        if self.escape_latched {
            // Another pad's hold is already escaping. One `home` is the whole
            // gesture; a second would be a second switch to the same place.
            return;
        }
        self.escape_latched = true;
        tracing::info!(slot, "Guide hold: returning the screen to the shell");
        match self.escape.fire() {
            Ok(()) => {
                self.escape_fires += 1;
                self.escape_last_fire_unix_ms = Some(unix_millis());
            }
            Err(e) => {
                self.escape_failures += 1;
                tracing::error!("the Guide escape fired and could not be delivered: {e}");
            }
        }
    }

    /// Apply an owner decision computed by [`super::watcher`].
    ///
    /// **The only thing that changes the route.** It is a decision handed in,
    /// never one taken here: the screen read that produced it happened on
    /// another thread, because an X round trip on the pad path would make
    /// controller latency a function of how busy the compositor is.
    ///
    /// The order is the rule, and it is [`routing::plan`] that states it:
    /// **quiesce the route being left, THEN switch.** Leaving a target still
    /// holding a button leaves one nothing downstream will correct — a game
    /// sees no disconnect and the shell sees no key-up — so the release has to
    /// go to the target that believes it has the button, which is only true
    /// before the switch.
    ///
    /// Returns whether anything changed, so the runtime republishes the report
    /// on a transition and not on every poll.
    ///
    /// # What this does NOT do yet
    ///
    /// It does not **mask**. Quiesce releases what the old target holds; it
    /// cannot suppress the physical release that arrives *afterwards* and
    /// crosses to the new one. So a button held across a transition still
    /// delivers a lone release to the target it did not press on — which is
    /// jedwards1230/tv-shell#295's shape, and is phase 3's job
    /// (`mask_forward_decision`, ported verbatim from v1). `masked_keys` and
    /// `masked_axes` stay empty for exactly that reason, and this is a stated
    /// gap rather than a solved problem.
    pub fn set_owner(&mut self, owner: InputOwner) -> bool {
        if self.pinned {
            // `[input].shell_keys` is an operator override, so it outranks the
            // arbitration rather than racing it. Logged at debug because the
            // watcher will keep offering: it is doing its job, and this is the
            // pin doing its own.
            tracing::debug!(?owner, "ignoring an owner decision: the route is pinned");
            return false;
        }
        let Some(transition) = routing::plan(self.owner, owner) else {
            return false;
        };
        tracing::info!(
            from = ?transition.from,
            to = ?transition.to,
            route = ?transition.route,
            "input owner transition"
        );
        self.quiesce_route(transition.quiesce);
        self.owner = transition.to;
        self.route = transition.route;
        true
    }

    /// Release everything the given route's target believes is held.
    ///
    /// Both halves exist for the same reason and neither is optional: the
    /// presenters and the keyboard both OUTLIVE any one route, by design (§7 /
    /// jedwards1230/tv-shell#402), so nothing about a route change tells the
    /// thing downstream to let go.
    fn quiesce_route(&mut self, route: Route) {
        match route {
            Route::App => {
                for (slot, held) in self.fleet.take_held() {
                    for forward in quiesce(&held, &self.profile) {
                        self.emit(slot, forward);
                    }
                }
            }
            Route::Shell => {
                let mut emits = Vec::new();
                for map in self.keymaps.values_mut() {
                    emits.extend(map.quiesce());
                }
                for emit in emits {
                    self.emit_key(emit);
                }
            }
        }
    }

    /// A pad's event stream failed — a USB unplug, usually. Retire it now rather
    /// than waiting for the next poll to notice its absence.
    pub fn on_stream_error(&mut self, path: &Path) {
        self.retire(path);
    }

    /// Drop a pad: return its presenter to rest, then release the grab.
    ///
    /// **Order matters.** Releasing first widens the window in which the pad is
    /// free while the presenter is still holding a button — exactly the state a
    /// game reads as stuck input.
    fn retire(&mut self, path: &Path) {
        let Some(retired) = self.fleet.retire(path) else {
            return;
        };
        tracing::info!(
            slot = retired.slot,
            pad = %retired.wire_id,
            held = retired.held_keys.len(),
            "pad left; returning its presenter to rest (the presenter itself stays)"
        );
        for forward in quiesce(&retired.held_keys, &self.profile) {
            self.emit(retired.slot, forward);
        }
        // The keyboard outlives the pad exactly as the presenter does, so a pad
        // that left mid-press must have its keys released too — otherwise the
        // shell is left navigating in one direction forever, with nothing
        // downstream able to correct it.
        if let Some(mut map) = self.keymaps.remove(path) {
            for emit in map.quiesce() {
                self.emit_key(emit);
            }
        }
        // A pad unplugged mid-hold takes its Guide state with it, and can then
        // release the fleet latch — otherwise a pad yanked while holding Guide
        // would leave the escape latched for the life of the session, and the
        // NEXT hold on another pad would do nothing at all.
        self.guides.remove(path);
        self.clear_escape_latch();
        self.backend.release(path);
    }

    /// Emit onto a presenter, counting a refusal.
    ///
    /// A failure is logged and counted rather than propagated: there is nothing
    /// useful a caller can do about a uinput node that will not take an event,
    /// and both callers — a live passthrough and a leave's quiesce — have to
    /// carry on regardless. Counting is what keeps it from being *silent*. A
    /// quiesce that fails leaves a button held on a presenter whose pad is gone,
    /// and nothing downstream can notice, because from a game's side no device
    /// disconnected. See [`InputReport::emit_failures`].
    fn emit(&mut self, slot: u8, forward: Forward) {
        if let Err(e) = self.backend.emit(slot, forward) {
            self.emit_failures += 1;
            tracing::warn!("{e}");
        }
    }

    /// Emit one key on the shell keyboard, counting a refusal.
    ///
    /// Counted rather than propagated for the same reason as [`Self::emit`]: a
    /// key the device refuses is a key nothing else will correct, and silence is
    /// the only unacceptable outcome.
    fn emit_key(&mut self, emit: KeyEmit) {
        if let Err(e) = self.backend.emit_key(emit) {
            self.emit_failures += 1;
            tracing::warn!("{e}");
        }
    }

    /// When this session next needs waking, across every pad.
    ///
    /// Two kinds of deadline, deliberately merged into one: a stick auto-repeat
    /// and a Guide hold. The runtime sleeps until the earliest instead of
    /// polling, so a session with nothing armed costs nothing.
    pub fn next_deadline(&self) -> Option<std::time::Instant> {
        let repeats = self.keymaps.values().filter_map(|m| m.next_deadline());
        let holds = self
            .guides
            .values()
            .filter_map(|g| g.deadline(self.guide_hold));
        repeats.chain(holds).min()
    }

    /// Fire every stick auto-repeat, and every Guide hold, due at `now`.
    pub fn tick(&mut self, now: std::time::Instant) {
        // The escape first: it is the thing someone is waiting on, and a burst
        // of repeats should not sit in front of it.
        let hold = self.guide_hold;
        let fired: Vec<PathBuf> = self
            .guides
            .iter_mut()
            .filter_map(|(path, g)| g.due(now, hold).then(|| path.clone()))
            .collect();
        for path in fired {
            if let Some(slot) = self.fleet.get(&path).map(|p| p.slot) {
                self.fire_escape(slot);
            }
        }

        let mut emits = Vec::new();
        for map in self.keymaps.values_mut() {
            emits.extend(map.tick(now));
        }
        for emit in emits {
            self.emit_key(emit);
        }
    }

    /// The `input-state` payload.
    pub fn report(&self) -> InputReport {
        InputReport {
            enabled: true,
            players: self.players,
            presenters: self.presenters.clone(),
            pads: self
                .fleet
                .pads()
                .map(|p| PadReport {
                    slot: p.slot,
                    wire_id: p.wire_id.clone(),
                    name: p.name.clone(),
                    path: p.path.display().to_string(),
                    vendor: format!("{:04x}", p.vendor),
                    product: format!("{:04x}", p.product),
                    grabbed: true,
                })
                .collect(),
            refused: self.refused.clone(),
            drops: self.drops.clone(),
            emit_failures: self.emit_failures,
            last_poll_unix_ms: self.last_poll_unix_ms,
            polls_completed: self.polls_completed,
            owner: self.owner,
            route: self.route,
            keyboard: self.keyboard.clone(),
            // STILL EMPTY, and now for a sharper reason than in phase 1.
            //
            // There ARE route changes to mask across now — that is the whole of
            // this phase — and `set_owner` quiesces each one. Quiesce is only
            // half the fix: it releases what the target being LEFT is holding,
            // and cannot suppress the physical release that arrives afterwards
            // and crosses to the NEW target. A button held across a transition
            // therefore still delivers a lone release to something that never
            // saw the press, which is jedwards1230/tv-shell#295's shape.
            //
            // Masking is phase 3 (`mask_forward_decision` and its axis sibling,
            // ported verbatim from v1 with their tests). Reported as empty
            // rather than omitted so the gap is readable at the television
            // instead of inferred from a bug.
            masked_keys: Vec::new(),
            masked_axes: Vec::new(),
            escape: EscapeReport {
                hold_ms: self.guide_hold.as_millis() as u64,
                armed: self.guides.values().any(|g| g.armed()),
                fires: self.escape_fires,
                failures: self.escape_failures,
                last_fire_unix_ms: self.escape_last_fire_unix_ms,
            },
        }
    }

    /// The backend, for the concrete async runtime that must await its streams.
    ///
    /// A deliberate, narrow leak: multiplexing event streams needs the concrete
    /// backend type, and putting an `async fn` on [`InputBackend`] would drag a
    /// reactor into every rule this module states.
    pub fn backend_mut(&mut self) -> &mut B {
        &mut self.backend
    }

    /// Release every pad. The presenters go with the process (see
    /// [`super::runtime`] on unclean exits).
    pub fn shutdown(&mut self) {
        let paths: Vec<PathBuf> = self.fleet.pads().map(|p| p.path.clone()).collect();
        for path in paths {
            self.retire(&path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input::discovery::Candidate;
    use crate::input::identity::bundled_db;
    use crate::input::keymap;
    use crate::input::presenter::{abs, btn, AbsRange, SYN_REPORT};
    use std::cell::RefCell;
    use std::collections::BTreeSet;
    use std::rc::Rc;

    /// What the double recorded. Shared so a test can read it while the session
    /// still owns the backend.
    #[derive(Debug, Default)]
    struct Log {
        /// Every `create_presenter(slot)` call, in order. The count is the
        /// jedwards1230/tv-shell#402 assertion.
        created: Vec<u8>,
        claimed: Vec<PathBuf>,
        released: Vec<PathBuf>,
        /// For each release, how many events had been emitted when it happened.
        /// This is what makes the quiesce-BEFORE-release ordering assertable:
        /// two independent lists record that both occurred but never in which
        /// order, and the order is the rule.
        released_at_emit_count: Vec<usize>,
        /// `(slot, forward)` for everything emitted.
        emitted: Vec<(u8, Forward)>,
        /// Every device this crate asked the backend to CREATE, in order, as
        /// `"presenter-N"` / `"keyboard"`.
        ///
        /// Beside `created` rather than instead of it: the #402 rule is about
        /// every permanent device, and a keyboard created on a route change
        /// would be exactly the hotplug that rule forbids. One ordered list is
        /// what makes "nothing was created outside `start`" assertable.
        device_creations: Vec<String>,
        /// Everything emitted on the shell keyboard, in order.
        keys: Vec<KeyEmit>,
        /// For each keyboard emit, how many presenter events had been emitted
        /// when it happened.
        ///
        /// The same trick `released_at_emit_count` uses, for the same reason:
        /// two independent lists record that both happened but never in which
        /// order, and with a route transition the ORDER is the rule.
        keys_at_emit_count: Vec<usize>,
    }

    /// A recording backend.
    ///
    /// **It fakes no device behaviour.** It does not pretend to grab, to exclude
    /// another reader, or to deliver input; it records which calls this crate
    /// made, in what order. Every assertion below is therefore about our own
    /// sequencing — never about a behaviour the double invented.
    struct Recorder {
        log: Rc<RefCell<Log>>,
        /// What `enumerate` returns next.
        devices: Rc<RefCell<Vec<Candidate>>>,
        /// Paths whose `claim` must fail, to exercise the open-failure path.
        claim_fails: BTreeSet<PathBuf>,
        /// When set, `create_presenter` returns no devnodes.
        presenter_without_devnode: bool,
        /// When set, `create_keyboard` returns no devnodes.
        keyboard_without_devnode: bool,
        /// When set, `enumerate` fails.
        enumerate_fails: bool,
        /// When set, the presenter refuses every event.
        emit_fails: bool,
    }

    impl Recorder {
        fn new(log: &Rc<RefCell<Log>>, devices: &Rc<RefCell<Vec<Candidate>>>) -> Recorder {
            Recorder {
                log: Rc::clone(log),
                devices: Rc::clone(devices),
                claim_fails: BTreeSet::new(),
                presenter_without_devnode: false,
                keyboard_without_devnode: false,
                enumerate_fails: false,
                emit_fails: false,
            }
        }
    }

    impl InputBackend for Recorder {
        fn enumerate(&mut self) -> Result<Vec<Candidate>, InputError> {
            if self.enumerate_fails {
                return Err(InputError::Enumerate("permission denied".into()));
            }
            Ok(self.devices.borrow().clone())
        }

        fn create_presenter(
            &mut self,
            slot: u8,
            _profile: &PadProfile,
        ) -> Result<Vec<PathBuf>, InputError> {
            {
                let mut log = self.log.borrow_mut();
                log.created.push(slot);
                log.device_creations.push(format!("presenter-{slot}"));
            }
            if self.presenter_without_devnode {
                return Ok(Vec::new());
            }
            Ok(vec![PathBuf::from(format!("/dev/input/event2{slot}"))])
        }

        fn claim(&mut self, path: &Path) -> Result<BTreeMap<u16, AbsRange>, InputError> {
            if self.claim_fails.contains(path) {
                return Err(InputError::Claim {
                    path: path.to_path_buf(),
                    detail: "device busy".into(),
                });
            }
            self.log.borrow_mut().claimed.push(path.to_path_buf());
            Ok(BTreeMap::from([
                (abs::X, AbsRange::new(-32768, 32767, 16, 128)),
                (abs::Z, AbsRange::new(0, 255, 0, 0)),
            ]))
        }

        fn release(&mut self, path: &Path) {
            let mut log = self.log.borrow_mut();
            let emitted = log.emitted.len();
            log.released.push(path.to_path_buf());
            log.released_at_emit_count.push(emitted);
        }

        fn emit(&mut self, slot: u8, forward: Forward) -> Result<(), InputError> {
            if self.emit_fails {
                return Err(InputError::Emit {
                    slot,
                    detail: "the uinput node refused the event".into(),
                });
            }
            self.log.borrow_mut().emitted.push((slot, forward));
            Ok(())
        }

        fn create_keyboard(
            &mut self,
            _profile: &KeyboardProfile,
        ) -> Result<Vec<PathBuf>, InputError> {
            self.log
                .borrow_mut()
                .device_creations
                .push("keyboard".into());
            if self.keyboard_without_devnode {
                return Ok(Vec::new());
            }
            Ok(vec![PathBuf::from("/dev/input/event30")])
        }

        fn emit_key(&mut self, emit: KeyEmit) -> Result<(), InputError> {
            if self.emit_fails {
                return Err(InputError::EmitKey {
                    code: emit.code,
                    detail: "the keyboard refused the event".into(),
                });
            }
            let mut log = self.log.borrow_mut();
            let emitted = log.emitted.len();
            log.keys.push(emit);
            log.keys_at_emit_count.push(emitted);
            Ok(())
        }
    }

    /// What the escape double recorded.
    ///
    /// Beside [`Log`] rather than inside it, deliberately: the whole claim is
    /// that firing the escape is a base-layer write and touches *nothing* the
    /// backend does, so the two must be separately readable. A test asserts a
    /// fire happened AND that the keyboard/presenter logs did not move.
    #[derive(Debug, Default)]
    struct EscapeLog {
        fires: usize,
    }

    /// A recording [`EscapeSink`]. It performs no write and fakes none — it
    /// records that the session asked for one, which is the session's whole
    /// half of the contract.
    struct RecordingEscape {
        log: Rc<RefCell<EscapeLog>>,
        /// When set, every fire is refused — the "fired and could not write"
        /// case, which must show up as a number rather than a silence.
        fails: bool,
    }

    impl EscapeSink for RecordingEscape {
        fn fire(&mut self) -> Result<(), super::super::escape::EscapeError> {
            self.log.borrow_mut().fires += 1;
            if self.fails {
                return Err(super::super::escape::EscapeError(
                    "the escape worker is gone".into(),
                ));
            }
            Ok(())
        }
    }

    /// A sink for the many tests that never press Guide. It panics if it is
    /// ever used, so a test that accidentally fires an escape says so instead of
    /// passing quietly.
    struct NeverFires;

    impl EscapeSink for NeverFires {
        fn fire(&mut self) -> Result<(), super::super::escape::EscapeError> {
            panic!("this test fired the Guide escape and did not mean to");
        }
    }

    fn never_fires() -> Box<dyn EscapeSink> {
        Box::new(NeverFires)
    }

    struct Harness {
        session: InputSession<Recorder>,
        log: Rc<RefCell<Log>>,
        devices: Rc<RefCell<Vec<Candidate>>>,
        escapes: Rc<RefCell<EscapeLog>>,
    }

    fn config(players: u8) -> ResolvedInput {
        ResolvedInput {
            players,
            shell_keys: false,
            db: bundled_db(),
            pin: None,
            poll_interval: std::time::Duration::from_secs(2),
            guide_hold: HOLD,
        }
    }

    /// Wall-clock now, for the many tests whose events carry no timing meaning.
    fn t0() -> std::time::Instant {
        std::time::Instant::now()
    }

    /// The escape threshold every test below uses — the real default, so a
    /// hold in a test is the same hold a couch produces.
    const HOLD: std::time::Duration =
        std::time::Duration::from_millis(super::super::escape::DEFAULT_HOLD_MS);

    fn harness(players: u8) -> Harness {
        harness_with(config(players))
    }

    /// A harness on the SHELL route — reached the only way the real core reaches
    /// it, by setting `shell_keys` in the config `start` is given.
    fn shell_harness(players: u8) -> Harness {
        harness_with(ResolvedInput {
            shell_keys: true,
            ..config(players)
        })
    }

    fn harness_with(config: ResolvedInput) -> Harness {
        harness_with_escape(config, false)
    }

    fn harness_with_escape(config: ResolvedInput, escape_fails: bool) -> Harness {
        let log = Rc::new(RefCell::new(Log::default()));
        let devices = Rc::new(RefCell::new(Vec::new()));
        let escapes = Rc::new(RefCell::new(EscapeLog::default()));
        let escape = Box::new(RecordingEscape {
            log: Rc::clone(&escapes),
            fails: escape_fails,
        });
        let session = InputSession::start(Recorder::new(&log, &devices), &config, escape).unwrap();
        Harness {
            session,
            log,
            devices,
            escapes,
        }
    }

    fn pad(path: &str, phys: &str) -> Candidate {
        Candidate {
            path: PathBuf::from(path),
            name: "Microsoft X-Box 360 pad".into(),
            vendor: 0x045e,
            product: 0x028e,
            version: 0x0110,
            bus: 3,
            uniq: None,
            phys: Some(phys.into()),
            has_btn_south: true,
        }
    }

    #[test]
    fn start_creates_one_presenter_per_player_and_registers_their_devnodes() {
        let h = harness(4);
        assert_eq!(h.log.borrow().created, vec![0, 1, 2, 3]);
        let report = h.session.report();
        assert!(report.enabled);
        assert_eq!(report.players, 4);
        assert_eq!(report.presenters.len(), 4);
        assert_eq!(report.presenters[2].name, "tv-shell-player-2");
        assert_eq!(report.presenters[2].devnodes, vec!["/dev/input/event22"]);
    }

    /// **Rule (§7 / jedwards1230/tv-shell#402): a physical unplug and replug does
    /// NOT destroy or recreate a presenter.**
    ///
    /// The acceptance test for this whole PR, at the level where the decision is
    /// made. A create or destroy on the pad's lifecycle is a hotplug event every
    /// game and Moonlight forwards to the streaming host. The presenter is
    /// created exactly `players` times, all before any pad is looked at, and a
    /// pad cycling in and out does not add one.
    #[test]
    fn a_pad_unplug_and_replug_never_touches_a_presenter() {
        let mut h = harness(2);
        let creations_after_start = h.log.borrow().created.len();
        assert_eq!(creations_after_start, 2);

        let p = pad("/dev/input/event3", "port-a");
        *h.devices.borrow_mut() = vec![p.clone()];
        assert_eq!(h.session.poll().len(), 1);

        // Unplug: the enumeration no longer lists it.
        h.devices.borrow_mut().clear();
        assert!(h.session.poll().is_empty());
        assert!(h.session.report().pads.is_empty());

        // Replug, on a different devnode as a real replug commonly is.
        *h.devices.borrow_mut() = vec![pad("/dev/input/event7", "port-a")];
        assert_eq!(h.session.poll().len(), 1);

        assert_eq!(
            h.log.borrow().created.len(),
            creations_after_start,
            "a pad cycling must not create a presenter"
        );
        assert_eq!(
            h.session.report().presenters.len(),
            2,
            "and must not destroy one either"
        );
    }

    /// **Rule: an arbitrating session starts `Unknown`, which routes to the APP.**
    ///
    /// The safe default at the one moment it is most load-bearing: a core
    /// restarting under a live game must not take the pad in the window before
    /// the watcher's first decision arrives. Stated at the device layer — the
    /// pad crosses onto its presenter and no key is synthesised — rather than by
    /// reading back the field that was set.
    ///
    /// **This reverses a phase-1 test deliberately.** That test asserted no
    /// keyboard is created without `shell_keys`; phase 2 creates it always,
    /// because the route can now become the shell's at any moment and the one
    /// thing that must never happen is creating the device at that moment
    /// (§7 / jedwards1230/tv-shell#402). The permanence rule is asserted by
    /// `every_permanent_device_is_created_once_in_start` below.
    ///
    /// **Mutation note.** Make the arbitrating branch of `start` choose
    /// `InputOwner::Shell` and both the owner assertion and the "no key was
    /// synthesised" assertion fail.
    #[test]
    fn an_arbitrating_session_starts_routed_to_the_app() {
        let mut h = harness(2);

        let p = pad("/dev/input/event3", "port-a");
        *h.devices.borrow_mut() = vec![p.clone()];
        h.session.poll();
        h.session.forward(&p.path, ev::KEY, btn::SOUTH, 1, t0());

        assert!(
            h.log.borrow().keys.is_empty(),
            "no key may be synthesised while the owner is unknown"
        );
        assert!(
            h.log.borrow().emitted.contains(&(
                0,
                Forward::Key {
                    code: btn::SOUTH,
                    value: 1
                }
            )),
            "and the pad crosses onto its presenter, which is where an unknown owner routes"
        );

        let report = h.session.report();
        assert_eq!(report.owner, InputOwner::Unknown);
        assert_eq!(report.route, Route::App);
    }

    // -----------------------------------------------------------------------
    // Phase 2: the owner decision, applied.
    // -----------------------------------------------------------------------

    const MOONLIGHT: crate::atoms::AppId = crate::atoms::AppId::new(9003);
    const STEAM: crate::atoms::AppId = crate::atoms::AppId::new(769);

    /// A claimed pad on an arbitrating session, holding A.
    fn holding_a() -> (Harness, Candidate) {
        let mut h = harness(2);
        let p = pad("/dev/input/event3", "port-a");
        *h.devices.borrow_mut() = vec![p.clone()];
        h.session.poll();
        h.session.forward(&p.path, ev::KEY, btn::SOUTH, 1, t0());
        (h, p)
    }

    /// **Rule: a transition releases the held button on the target it is
    /// LEAVING — which is only possible if it quiesces BEFORE it switches.**
    ///
    /// This is the ordering assertion, and it is stated as an observable rather
    /// than as a sequence of two lists: a `set_owner` that switched first would
    /// send the release to the KEYBOARD, because the route would already be the
    /// shell's. So "the presenter got the release and the keyboard got nothing"
    /// is exactly "quiesce came first".
    ///
    /// **Mutation note — two of them.** Delete the `quiesce_route` call and the
    /// presenter never sees the release. Move it AFTER the two assignments and
    /// the release lands on the keyboard instead, failing both assertions.
    #[test]
    fn a_transition_quiesces_the_route_it_is_leaving_before_it_switches() {
        let (mut h, _p) = holding_a();
        let before = h.log.borrow().emitted.len();

        assert!(h.session.set_owner(InputOwner::Shell), "the owner changed");

        let log = h.log.borrow();
        assert!(
            log.emitted[before..].contains(&(
                0,
                Forward::Key {
                    code: btn::SOUTH,
                    value: 0
                }
            )),
            "the presenter must be told the button it believes is held has come up"
        );
        assert!(
            log.keys.is_empty(),
            "and nothing may reach the keyboard, which had not been handed the route yet"
        );
    }

    /// **Rule: leaving the SHELL releases the key it is holding.**
    ///
    /// The mirror image, and it matters just as much: the keyboard outlives the
    /// route (§7), so a shell left with a key down navigates in one direction
    /// forever with nothing downstream able to correct it.
    ///
    /// **Mutation note.** Make `quiesce_route` a no-op for `Route::Shell` (keep
    /// only the `App` arm) and the key-up disappears.
    #[test]
    fn leaving_the_shell_releases_the_key_it_is_holding() {
        let (mut h, p) = holding_a();
        h.session.set_owner(InputOwner::Shell);
        h.session.forward(&p.path, ev::KEY, btn::SOUTH, 1, t0());
        assert_eq!(
            *h.log.borrow().keys.last().unwrap(),
            KeyEmit {
                code: keymap::key::ENTER,
                value: 1
            },
            "A is down on the shell route"
        );
        let keys_before = h.log.borrow().keys.len();
        let emitted_before = h.log.borrow().emitted.len();

        assert!(h.session.set_owner(InputOwner::App { id: MOONLIGHT }));

        let log = h.log.borrow();
        assert!(
            log.keys[keys_before..].contains(&KeyEmit {
                code: keymap::key::ENTER,
                value: 0
            }),
            "the shell must be told the key came up"
        );
        assert_eq!(
            log.emitted.len(),
            emitted_before,
            "and the release went to the keyboard, not to the presenter it was switching TO"
        );
    }

    /// **Rule: an owner change with no route change still quiesces.**
    ///
    /// App-to-app is a launch from inside an app, and the button held at the
    /// moment of the switch must not be inherited by the new one. The route is
    /// unchanged, so a `set_owner` that only acted on route changes would do
    /// nothing here — and this is the case where doing nothing is a stuck
    /// button in a game the user just started.
    ///
    /// **Mutation note.** Have `routing::plan` return `None` when the routes
    /// match and the release disappears.
    #[test]
    fn an_app_to_app_switch_still_releases_the_held_button() {
        let (mut h, p) = holding_a();
        // Settle on one app first. That transition consumes the button
        // `holding_a` pressed — correctly — so press it again to be holding one
        // at the moment of the app-to-app switch this test is about.
        h.session.set_owner(InputOwner::App { id: MOONLIGHT });
        h.session.forward(&p.path, ev::KEY, btn::SOUTH, 1, t0());
        let before = h.log.borrow().emitted.len();

        assert!(h.session.set_owner(InputOwner::App { id: STEAM }));

        assert!(
            h.log.borrow().emitted[before..].contains(&(
                0,
                Forward::Key {
                    code: btn::SOUTH,
                    value: 0
                }
            )),
            "the button held across an app switch must not be inherited"
        );
    }

    /// **Rule: an unchanged owner is not a transition.**
    ///
    /// The watcher settles and re-asserts; a `set_owner` that quiesced on every
    /// call would release a button the user is still holding, which is a game
    /// that stops responding while you press it.
    ///
    /// **Mutation note.** Drop the `from == to` guard in `routing::plan` and
    /// this fails on both halves.
    #[test]
    fn re_applying_the_same_owner_changes_nothing() {
        let (mut h, p) = holding_a();
        h.session.set_owner(InputOwner::App { id: MOONLIGHT });
        // Hold a button across the repeated decision, so "nothing was emitted"
        // means "the hold survived" and not merely "nothing was held".
        h.session.forward(&p.path, ev::KEY, btn::SOUTH, 1, t0());
        let before = h.log.borrow().emitted.len();

        assert!(!h.session.set_owner(InputOwner::App { id: MOONLIGHT }));
        assert_eq!(
            h.log.borrow().emitted.len(),
            before,
            "a repeated decision must not release a button the user is holding"
        );
    }

    /// **Rule: no device is created or destroyed by a transition.**
    ///
    /// §7 / jedwards1230/tv-shell#402: create/destroy is a hotplug event every
    /// game and Moonlight forward to the streaming host. Routing changes where
    /// events go and never what devices exist — which is the whole reason the
    /// keyboard is created in `start` even for a session that may never use it.
    ///
    /// **Mutation note.** Have `set_owner` create the keyboard lazily on the
    /// first shell transition and `device_creations` grows.
    #[test]
    fn a_transition_creates_and_destroys_no_device() {
        let (mut h, _p) = holding_a();
        let devices = h.log.borrow().device_creations.clone();
        assert_eq!(devices, vec!["presenter-0", "presenter-1", "keyboard"]);

        for owner in [
            InputOwner::Shell,
            InputOwner::ShellOverlay,
            InputOwner::App { id: MOONLIGHT },
            InputOwner::Unknown,
            InputOwner::Shell,
        ] {
            h.session.set_owner(owner);
        }

        assert_eq!(
            h.log.borrow().device_creations,
            devices,
            "five transitions, and the device list is byte-identical"
        );
        assert!(
            h.log.borrow().released.is_empty(),
            "and no pad was handed back either — routing never re-claims"
        );
    }

    /// **Rule: `shell_keys` PINS the route, and arbitration cannot move it.**
    ///
    /// The operator override kept from phase 1. Someone who set that flag to
    /// take a measurement must not have the route pulled out from under them by
    /// whatever happens to be on screen.
    ///
    /// **Mutation note.** Delete the `self.pinned` guard in `set_owner` and the
    /// route follows the decision.
    #[test]
    fn a_pinned_session_refuses_every_owner_decision() {
        let mut h = shell_harness(2);
        assert_eq!(h.session.report().owner, InputOwner::Shell);

        assert!(!h.session.set_owner(InputOwner::App { id: MOONLIGHT }));

        let report = h.session.report();
        assert_eq!(report.owner, InputOwner::Shell);
        assert_eq!(report.route, Route::Shell);
    }

    /// **Rule: the route a transition switched TO is where the next event
    /// goes.**
    ///
    /// The point of the whole phase, and the symptom it fixes: measured on
    /// hardware, a held Guide returned the screen to the shell and the home
    /// screen was completely inert, because the pad kept forwarding to the app.
    /// Asserted at the device layer — a pad press becomes a KEY on the keyboard
    /// and reaches no presenter — rather than by reading back `report.route`.
    ///
    /// **Mutation note.** Drop the `self.route = transition.route` assignment
    /// and the press still goes to the presenter.
    #[test]
    fn after_a_transition_the_pad_drives_the_new_target() {
        let (mut h, p) = holding_a();
        h.session.set_owner(InputOwner::Shell);
        let emitted_before = h.log.borrow().emitted.len();

        h.session.forward(&p.path, ev::KEY, btn::EAST, 1, t0());

        let log = h.log.borrow();
        assert_eq!(
            log.keys.last(),
            Some(&KeyEmit {
                code: keymap::key::ESC,
                value: 1
            }),
            "B goes back, as a key, to the shell"
        );
        assert_eq!(
            log.emitted.len(),
            emitted_before,
            "and nothing reached the presenter, so a game behind the shell sees a still pad"
        );
    }

    /// **Rule (§7 / jedwards1230/tv-shell#402, extended to the keyboard): every
    /// permanent device is created in `start` and NOWHERE else.**
    ///
    /// The keyboard is under the same permanence rule as the presenters, for the
    /// same reason: a device that appeared or vanished on a route change, a pad
    /// joining, or a stick repeat is a hotplug event apps forward to the
    /// streaming host. This drives a full pad lifecycle — join, input, repeat,
    /// unplug, replug — through the real entry points and asserts the ordered
    /// creation log is untouched by all of it.
    ///
    /// **Mutation note.** Move `create_keyboard` into `forward` (lazily, "only
    /// when a key is first needed" — the intuitive implementation) and this
    /// fails.
    #[test]
    fn the_keyboard_is_created_once_in_start_and_nothing_else_creates_a_device() {
        let mut h = shell_harness(2);
        let after_start = h.log.borrow().device_creations.clone();
        assert_eq!(
            after_start,
            vec!["presenter-0", "presenter-1", "keyboard"],
            "the keyboard is created in start, beside the presenters"
        );

        let p = pad("/dev/input/event3", "port-a");
        *h.devices.borrow_mut() = vec![p.clone()];
        h.session.poll();
        h.session.forward(&p.path, ev::KEY, btn::SOUTH, 1, t0());
        h.session.forward(&p.path, ev::KEY, btn::SOUTH, 0, t0());
        h.session.forward(&p.path, ev::ABS, abs::X, 32767, t0());
        h.session
            .tick(std::time::Instant::now() + std::time::Duration::from_secs(1));
        h.devices.borrow_mut().clear();
        h.session.poll();
        *h.devices.borrow_mut() = vec![pad("/dev/input/event7", "port-a")];
        h.session.poll();
        h.session.shutdown();

        assert_eq!(
            h.log.borrow().device_creations,
            after_start,
            "a device was created after start"
        );
    }

    /// **Rule: on the shell route a pad event becomes a KEY, and the presenter
    /// receives nothing.**
    ///
    /// Both halves matter. If the presenter also got the event, a game behind
    /// the shell would see the user pressing buttons in it while they navigate
    /// the shell — which is the leak the whole routing design exists to prevent.
    ///
    /// **Mutation note.** Drop the `return` after the keymap in `forward` and
    /// the presenter half fails; drop the keymap call and the key half does.
    #[test]
    fn on_the_shell_route_a_button_becomes_a_key_and_the_presenter_gets_nothing() {
        let mut h = shell_harness(2);
        let p = pad("/dev/input/event3", "port-a");
        *h.devices.borrow_mut() = vec![p.clone()];
        h.session.poll();

        h.session.forward(&p.path, ev::KEY, btn::SOUTH, 1, t0());
        h.session.forward(&p.path, ev::KEY, btn::SOUTH, 0, t0());

        let log = h.log.borrow();
        assert_eq!(
            log.keys,
            vec![
                KeyEmit {
                    code: keymap::key::ENTER,
                    value: 1
                },
                KeyEmit {
                    code: keymap::key::ENTER,
                    value: 0
                },
            ]
        );
        assert!(
            log.emitted.is_empty(),
            "nothing may cross onto the presenter while the shell owns the pad: {:?}",
            log.emitted
        );
    }

    /// **Rule: a stick held on the shell route auto-repeats on v1's timing, and
    /// the runtime is told when to come back.**
    ///
    /// Driven entirely through the real entry points — `forward` for the
    /// deflection, `next_deadline` for the wake-up the runtime sleeps on,
    /// `tick` for the repeat — because a test that armed the latch directly
    /// would prove the timer and not the path to it.
    ///
    /// **Mutation note.** Make `next_deadline` return `None` and the runtime
    /// would never wake; this fails on the deadline assertion rather than
    /// silently still passing on `tick`.
    #[test]
    fn a_stick_held_on_the_shell_route_repeats_and_schedules_its_own_wake_up() {
        let mut h = shell_harness(2);
        let p = pad("/dev/input/event3", "port-a");
        *h.devices.borrow_mut() = vec![p.clone()];
        h.session.poll();

        assert_eq!(
            h.session.next_deadline(),
            None,
            "an idle session must not wake the runtime at all"
        );

        let t = std::time::Instant::now();
        h.session.forward(&p.path, ev::ABS, abs::X, 32767, t0());
        let due = h
            .session
            .next_deadline()
            .expect("a deflected stick arms a repeat");
        assert!(
            due >= t + keymap::repeat::INITIAL_DELAY,
            "the first repeat must wait v1's calibrated initial delay"
        );

        h.session.tick(due);
        assert_eq!(
            h.log.borrow().keys,
            vec![
                KeyEmit {
                    code: keymap::key::RIGHT,
                    value: 1
                },
                KeyEmit {
                    code: keymap::key::RIGHT,
                    value: 0
                },
                KeyEmit {
                    code: keymap::key::RIGHT,
                    value: 1
                },
            ],
            "press, then a release/press repeat"
        );
    }

    /// **Rule: a pad that leaves mid-press releases its KEYS too, not only its
    /// presenter buttons.**
    ///
    /// The keyboard outlives the pad exactly as the presenter does, so the same
    /// stuck-input failure applies — and it is worse here, because a stuck arrow
    /// key means a shell that scrolls forever with no device left to blame.
    ///
    /// **Mutation note.** Delete the `keymaps.remove(...)` quiesce in `retire`
    /// and this fails.
    #[test]
    fn a_pad_that_leaves_releases_the_keys_it_was_holding() {
        let mut h = shell_harness(2);
        let p = pad("/dev/input/event3", "port-a");
        *h.devices.borrow_mut() = vec![p.clone()];
        h.session.poll();

        h.session.forward(&p.path, ev::KEY, btn::SOUTH, 1, t0());
        h.session.forward(&p.path, ev::ABS, abs::X, 32767, t0());
        h.log.borrow_mut().keys.clear();

        // Unplug.
        h.devices.borrow_mut().clear();
        h.session.poll();

        let mut released: Vec<u16> = h
            .log
            .borrow()
            .keys
            .iter()
            .inspect(|e| assert_eq!(e.value, 0, "a leave may only RELEASE"))
            .map(|e| e.code)
            .collect();
        released.sort_unstable();
        assert_eq!(released, vec![keymap::key::ENTER, keymap::key::RIGHT]);

        // And the repeat is disarmed with it, or the runtime wakes forever for a
        // pad that is gone.
        assert_eq!(h.session.next_deadline(), None);
    }

    /// **Rule: a keyboard the backend created without a devnode fails the
    /// start.**
    ///
    /// Same reasoning as the presenter case: a devnode we never saw is one
    /// discovery cannot be taught to skip. It also means a session that came up
    /// on the shell route with no keyboard — every key silently going nowhere —
    /// is unrepresentable.
    #[test]
    fn a_keyboard_without_a_devnode_fails_the_start() {
        let log = Rc::new(RefCell::new(Log::default()));
        let devices = Rc::new(RefCell::new(Vec::new()));
        let mut recorder = Recorder::new(&log, &devices);
        recorder.keyboard_without_devnode = true;
        let started = InputSession::start(
            recorder,
            &ResolvedInput {
                shell_keys: true,
                ..config(2)
            },
            never_fires(),
        );
        assert!(matches!(started, Err(InputError::Keyboard(_))));
    }

    /// **Rule: a key the keyboard refuses is COUNTED, not silent.**
    ///
    /// The same reasoning as `emit_failures` for the presenters: a refused key
    /// is one nothing downstream can correct, and a refused *release* leaves the
    /// shell holding it.
    ///
    /// **Mutation note.** Drop the increment in `emit_key` and this fails.
    #[test]
    fn a_refused_key_is_counted() {
        let log = Rc::new(RefCell::new(Log::default()));
        let devices = Rc::new(RefCell::new(Vec::new()));
        let mut recorder = Recorder::new(&log, &devices);
        recorder.emit_fails = true;
        let mut session = InputSession::start(
            recorder,
            &ResolvedInput {
                shell_keys: true,
                ..config(2)
            },
            never_fires(),
        )
        .unwrap();

        let p = pad("/dev/input/event3", "port-a");
        *devices.borrow_mut() = vec![p.clone()];
        session.poll();
        assert_eq!(session.report().emit_failures, 0);
        session.forward(&p.path, ev::KEY, btn::SOUTH, 1, t0());
        assert_eq!(session.report().emit_failures, 1);
    }

    /// The report says which route the core chose and names the keyboard, so a
    /// hardware session READS the decision instead of inferring it from
    /// behaviour (V2_GAMEPAD_HANDOFF §6).
    #[test]
    fn the_report_carries_the_owner_route_and_keyboard() {
        let h = shell_harness(2);
        let report = h.session.report();
        assert_eq!(report.owner, InputOwner::Shell);
        assert_eq!(report.route, Route::Shell);
        let keyboard = report.keyboard.expect("the shell route names its keyboard");
        assert_eq!(keyboard.name, "tv-shell-keys");
        assert_eq!(keyboard.devnodes, vec!["/dev/input/event30"]);
        // Phase 1 masks nothing, and says so rather than omitting the fields.
        assert!(report.masked_keys.is_empty());
        assert!(report.masked_axes.is_empty());
    }
    /// **Rule: a leave returns the presenter to rest BEFORE releasing the pad.**
    ///
    /// The presenter outlives the pad, so a button held at unplug would stay
    /// held forever with nothing able to notice.
    #[test]
    fn a_leave_quiesces_the_presenter_then_releases_the_pad() {
        let mut h = harness(2);
        let p = pad("/dev/input/event3", "port-a");
        *h.devices.borrow_mut() = vec![p.clone()];
        h.session.poll();

        // Press and hold A, then yank the pad.
        h.session.forward(&p.path, ev::KEY, btn::SOUTH, 1, t0());
        h.log.borrow_mut().emitted.clear();
        h.devices.borrow_mut().clear();
        h.session.poll();

        let log = h.log.borrow();
        assert!(
            log.emitted.contains(&(
                0,
                Forward::Key {
                    code: btn::SOUTH,
                    value: 0
                }
            )),
            "the held button must be released on the presenter: {:?}",
            log.emitted
        );
        assert_eq!(
            log.emitted.last(),
            Some(&(0, Forward::Sync)),
            "and the reset must be flushed"
        );
        assert_eq!(log.released, vec![p.path.clone()]);

        // ORDER, not merely occurrence: the release must come after the whole
        // quiesce. Two separate lists record that both happened but never in
        // which sequence, and the sequence is the rule — releasing first widens
        // the window in which the pad is free and the presenter still holds a
        // button.
        assert_eq!(
            log.released_at_emit_count,
            vec![log.emitted.len()],
            "the pad was released before the quiesce finished"
        );
    }

    /// **Rule: an advertised event crosses to the right slot's presenter,
    /// unchanged.**
    #[test]
    fn events_are_forwarded_one_to_one_to_the_pads_own_slot() {
        let mut h = harness(2);
        let p1 = pad("/dev/input/event3", "a");
        let p2 = pad("/dev/input/event4", "b");
        *h.devices.borrow_mut() = vec![p1.clone(), p2.clone()];
        h.session.poll();
        h.log.borrow_mut().emitted.clear();

        h.session.forward(&p2.path, ev::KEY, btn::START, 1, t0());
        h.session.forward(&p2.path, ev::SYN, SYN_REPORT, 0, t0());
        h.session.forward(&p1.path, ev::ABS, abs::X, -32768, t0());

        assert_eq!(
            h.log.borrow().emitted,
            vec![
                (
                    1,
                    Forward::Key {
                        code: btn::START,
                        value: 1
                    }
                ),
                (1, Forward::Sync),
                (
                    0,
                    Forward::Abs {
                        code: abs::X,
                        value: -32768
                    }
                ),
            ]
        );
    }

    /// **Rule: an event from a pad the fleet no longer holds emits nothing.**
    ///
    /// A stream drains after a retire. Routing those late events by the slot
    /// they used to occupy would inject one player's input into another's
    /// presenter the moment the slot is reused.
    #[test]
    fn events_from_a_retired_pad_are_dropped_not_routed_to_its_old_slot() {
        let mut h = harness(2);
        let p = pad("/dev/input/event3", "a");
        *h.devices.borrow_mut() = vec![p.clone()];
        h.session.poll();
        h.devices.borrow_mut().clear();
        h.session.poll();
        h.log.borrow_mut().emitted.clear();

        h.session.forward(&p.path, ev::KEY, btn::SOUTH, 1, t0());
        assert!(h.log.borrow().emitted.is_empty());
    }

    /// **Rule: a presenter that refuses an event is counted, not just logged.**
    ///
    /// `retire` documents that it returns the presenter to rest. If those emits
    /// fail, that claim is false and **nothing downstream can notice**: the pad
    /// is gone, so no later event corrects the stuck button, and from a game's
    /// side no device disconnected. A log line alone leaves the only evidence in
    /// a journal nobody is reading at the time. The count makes it a number on
    /// `input-state`.
    #[test]
    fn a_presenter_that_refuses_events_is_counted() {
        let mut h = harness(2);
        let p = pad("/dev/input/event3", "a");
        *h.devices.borrow_mut() = vec![p.clone()];
        h.session.poll();
        assert_eq!(h.session.report().emit_failures, 0);

        h.session.backend.emit_fails = true;

        // A live passthrough event the device refuses.
        h.session.forward(&p.path, ev::KEY, btn::SOUTH, 1, t0());
        assert_eq!(h.session.report().emit_failures, 1);

        // And a whole quiesce that cannot land: every release and axis reset,
        // plus the sync. The pad still leaves — holding it would be worse — but
        // the incomplete reset is now visible rather than silent.
        h.devices.borrow_mut().clear();
        h.session.poll();
        let failures = h.session.report().emit_failures;
        assert!(
            failures > 1,
            "a failed quiesce must be counted, got {failures}"
        );
        assert!(h.session.report().pads.is_empty(), "the pad still leaves");
        assert_eq!(h.log.borrow().released, vec![p.path.clone()]);
    }

    /// **Rule: dropped events are counted, per reason.**
    #[test]
    fn drops_are_counted_and_reported() {
        let mut h = harness(2);
        let p = pad("/dev/input/event3", "a");
        *h.devices.borrow_mut() = vec![p.clone()];
        h.session.poll();

        // BTN_TOUCH twice, and one unadvertised axis.
        h.session.forward(&p.path, ev::KEY, 0x14a, 1, t0());
        h.session.forward(&p.path, ev::KEY, 0x14a, 0, t0());
        h.session.forward(&p.path, ev::ABS, 0x12, 1, t0());

        let drops = h.session.report().drops;
        assert_eq!(drops.get(&DropReason::UnadvertisedKey), Some(&2));
        assert_eq!(drops.get(&DropReason::UnadvertisedAxis), Some(&1));
    }

    /// **Rule: a refused device is reported with its reason.**
    #[test]
    fn refusals_are_reported_with_a_reason_and_an_explanation() {
        let mut h = harness(2);
        *h.devices.borrow_mut() = vec![Candidate {
            vendor: 0,
            product: 0,
            name: "ydotoold virtual device".into(),
            ..pad("/dev/input/event9", "")
        }];
        h.session.poll();

        let report = h.session.report();
        assert!(
            report.pads.is_empty(),
            "an unknown injector must not be claimed"
        );
        assert_eq!(report.refused.len(), 1);
        assert_eq!(report.refused[0].reason, Refusal::NotInTheControllerDb);
        assert!(!report.refused[0].explanation.is_empty());
        assert_eq!(h.log.borrow().claimed.len(), 0, "and never even opened");
    }

    /// **Rule: the session never claims its own presenters.**
    ///
    /// The presenter carries a database-known id on purpose, so only devnode
    /// ownership stops the session grabbing the device it just created and
    /// feeding its own output back into itself. Here the enumeration lists the
    /// presenters exactly as `/dev/input` would.
    #[test]
    fn the_session_never_claims_its_own_presenters() {
        let mut h = harness(2);
        *h.devices.borrow_mut() = vec![
            pad("/dev/input/event20", "virtual-0"),
            pad("/dev/input/event21", "virtual-1"),
        ];
        h.session.poll();

        let report = h.session.report();
        assert!(report.pads.is_empty());
        assert_eq!(h.log.borrow().claimed.len(), 0);
        assert_eq!(report.refused.len(), 2);
        assert!(report
            .refused
            .iter()
            .all(|r| r.reason == Refusal::OurOwnPresenter));
    }

    /// **Rule: a pad we grabbed but cannot seat is given straight back.**
    ///
    /// Holding an exclusive grab on a pad we will never present is strictly
    /// worse than not claiming it: the pad is then dead to the game too.
    #[test]
    fn a_pad_beyond_capacity_is_released_not_held() {
        let mut h = harness(1);
        *h.devices.borrow_mut() =
            vec![pad("/dev/input/event3", "a"), pad("/dev/input/event4", "b")];
        let joined = h.session.poll();

        assert_eq!(joined.len(), 1, "only one slot exists");
        assert_eq!(h.session.report().pads.len(), 1);
        assert_eq!(
            h.log.borrow().released,
            vec![PathBuf::from("/dev/input/event4")],
            "the unseatable pad must be released, not held"
        );
    }

    /// A pad that fails to open is simply not in the fleet, and stays a
    /// candidate — the right behaviour for a device still settling.
    #[test]
    fn a_pad_that_fails_to_open_is_retried_rather_than_seated() {
        let log = Rc::new(RefCell::new(Log::default()));
        let devices = Rc::new(RefCell::new(vec![pad("/dev/input/event3", "a")]));
        let mut backend = Recorder::new(&log, &devices);
        backend.claim_fails = BTreeSet::from([PathBuf::from("/dev/input/event3")]);
        let mut session = InputSession::start(backend, &config(2), never_fires()).unwrap();

        assert!(session.poll().is_empty());
        assert!(session.report().pads.is_empty());
        // Still a claim candidate next time round.
        assert!(session.poll().is_empty());
        assert!(session.report().pads.is_empty());
    }

    /// **Rule: a presenter with no devnode is a fatal start, not a warning.**
    ///
    /// Without a devnode the session cannot register it as ours, so discovery
    /// would grab it on the very next poll.
    #[test]
    fn a_presenter_whose_devnode_never_appeared_fails_the_start() {
        let log = Rc::new(RefCell::new(Log::default()));
        let devices = Rc::new(RefCell::new(Vec::new()));
        let mut backend = Recorder::new(&log, &devices);
        backend.presenter_without_devnode = true;
        // `unwrap_err` would need `InputSession: Debug`, and so `Debug` on every
        // backend. Match instead.
        let err = match InputSession::start(backend, &config(2), never_fires()) {
            Ok(_) => panic!("a presenter with no devnode must not yield a session"),
            Err(e) => e,
        };
        assert!(
            matches!(err, InputError::Presenter { slot: 0, .. }),
            "{err}"
        );
    }

    /// A stream error retires the pad immediately, without waiting for a poll.
    #[test]
    fn a_stream_error_retires_the_pad() {
        let mut h = harness(2);
        let p = pad("/dev/input/event3", "a");
        *h.devices.borrow_mut() = vec![p.clone()];
        h.session.poll();

        h.session.on_stream_error(&p.path);
        assert!(h.session.report().pads.is_empty());
        assert_eq!(h.log.borrow().released, vec![p.path.clone()]);
    }

    /// **Rule: a failed enumeration changes nothing.**
    ///
    /// "We could not read the device list" is not evidence that every pad was
    /// unplugged. Treating it as one would quiesce and release a fleet that is
    /// still physically present — mid-game.
    #[test]
    fn a_failed_enumeration_does_not_retire_the_fleet() {
        let log = Rc::new(RefCell::new(Log::default()));
        let devices = Rc::new(RefCell::new(vec![pad("/dev/input/event3", "a")]));
        let mut session =
            InputSession::start(Recorder::new(&log, &devices), &config(2), never_fires()).unwrap();
        assert_eq!(session.poll().len(), 1);

        session.backend.enumerate_fails = true;
        session.poll();

        assert_eq!(session.report().pads.len(), 1, "the fleet must survive");
        assert!(log.borrow().released.is_empty(), "nothing may be released");
    }

    /// Shutting down releases every pad and leaves its presenter at rest.
    #[test]
    fn shutdown_releases_every_pad() {
        let mut h = harness(2);
        let p1 = pad("/dev/input/event3", "a");
        let p2 = pad("/dev/input/event4", "b");
        *h.devices.borrow_mut() = vec![p1.clone(), p2.clone()];
        h.session.poll();

        h.session.shutdown();
        assert!(h.session.report().pads.is_empty());
        let released = h.log.borrow().released.clone();
        assert!(released.contains(&p1.path) && released.contains(&p2.path));
    }

    #[test]
    fn the_disabled_report_is_structurally_empty() {
        let r = InputReport::disabled();
        assert!(!r.enabled);
        assert_eq!(r.players, 0);
        assert!(r.presenters.is_empty() && r.pads.is_empty() && r.refused.is_empty());
        assert!(r.drops.is_empty());
        assert_eq!(r.emit_failures, 0);
        assert_eq!(r.last_poll_unix_ms, None);
    }

    /// **Rule: every completed poll stamps the report.**
    ///
    /// A snapshot-based `input-state` cannot hang, but it also cannot show that
    /// the loop behind it has stopped. The timestamp is what makes a dead loop
    /// visible instead of merely stale-looking.
    #[test]
    fn a_completed_poll_stamps_the_report() {
        let mut h = harness(2);
        let before = h.session.report();
        assert_eq!(before.last_poll_unix_ms, None, "before any poll");
        assert_eq!(before.polls_completed, 0);

        h.session.poll();
        let after = h.session.report();
        assert!(after.last_poll_unix_ms.is_some_and(|ms| ms > 0));
        assert_eq!(after.polls_completed, 1);

        // A poll whose enumeration FAILED must not count: the whole point is to
        // distinguish "we ran" from "we did not".
        //
        // Asserted on the COUNTER, not the timestamp. Two polls in the same
        // millisecond carry the same stamp, so a timestamp assertion here holds
        // whether or not the failing poll stamped — which is exactly how the
        // first version of this test passed against a `poll` mutated to stamp
        // unconditionally at its top.
        h.session.backend.enumerate_fails = true;
        h.session.poll();
        assert_eq!(
            h.session.report().polls_completed,
            1,
            "a failed enumeration is not a completed poll"
        );

        h.session.backend.enumerate_fails = false;
        h.session.poll();
        assert_eq!(h.session.report().polls_completed, 2);
    }

    // ---- the Guide escape (jedwards1230/tv-shell#496) ----------------------
    //
    // Every test below reaches the escape the only way the real core does:
    // `forward` with an `EV_KEY`/`BTN_MODE` event, then `tick` with an instant.
    // Nothing pokes a `GuideWatch` or a counter directly, so no assertion here
    // is about a state the pad cannot produce.

    /// Claim one pad and hand back its path, on the given harness.
    fn one_pad(h: &mut Harness, path: &str, phys: &str) -> PathBuf {
        let p = pad(path, phys);
        h.devices.borrow_mut().push(p.clone());
        h.session.poll();
        p.path
    }

    /// A little past the hold threshold, from `t`.
    fn past(t: std::time::Instant) -> std::time::Instant {
        t + HOLD + std::time::Duration::from_millis(1)
    }

    /// **Rule: a held Guide writes the base layer, and touches the shell in no
    /// way at all.**
    ///
    /// The acceptance for jedwards1230/tv-shell#496 at the level where the
    /// decision is made. v1 delivered this escape as a message the shell acted
    /// on, and it failed whenever the shell was wedged — so the assertion is not
    /// only "a fire happened" but that the fire needed nothing else: no key on
    /// the shell keyboard, no event onto a presenter.
    ///
    /// **Mutation note.** Route the fire through `emit_key` (a "tell the shell"
    /// implementation) and the touched-nothing assertions fail while the fire
    /// count still passes — which is why both are asserted.
    #[test]
    fn a_held_guide_writes_the_base_layer_and_touches_the_shell_in_no_way() {
        let mut h = shell_harness(2);
        let path = one_pad(&mut h, "/dev/input/event3", "port-a");
        let keys_before = h.log.borrow().keys.len();
        let emits_before = h.log.borrow().emitted.len();

        let t = t0();
        h.session.forward(&path, ev::KEY, btn::MODE, 1, t);
        h.session.tick(past(t));

        assert_eq!(h.escapes.borrow().fires, 1, "the hold escaped");
        assert_eq!(
            h.log.borrow().keys.len(),
            keys_before,
            "the escape must not need the shell keyboard"
        );
        assert_eq!(
            h.log.borrow().emitted.len(),
            emits_before,
            "nor a presenter — Guide never crosses"
        );
        let report = h.session.report();
        assert_eq!(report.escape.fires, 1);
        assert_eq!(report.escape.failures, 0);
        assert!(report.escape.last_fire_unix_ms.is_some_and(|ms| ms > 0));
    }

    /// **Rule: a TAP is not an escape — it goes to whatever is on screen.**
    ///
    /// v1 let a Guide tap through to the game and reserved the hold for the
    /// escape; escaping on a tap would end a game every time the button was
    /// brushed. On the app route the tap is a real Guide press+release on the
    /// presenter (so Steam Big Picture still opens); on the shell route it is
    /// the drawer key, which is Tab — the code measured to arrive.
    ///
    /// **Mutation note.** Fire on press (a zero threshold), or on release
    /// regardless of elapsed time, and the `fires` assertions fail. Drop the
    /// replay and the presenter/keyboard assertions do.
    #[test]
    fn a_guide_tap_reaches_the_target_and_escapes_nothing() {
        // The app route: a real Guide, press then release, onto the presenter.
        let mut h = harness(2);
        let path = one_pad(&mut h, "/dev/input/event3", "port-a");
        let before = h.log.borrow().emitted.len();
        let t = t0();
        h.session.forward(&path, ev::KEY, btn::MODE, 1, t);
        assert_eq!(
            h.log.borrow().emitted.len(),
            before,
            "the press is BUFFERED — nothing may cross until the gesture is known"
        );
        h.session
            .forward(&path, ev::KEY, btn::MODE, 0, t + HOLD.mul_f64(0.5));
        assert_eq!(h.escapes.borrow().fires, 0, "a tap escapes nothing");
        let tail: Vec<(u8, Forward)> = h.log.borrow().emitted[before..].to_vec();
        assert_eq!(
            tail,
            vec![
                (
                    0,
                    Forward::Key {
                        code: btn::MODE,
                        value: 1
                    }
                ),
                (
                    0,
                    Forward::Key {
                        code: btn::MODE,
                        value: 0
                    }
                ),
            ]
        );

        // The shell route: the drawer.
        let mut h = shell_harness(2);
        let path = one_pad(&mut h, "/dev/input/event3", "port-a");
        let t = t0();
        h.session.forward(&path, ev::KEY, btn::MODE, 1, t);
        h.session
            .forward(&path, ev::KEY, btn::MODE, 0, t + HOLD.mul_f64(0.5));
        assert_eq!(h.escapes.borrow().fires, 0);
        assert_eq!(
            h.log.borrow().keys.clone(),
            vec![
                KeyEmit {
                    code: keymap::key::TAB,
                    value: 1
                },
                KeyEmit {
                    code: keymap::key::TAB,
                    value: 0
                },
            ]
        );
    }

    /// **Rule: a fired hold's release is swallowed — it never reaches the
    /// target.**
    ///
    /// The target never saw the press (it is buffered), so a release would be
    /// the only edge it ever got. That is jedwards1230/tv-shell#295's shape, in
    /// the one direction this phase can produce.
    ///
    /// **Mutation note.** Replay the tap unconditionally on release and this
    /// fails.
    #[test]
    fn a_fired_holds_release_reaches_nothing() {
        let mut h = harness(2);
        let path = one_pad(&mut h, "/dev/input/event3", "port-a");
        let t = t0();
        h.session.forward(&path, ev::KEY, btn::MODE, 1, t);
        h.session.tick(past(t));
        let after_fire = h.log.borrow().emitted.len();

        h.session
            .forward(&path, ev::KEY, btn::MODE, 0, t + HOLD + HOLD);
        assert_eq!(
            h.log.borrow().emitted.len(),
            after_fire,
            "the release of a fired hold must not leak onto the presenter"
        );
        assert_eq!(h.escapes.borrow().fires, 1, "and it did not fire again");
    }

    /// **Rule: two pads each holding HALF the gesture never complete it between
    /// them.**
    ///
    /// v1's per-pad-complete rule (`docs/V2_GAMEPAD_HANDOFF.md` §5), ported. Pad
    /// A holds Guide for part of the threshold and lets go; pad B picks it up
    /// for the rest. Neither pad held it long enough, so nothing escapes — a
    /// fleet-level timer would fire here, which is exactly the bug the rule
    /// prevents.
    ///
    /// **Mutation note.** Keep the hold state on the session instead of per pad
    /// — one `pressed_at`, set by any pad — and this fails while every
    /// single-pad test above still passes.
    #[test]
    fn two_pads_each_holding_half_the_gesture_never_complete_it() {
        let mut h = harness(2);
        let a = one_pad(&mut h, "/dev/input/event3", "port-a");
        let b = one_pad(&mut h, "/dev/input/event4", "port-b");

        let t = t0();
        let handover = t + HOLD.mul_f64(0.6);
        // A holds for 60% of the threshold. B takes over, overlapping by a
        // moment — which is what a real handover between two people looks like,
        // and the ordering under which a fleet-level timer misfires.
        h.session.forward(&a, ev::KEY, btn::MODE, 1, t);
        h.session.tick(handover);
        h.session.forward(&b, ev::KEY, btn::MODE, 1, handover);
        h.session.forward(
            &a,
            ev::KEY,
            btn::MODE,
            0,
            handover + std::time::Duration::from_millis(1),
        );
        h.session.tick(past(t));

        assert_eq!(
            h.escapes.borrow().fires,
            0,
            "no single pad held Guide for the threshold, so nothing may escape"
        );

        // The probe is live: B holding it out on its OWN does escape, so the
        // assertion above is not passing because the escape is simply broken.
        h.session.tick(past(handover));
        assert_eq!(h.escapes.borrow().fires, 1);
    }

    /// **Rule: two pads holding Guide together escape ONCE.**
    ///
    /// v1's fleet-level dedup latch (`home_hold_active`), ported. Two `home`
    /// writes say the same thing; the second is a switch to where the box
    /// already is.
    ///
    /// **Mutation note.** Drop the latch and the first count becomes 2; stop
    /// clearing it and the second hold never fires, so the two halves pin
    /// opposite mistakes. (Clearing on `armed` instead of `holding` is NOT
    /// killed here, and that is recorded rather than papered over — see
    /// `GuideWatch::holding`: at today's call sites the two are equivalent.)
    #[test]
    fn two_pads_holding_together_escape_once() {
        let mut h = harness(2);
        let a = one_pad(&mut h, "/dev/input/event3", "port-a");
        let b = one_pad(&mut h, "/dev/input/event4", "port-b");

        let t = t0();
        h.session.forward(&a, ev::KEY, btn::MODE, 1, t);
        h.session.forward(&b, ev::KEY, btn::MODE, 1, t);
        h.session.tick(past(t));
        assert_eq!(h.escapes.borrow().fires, 1);

        // Both let go, and the latch clears — a SECOND deliberate hold must
        // still work, or the escape is a once-per-boot affair.
        let later = t + std::time::Duration::from_secs(10);
        h.session.forward(&a, ev::KEY, btn::MODE, 0, later);
        h.session.forward(&b, ev::KEY, btn::MODE, 0, later);
        h.session.forward(&a, ev::KEY, btn::MODE, 1, later);
        h.session.tick(past(later));
        assert_eq!(h.escapes.borrow().fires, 2);
    }

    /// **Rule: an escape that fired and could NOT be delivered is a number, not
    /// a silence.**
    ///
    /// The one failure mode that is invisible from the couch: the user held the
    /// button, the core agreed, and the screen did not change. `input-state`
    /// has to say which of the three it was.
    ///
    /// **Mutation note.** Ignore the sink's `Err` (or count it as a fire) and
    /// this fails on both numbers.
    #[test]
    fn an_escape_that_could_not_be_delivered_is_counted() {
        let mut h = harness_with_escape(config(2), true);
        let path = one_pad(&mut h, "/dev/input/event3", "port-a");
        let t = t0();
        h.session.forward(&path, ev::KEY, btn::MODE, 1, t);
        h.session.tick(past(t));

        assert_eq!(h.escapes.borrow().fires, 1, "the sink was asked");
        let report = h.session.report();
        assert_eq!(report.escape.fires, 0, "but nothing was delivered");
        assert_eq!(report.escape.failures, 1);
        assert_eq!(report.escape.last_fire_unix_ms, None);
    }

    /// **Rule: `armed` is readable WHILE the hold is in progress.**
    ///
    /// It exists to tell "the core never saw your press" apart from "the core
    /// saw it and the write did not take", and a flag only true after the fact
    /// answers neither.
    ///
    /// **Mutation note.** Report `armed` from a pad's `holding` instead of its
    /// `armed` and the post-fire assertion fails; hard-code it `false` and the
    /// mid-hold one does.
    #[test]
    fn armed_is_readable_during_the_hold() {
        let mut h = shell_harness(2);
        let path = one_pad(&mut h, "/dev/input/event3", "port-a");
        assert!(!h.session.report().escape.armed);

        let t = t0();
        h.session.forward(&path, ev::KEY, btn::MODE, 1, t);
        let mid = h.session.report();
        assert!(mid.escape.armed, "the press is in, the threshold is not");
        assert_eq!(mid.escape.fires, 0);
        assert_eq!(mid.escape.hold_ms, super::super::escape::DEFAULT_HOLD_MS);

        h.session.tick(past(t));
        let after = h.session.report();
        assert!(!after.escape.armed, "it is no longer waiting to fire");
        assert_eq!(after.escape.fires, 1);
    }

    /// **Rule: the threshold is the CONFIGURED one, and short of it nothing
    /// fires.**
    ///
    /// **Mutation note.** Fire at `pressed_at` rather than `pressed_at + hold`
    /// and the first assertion fails; read the hold from a constant rather than
    /// from the config and the longer-threshold half does.
    #[test]
    fn the_threshold_comes_from_the_config() {
        let long = std::time::Duration::from_millis(1_500);
        let mut h = harness_with(ResolvedInput {
            guide_hold: long,
            ..config(2)
        });
        let path = one_pad(&mut h, "/dev/input/event3", "port-a");
        let t = t0();
        h.session.forward(&path, ev::KEY, btn::MODE, 1, t);

        // Past the DEFAULT threshold, short of the configured one.
        h.session.tick(past(t));
        assert_eq!(h.escapes.borrow().fires, 0);
        assert_eq!(h.session.next_deadline(), Some(t + long));

        h.session.tick(t + long);
        assert_eq!(h.escapes.borrow().fires, 1);
    }

    /// **Rule: a pad yanked mid-hold does not leave the escape latched.**
    ///
    /// The latch clears when no pad is holding, and an unplugged pad holds
    /// nothing. Without the removal on retire, a controller whose battery died
    /// mid-hold would disable the escape for the life of the session — with
    /// `input-state` reporting nothing wrong.
    ///
    /// **Mutation note.** Drop the `guides.remove` in `retire` and the second
    /// pad's hold stops firing.
    #[test]
    fn a_pad_yanked_mid_hold_leaves_the_escape_usable() {
        let mut h = harness(2);
        let a = one_pad(&mut h, "/dev/input/event3", "port-a");
        let b = one_pad(&mut h, "/dev/input/event4", "port-b");

        let t = t0();
        h.session.forward(&a, ev::KEY, btn::MODE, 1, t);
        h.session.tick(past(t));
        assert_eq!(h.escapes.borrow().fires, 1);

        // A is unplugged while still holding Guide, so its release never comes.
        h.devices.borrow_mut().retain(|c| c.path != a);
        h.session.poll();

        let later = t + std::time::Duration::from_secs(10);
        h.session.forward(&b, ev::KEY, btn::MODE, 1, later);
        h.session.tick(past(later));
        assert_eq!(
            h.escapes.borrow().fires,
            2,
            "the latch must not survive the pad that set it"
        );
    }

    /// **Rule: a hold arms a deadline the runtime can sleep on.**
    ///
    /// The threshold elapses while the user holds perfectly still, so there is
    /// no event to hang the fire off — without a deadline the escape would only
    /// happen when something else happened to wake the loop.
    ///
    /// **Mutation note.** Leave the Guide holds out of `next_deadline` and this
    /// fails, while every test above still passes because they call `tick`
    /// themselves.
    #[test]
    fn a_hold_arms_a_deadline_for_the_runtime() {
        let mut h = harness(2);
        let path = one_pad(&mut h, "/dev/input/event3", "port-a");
        assert_eq!(h.session.next_deadline(), None);

        let t = t0();
        h.session.forward(&path, ev::KEY, btn::MODE, 1, t);
        assert_eq!(h.session.next_deadline(), Some(t + HOLD));

        h.session
            .forward(&path, ev::KEY, btn::MODE, 0, t + HOLD.mul_f64(0.5));
        assert_eq!(h.session.next_deadline(), None, "a tap disarms it");
    }
}
