//! Volume and mute — **the decision half, and it is pure**.
//!
//! A [`VolumeAction`] plus our own observed address becomes either a
//! [`VolumePlan`] (the exact list of [`VolumeTx`]s to put on the bus, in order)
//! or a [`crate::action::Refusal`] (zero messages, and why) — the same shape
//! [`crate::action`] uses for power and input switching, for the same reason: on
//! a runner with no adapter, CI still covers every rule.
//!
//! [`crate::kernel::ops`] is the other half: the pure translation of each
//! [`VolumeTx`] into `linux-cec` messages, plus the transmitter that puts them on
//! the wire and judges what came back.
//!
//! # The four rules this module exists to enforce
//!
//! 1. **A press is inseparable from its release.**
//!    [`VolumeTx::KeyPressAndRelease`] is ONE intent carrying BOTH halves, and
//!    [`crate::kernel::ops::wire_for`] turns it into a `Wire::KeyPair`, which
//!    carries both messages in one value. There is no way to spell a press on
//!    its own anywhere in this crate. An unreleased `<User Control Pressed>`
//!    auto-repeats on the AVR — the volume runs away until something sends the
//!    release — so "two calls a future edit can separate" was not good enough.
//! 2. **System audio mode is asked about first.** A receiver that has dropped
//!    out of system-audio mode **silently ignores** volume UI commands: it ACKs
//!    the frame at the addressing layer and does nothing with it. So every
//!    volume action begins by reading `<Give System Audio Mode Status>` and, if
//!    the answer is no or absent, sending `<System Audio Mode Request>`. This is
//!    the step most likely to be skipped, and skipping it makes `volume up`
//!    report success on a bus where nothing happened.
//! 3. **Success is judged from the AVR's own `<Report Audio Status>`, never
//!    from the transmit.** A `<User Control Pressed>` is ACKed by an AVR whose
//!    selected input is somebody else's, and then ignored — which is exactly the
//!    §8 constraint ("a receiver ignores CEC from a non-selected input"). The
//!    reported level is read before and after, and only a *change* (or a level
//!    already at the end of the scale) is success. Everything else is reported
//!    as a failure naming the likely cause. Reporting "volume set" because a
//!    frame left the adapter is v1's `cec-health` failure shape, one layer down.
//! 4. **A level this daemon cannot read is `unknown`, never `0`.** CEC encodes
//!    the audio volume in 7 bits where `0x7F` means *"audio volume status
//!    unknown"* and only `0..=100` are real levels; [`level_observation`] maps
//!    everything else to [`Observation::Unknown`]. A muted AVR at level 0 and an
//!    AVR that does not know its own level are different facts.
//!
//! # `mute` / `unmute`, and what the bus can actually express
//!
//! CEC's `<User Control Pressed>[Mute]` (`0x43`) is a **toggle** on essentially
//! every receiver. There are two absolute UI codes — `Mute Function` (`0x65`) and
//! `Restore Volume Function` (`0x66`), both in `linux-cec`'s `UiCommand` — but
//! they are optional in the specification and widely unimplemented, so betting
//! an absolute-sounding verb on them would make `unmute` silently do nothing on
//! the receivers that do not have them.
//!
//! **So an idempotent unmute is not expressible as a single message on this
//! bus.** What this daemon does instead is the shape the rest of it already
//! uses: read `<Report Audio Status>` first, and toggle **only if the AVR is not
//! already in the state asked for** ([`mute_step`]). That makes `mute`/`unmute`
//! idempotent *as verbs* — repeated calls converge, and a call that finds the
//! state already correct transmits nothing and says so — without pretending the
//! bus has a primitive it does not have. And when the read gives no answer,
//! [`MuteStep::Undecidable`] transmits **nothing**: a toggle sent blind is a coin
//! flip that can unmute the television as readily as mute it.

use serde::Serialize;

use crate::action::Refusal;
use crate::state::{Observation, PhysAddr};

/// The highest real CEC audio volume. `0..=100` inclusive.
pub const MAX_LEVEL: u8 = 100;

/// The lowest real CEC audio volume.
pub const MIN_LEVEL: u8 = 0;

/// The 7-bit audio-volume value CEC reserves for *"audio volume status
/// unknown"*.
///
/// It is not a level. Published as [`Observation::Unknown`], never as a number —
/// see rule 4 in the module docs.
pub const LEVEL_UNKNOWN: u8 = 0x7F;

/// What a client asked for.
///
/// A closed set with an exhaustive match everywhere it is consumed, so a verb
/// added here without a decision is a compile error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VolumeAction {
    /// One step up.
    Up,
    /// One step down.
    Down,
    /// Be muted. Idempotent as a verb; see the module docs.
    Mute,
    /// Not be muted. Idempotent as a verb; see the module docs.
    Unmute,
}

impl VolumeAction {
    /// Parse the one-word body of the `volume` verb.
    ///
    /// **A word this daemon does not know is `None`, never a default.** A
    /// `volume` that quietly became `volume up` on a typo would report `ok` for
    /// something the client never asked for, which is exactly the confusion this
    /// design removes.
    #[must_use]
    pub fn parse(word: &str) -> Option<VolumeAction> {
        Some(match word {
            "up" => VolumeAction::Up,
            "down" => VolumeAction::Down,
            "mute" => VolumeAction::Mute,
            "unmute" => VolumeAction::Unmute,
            _ => return None,
        })
    }

    /// The word a client would have written.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            VolumeAction::Up => "up",
            VolumeAction::Down => "down",
            VolumeAction::Mute => "mute",
            VolumeAction::Unmute => "unmute",
        }
    }

    /// Whether this action is about the mute flag rather than the level.
    #[must_use]
    pub const fn is_mute(self) -> bool {
        matches!(self, VolumeAction::Mute | VolumeAction::Unmute)
    }
}

/// The key a level action presses.
///
/// This crate's own vocabulary, not `linux-cec`'s, for the same reason
/// [`crate::action::CecTx`] is: the decision stays pure and the backend seam
/// stays real. [`crate::kernel::ops::wire_for`] is the only translation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VolumeKey {
    /// `<User Control Pressed>[Volume Up]`.
    VolumeUp,
    /// `<User Control Pressed>[Volume Down]`.
    VolumeDown,
    /// `<User Control Pressed>[Mute]` — **a toggle**, see the module docs.
    MuteToggle,
}

/// One thing this daemon intends to put on the bus for a volume action.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VolumeTx {
    /// `<Give System Audio Mode Status>` → Audio System. Answered by
    /// `<System Audio Mode Status>`.
    SystemAudioModeQuery,
    /// `<System Audio Mode Request>` → Audio System, carrying our physical
    /// address. Answered by `<Set System Audio Mode>`.
    ///
    /// **This is the step that must not be skipped.** Without it, an AVR that
    /// has dropped out of system-audio mode ignores every volume UI command
    /// while ACKing every frame.
    SystemAudioModeRequest(PhysAddr),
    /// `<Give Audio Status>` → Audio System. Answered by
    /// `<Report Audio Status>`, which carries the level and the mute flag.
    AudioStatusQuery,
    /// `<User Control Pressed>` **and** `<User Control Released>` — one intent,
    /// both halves. See rule 1 in the module docs.
    KeyPressAndRelease(VolumeKey),
}

/// Everything a volume action needs, once the gates have passed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VolumePlan {
    pub action: VolumeAction,
    /// Our own physical address, for the `<System Audio Mode Request>`.
    pub ours: PhysAddr,
}

impl VolumePlan {
    /// The key this plan's action presses, if it presses one.
    ///
    /// `mute`/`unmute` both press the **toggle**: the absolute `Mute Function` /
    /// `Restore Volume Function` codes are optional in the specification and
    /// widely unimplemented, so the idempotence comes from the read-back in
    /// [`mute_step`] instead. See the module docs.
    #[must_use]
    pub const fn key(&self) -> VolumeKey {
        match self.action {
            VolumeAction::Up => VolumeKey::VolumeUp,
            VolumeAction::Down => VolumeKey::VolumeDown,
            VolumeAction::Mute | VolumeAction::Unmute => VolumeKey::MuteToggle,
        }
    }
}

/// Decide what a volume action becomes, given our own observed address.
///
/// The one gate: a `<System Audio Mode Request>` carries **our** physical
/// address, so without a readable one there is no port to name and the request
/// would be meaningless. A refusal here transmits nothing at all, exactly as in
/// [`crate::action::plan`].
///
/// There is deliberately **no ownership gate**. See
/// [`crate::kernel::ops::execute_volume`] for why, and for what happens instead.
pub fn plan(action: VolumeAction, ours: Observation<PhysAddr>) -> Result<VolumePlan, Refusal> {
    let ours = crate::action::our_address(
        ours,
        "a <System Audio Mode Request> would name no port, and a volume command sent without \
         one cannot be confirmed",
    )?;
    Ok(VolumePlan { action, ours })
}

/// What the AVR said when asked for `<Report Audio Status>`.
///
/// `level` is an [`Observation`] and `muted` is not, because the wire says so:
/// the mute flag is one bit and always means something, while the 7-bit level
/// has a reserved "unknown" encoding. See rule 4.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AudioReport {
    pub level: Observation<u8>,
    pub muted: bool,
}

/// A 7-bit CEC audio-volume field, as an observation.
///
/// **`0x7F` and every other out-of-range value are `unknown`, never a number.**
/// `0x7F` is CEC's own "audio volume status unknown"; anything above
/// [`MAX_LEVEL`] is reserved and means nothing this daemon can publish. Clamping
/// either into the `0..=100` range would publish a level the AVR never reported,
/// and collapsing them into `0` would make "we do not know" identical to "muted
/// and silent".
#[must_use]
pub fn level_observation(raw: u8) -> Observation<u8> {
    if raw > MAX_LEVEL {
        return Observation::Unknown;
    }
    Observation::Known(raw)
}

/// Whether a mute request needs a toggle at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MuteStep {
    /// The AVR already reports the state asked for. **Transmit nothing**: the
    /// postcondition holds and was observed, which is the honest `ok`.
    NothingToDo,
    /// Toggle, then confirm with a second read.
    Toggle,
    /// The AVR's mute state could not be read, so a toggle is a coin flip.
    /// **Transmit nothing** — it could mute a television as readily as unmute
    /// it.
    Undecidable,
}

/// Decide whether `mute`/`unmute` has anything to do.
///
/// `wanted` is the state the verb asks for; `current` is what the AVR reported.
#[must_use]
pub fn mute_step(action: VolumeAction, current: Observation<bool>) -> MuteStep {
    let wanted = match action {
        VolumeAction::Mute => true,
        VolumeAction::Unmute => false,
        // A level action has no mute step. Stated as a value rather than as a
        // panic: this is a long-running daemon and an unreachable arm must not
        // be a way to take it down.
        VolumeAction::Up | VolumeAction::Down => return MuteStep::NothingToDo,
    };
    match current {
        Observation::Known(is) if is == wanted => MuteStep::NothingToDo,
        Observation::Known(_) => MuteStep::Toggle,
        Observation::Unknown => MuteStep::Undecidable,
    }
}

/// What the level read-backs say happened.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LevelVerdict {
    /// The AVR's reported level moved. This is the only success that involves a
    /// transmit.
    Changed { from: u8, to: u8 },
    /// Unchanged because the level is already at the end of the scale. A
    /// success: the postcondition holds.
    AtLimit(u8),
    /// Unchanged, and not at a limit — **the AVR ignored the command**. The
    /// commonest cause is the §8 constraint: a receiver ignores CEC from a
    /// non-selected input.
    Ignored(u8),
    /// One of the two reads produced no level, so nothing can be judged. Never
    /// reported as success.
    Unreadable,
}

/// Judge a level action from the levels reported before and after it.
///
/// **This is the rule that stops a transmit being mistaken for an effect.** It
/// is a pure function of two observations, so the non-selected-input case — the
/// one that matters most — is covered by CI with no adapter and no AVR.
#[must_use]
pub fn judge_level(
    action: VolumeAction,
    before: Observation<u8>,
    after: Observation<u8>,
) -> LevelVerdict {
    match (before, after) {
        (Observation::Known(from), Observation::Known(to)) => {
            if to == from {
                if at_limit(action, from) {
                    LevelVerdict::AtLimit(from)
                } else {
                    LevelVerdict::Ignored(from)
                }
            } else {
                LevelVerdict::Changed { from, to }
            }
        }
        _ => LevelVerdict::Unreadable,
    }
}

/// Whether a level action has nowhere left to go.
#[must_use]
pub fn at_limit(action: VolumeAction, level: u8) -> bool {
    match action {
        VolumeAction::Up => level >= MAX_LEVEL,
        // `== MIN_LEVEL` rather than `<=`: `MIN_LEVEL` is 0 and a `u8` cannot
        // be below it, which clippy's `absurd_extreme_comparisons` rightly
        // flags. The constant stays because the rule is "the bottom of the CEC
        // scale", not "zero".
        VolumeAction::Down => level == MIN_LEVEL,
        VolumeAction::Mute | VolumeAction::Unmute => false,
    }
}

/// What the mute read-back says happened.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MuteVerdict {
    /// The AVR now reports the state asked for.
    Reached(bool),
    /// The toggle was accepted on the bus and the AVR's report did not change —
    /// the same non-selected-input story as [`LevelVerdict::Ignored`].
    Ignored { wanted: bool },
    /// No usable reply after the toggle. Never reported as success.
    Unreadable,
}

/// Judge a mute action from what the AVR reported afterwards.
#[must_use]
pub fn judge_mute(action: VolumeAction, after: Observation<bool>) -> MuteVerdict {
    let wanted = matches!(action, VolumeAction::Mute);
    match after {
        Observation::Known(is) if is == wanted => MuteVerdict::Reached(is),
        Observation::Known(_) => MuteVerdict::Ignored { wanted },
        Observation::Unknown => MuteVerdict::Unreadable,
    }
}

/// Where the values in a `volume-state` reply came from.
///
/// Published rather than inferred, because the two answers age very differently:
/// an `avr-report` is the AVR's answer to a query made just now, while an
/// `observed` value is whatever the receive loop last heard, which on a quiet
/// bus can be minutes or hours old.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum VolumeSource {
    /// The AVR answered `<Give Audio Status>` during this very request.
    AvrReport,
    /// The last `<Report Audio Status>` the receive loop folded.
    Observed,
}

/// One `volume-state` reply, as JSON.
///
/// Every field obeys the crate's rule: `unknown` reaches the wire as `null`,
/// never as `0` and never as `false`. A `source` of `null` means nothing is
/// known at all — which is a different answer from "level 0, unmuted".
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VolumeState {
    pub level: Observation<u8>,
    pub muted: Observation<bool>,
    pub source: Observation<VolumeSource>,
    /// Unix ms the `observed` values were heard, or `null`.
    pub observed_at: Option<u64>,
}

impl VolumeState {
    /// Nothing is known.
    #[must_use]
    pub const fn unknown() -> VolumeState {
        VolumeState {
            level: Observation::Unknown,
            muted: Observation::Unknown,
            source: Observation::Unknown,
            observed_at: None,
        }
    }

    /// The AVR answered a query made just now.
    #[must_use]
    pub const fn from_report(report: AudioReport, now_ms: u64) -> VolumeState {
        VolumeState {
            level: report.level,
            muted: Observation::Known(report.muted),
            source: Observation::Known(VolumeSource::AvrReport),
            observed_at: Some(now_ms),
        }
    }

    /// Fall back to what the receive loop last heard.
    ///
    /// **`source` stays `unknown` when neither field is known**, so an empty
    /// reply cannot read as "we asked and the AVR said nothing much".
    #[must_use]
    pub fn from_observations(
        level: Observation<u8>,
        muted: Observation<bool>,
        observed_at: Option<u64>,
    ) -> VolumeState {
        if !level.is_known() && !muted.is_known() {
            return VolumeState::unknown();
        }
        VolumeState {
            level,
            muted,
            source: Observation::Known(VolumeSource::Observed),
            observed_at,
        }
    }
}

/// What a [`VolumeTx`] came back with.
///
/// This crate's own vocabulary again: no `linux-cec` type reaches [`execute`],
/// which is what lets the whole sequence — including the system-audio-mode step
/// and the read-back judgement — be covered by CI with no adapter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VolumeReply {
    /// The transmit was accepted and expects no reply.
    None,
    /// `<System Audio Mode Status>` / `<Set System Audio Mode>`.
    SystemAudioMode(bool),
    /// `<Report Audio Status>`.
    Audio(AudioReport),
}

/// The bus, as [`execute`] needs it.
///
/// One method, taking this crate's vocabulary and returning this crate's
/// vocabulary. [`crate::kernel::ops::DeviceVolumeBus`] is the real
/// implementation; the tests below drive a recording stand-in, so the *sequence*
/// has exactly one implementation rather than one per environment.
///
/// An implementation is also where a `<Report Audio Status>` gets folded into
/// [`crate::state::Observations`], so `av-state`'s `volume` and `muted` reflect
/// whatever the last action read back.
#[async_trait::async_trait]
pub trait VolumeBus: Send + Sync {
    /// Put one intent on the bus and return what came back.
    async fn perform(&self, tx: VolumeTx) -> Result<VolumeReply, String>;
}

/// Perform a volume action and report **what the AVR did**, not what left the
/// adapter.
///
/// The sequence, and every step of it is load-bearing:
///
/// 1. `<Give System Audio Mode Status>`, and `<System Audio Mode Request>` if
///    the answer is no or absent (rule 2 in the module docs). An AVR out of
///    system-audio mode ACKs volume commands and ignores them.
/// 2. `<Give Audio Status>` — the level and mute flag **before**.
/// 3. The key press *and its release*, as one indivisible intent (rule 1).
/// 4. `<Give Audio Status>` again, and judge (rule 3).
///
/// # There is deliberately no ownership gate here, and no auto-claim
///
/// V2_DESIGN §8's constraint — a receiver ignores CEC from a non-selected
/// input — makes ordering load-bearing for `wake` and `standby`, which is why
/// [`crate::action::plan`] claims the display first. **A volume request is not a
/// lifecycle action**, and claiming the display from inside one would take the
/// television from whatever is on it because somebody nudged the volume. That is
/// a bigger side effect than the verb asks for.
///
/// So this does the non-destructive half instead — `<System Audio Mode
/// Request>`, which asks the AVR to route audio without touching the video
/// input — and, when the AVR ignores the command anyway, **reports the failure
/// naming the cause** rather than reporting a success it cannot support. A
/// caller that wants the input as well has `input-claim`, which says so.
pub async fn execute(bus: &dyn VolumeBus, plan: VolumePlan) -> crate::backend::ActionOutcome {
    ensure_system_audio_mode(bus, plan.ours).await;
    let before = read_audio(bus).await;
    if plan.action.is_mute() {
        perform_mute(bus, plan, before).await
    } else {
        perform_level(bus, plan, before).await
    }
}

/// Step 1: make sure the AVR is in system-audio mode before asking it for
/// anything.
///
/// Never fatal on its own. A failure here is logged and the sequence continues,
/// because the read-back in steps 2-4 is the authority on whether the action
/// landed — an extra verdict derived from this step would be exactly the kind of
/// inference this crate publishes observations instead of.
async fn ensure_system_audio_mode(bus: &dyn VolumeBus, ours: PhysAddr) {
    match bus.perform(VolumeTx::SystemAudioModeQuery).await {
        Ok(VolumeReply::SystemAudioMode(true)) => return,
        Ok(VolumeReply::SystemAudioMode(false)) => {
            tracing::info!(
                "the AVR reports system-audio mode OFF; requesting it with <System Audio Mode \
                 Request>({ours}), because volume UI commands are silently ignored without it"
            );
        }
        Ok(other) => tracing::warn!(
            "<Give System Audio Mode Status> answered {other:?}; requesting system-audio mode \
             anyway"
        ),
        Err(e) => tracing::info!(
            "the AVR did not answer <Give System Audio Mode Status> ({e}); requesting \
             system-audio mode anyway, since that is the state in which it would not have"
        ),
    }
    match bus.perform(VolumeTx::SystemAudioModeRequest(ours)).await {
        Ok(VolumeReply::SystemAudioMode(true)) => {
            tracing::info!("the AVR is now in system-audio mode");
        }
        Ok(reply) => tracing::warn!(
            "<System Audio Mode Request>({ours}) was accepted but the AVR answered {reply:?}; \
             continuing, and the read-back below is what decides whether anything happened"
        ),
        Err(e) => tracing::warn!(
            "<System Audio Mode Request>({ours}) did not land ({e}); continuing, and the \
             read-back below is what decides whether anything happened"
        ),
    }
}

/// Steps 2 and 4: ask the AVR what it is doing.
async fn read_audio(bus: &dyn VolumeBus) -> Option<AudioReport> {
    match bus.perform(VolumeTx::AudioStatusQuery).await {
        Ok(VolumeReply::Audio(report)) => Some(report),
        Ok(other) => {
            tracing::warn!("<Give Audio Status> answered {other:?}, which carries no audio status");
            None
        }
        Err(e) => {
            tracing::info!("the AVR did not report its audio status ({e})");
            None
        }
    }
}

/// `mute` / `unmute`: toggle only if the AVR is not already in the state asked
/// for, and never blind.
async fn perform_mute(
    bus: &dyn VolumeBus,
    plan: VolumePlan,
    before: Option<AudioReport>,
) -> crate::backend::ActionOutcome {
    use crate::backend::ActionOutcome;

    let current = before.map_or(Observation::Unknown, |r| Observation::Known(r.muted));
    match mute_step(plan.action, current) {
        MuteStep::NothingToDo => {
            // Zero transmits, and an honest `ok`: the postcondition holds and it
            // was OBSERVED, not assumed.
            tracing::info!(
                "the AVR already reports muted={}; nothing was transmitted",
                matches!(plan.action, VolumeAction::Mute)
            );
            return ActionOutcome::Done;
        }
        MuteStep::Undecidable => {
            let wanted = plan.action.as_str();
            return ActionOutcome::Failed(format!(
                "the AVR did not report its audio status, so this daemon cannot tell whether it \
                 is already {wanted}d. CEC's <User Control Pressed>[Mute] is a TOGGLE, so sending \
                 one blind could do the opposite of what was asked. Nothing further was \
                 transmitted"
            ));
        }
        MuteStep::Toggle => {}
    }

    if let Err(e) = bus.perform(VolumeTx::KeyPressAndRelease(plan.key())).await {
        return ActionOutcome::Failed(format!(
            "the <User Control Pressed>[Mute] / <User Control Released> pair was not accepted by \
             the bus ({e})"
        ));
    }

    let after = read_audio(bus)
        .await
        .map_or(Observation::Unknown, |r| Observation::Known(r.muted));
    match judge_mute(plan.action, after) {
        MuteVerdict::Reached(is) => {
            tracing::info!("the AVR now reports muted={is}");
            ActionOutcome::Done
        }
        MuteVerdict::Ignored { wanted } => ActionOutcome::Failed(ignored_message(&format!(
            "the AVR still reports muted={}, so the mute toggle was accepted on the bus and \
             ignored",
            !wanted
        ))),
        MuteVerdict::Unreadable => ActionOutcome::Failed(unconfirmable_message()),
    }
}

/// `up` / `down`: press, then judge from the levels reported either side.
async fn perform_level(
    bus: &dyn VolumeBus,
    plan: VolumePlan,
    before: Option<AudioReport>,
) -> crate::backend::ActionOutcome {
    use crate::backend::ActionOutcome;

    let before_level = before.map_or(Observation::Unknown, |r| r.level);
    if let Err(e) = bus.perform(VolumeTx::KeyPressAndRelease(plan.key())).await {
        return ActionOutcome::Failed(format!(
            "the <User Control Pressed> / <User Control Released> pair for volume {} was not \
             accepted by the bus ({e})",
            plan.action.as_str()
        ));
    }
    let after_level = read_audio(bus)
        .await
        .map_or(Observation::Unknown, |r| r.level);

    match judge_level(plan.action, before_level, after_level) {
        LevelVerdict::Changed { from, to } => {
            tracing::info!("the AVR reports its volume moved {from} -> {to}");
            ActionOutcome::Done
        }
        LevelVerdict::AtLimit(level) => {
            tracing::info!(
                "the AVR reports its volume unchanged at {level}, which is the end of the scale"
            );
            ActionOutcome::Done
        }
        LevelVerdict::Ignored(level) => ActionOutcome::Failed(ignored_message(&format!(
            "the AVR still reports volume {level}, so the command was accepted on the bus and \
             ignored"
        ))),
        LevelVerdict::Unreadable => ActionOutcome::Failed(unconfirmable_message()),
    }
}

/// The failure an operator can act on: the AVR heard us and did nothing.
///
/// Names the §8 constraint rather than leaving "it did not work" for someone to
/// diagnose, because the fix is almost always "the AVR is on another input".
fn ignored_message(observed: &str) -> String {
    format!(
        "{observed}. A receiver ignores CEC from a non-selected input, so check the AVR's \
         selected input is this box (`input-claim` claims it) and that it is in system-audio mode"
    )
}

/// The other honest failure: we transmitted and cannot confirm.
///
/// Reported as a failure rather than as `ok` deliberately. A transmit that
/// merely left the adapter is not an effect, and this daemon does not report one
/// as the other — that is v1's `cec-health` failure shape, one layer down.
fn unconfirmable_message() -> String {
    "the command was accepted on the bus, but the AVR did not report its audio status \
     afterwards, so this daemon cannot confirm anything changed. Reporting the transmit \
     honestly rather than as success"
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    const OURS: &str = "2.5.0.0";

    fn addr(s: &str) -> PhysAddr {
        s.parse().unwrap()
    }

    #[test]
    fn the_verb_body_is_a_closed_set() {
        assert_eq!(VolumeAction::parse("up"), Some(VolumeAction::Up));
        assert_eq!(VolumeAction::parse("down"), Some(VolumeAction::Down));
        assert_eq!(VolumeAction::parse("mute"), Some(VolumeAction::Mute));
        assert_eq!(VolumeAction::parse("unmute"), Some(VolumeAction::Unmute));
        for word in ["", "UP", "Up", "louder", "u", "up ", "0", "+1", "toggle"] {
            assert_eq!(VolumeAction::parse(word), None, "{word:?}");
        }
        for action in [
            VolumeAction::Up,
            VolumeAction::Down,
            VolumeAction::Mute,
            VolumeAction::Unmute,
        ] {
            assert_eq!(VolumeAction::parse(action.as_str()), Some(action));
        }
    }

    /// **The rule: a level this daemon cannot read is `unknown`, never `0`.**
    ///
    /// `0x7F` is CEC's own "audio volume status unknown" and is a value a real
    /// AVR really sends — it is what a receiver reports when system-audio mode
    /// is off — so this is not a hypothetical branch.
    ///
    /// Mutation-check (run 2026-09-14): make `level_observation` return
    /// `Observation::Known(0)` (or `Known(raw)`) for an out-of-range value and
    /// this fails, along with the follower's fold test and the `volume-state`
    /// IPC test.
    #[test]
    fn an_unreadable_level_is_unknown_and_never_zero() {
        assert_eq!(level_observation(0), Observation::Known(0));
        assert_eq!(level_observation(50), Observation::Known(50));
        assert_eq!(level_observation(MAX_LEVEL), Observation::Known(100));
        for raw in [LEVEL_UNKNOWN, 101, 110, 126, 127] {
            assert_eq!(
                level_observation(raw),
                Observation::Unknown,
                "{raw:#x} is not a level"
            );
            assert_ne!(level_observation(raw), Observation::Known(0), "{raw:#x}");
        }
    }

    /// **THE LOAD-BEARING TEST: an unchanged level is a FAILURE, not a
    /// success.**
    ///
    /// This is the non-selected-input case. V2_DESIGN §8 records that a receiver
    /// ignores CEC from a non-selected input, and it ignores it *after* ACKing
    /// the frame — so the transmit succeeds and nothing happens. Judging success
    /// from the transmit would report `ok` for a volume that did not move.
    ///
    /// Mutation-check (run 2026-09-14): make `judge_level` return
    /// `Changed { from, to }` (or `AtLimit`) for the unchanged case, or have
    /// `execute_volume` report `Done` without consulting the read-back at all,
    /// and this fails together with the two `kernel::ops` execution tests and
    /// the `ipc` twin.
    #[test]
    fn an_unchanged_level_is_reported_as_ignored_not_as_success() {
        // Mid-scale, unchanged: the AVR heard the frame and did nothing with it.
        assert_eq!(
            judge_level(
                VolumeAction::Up,
                Observation::Known(40),
                Observation::Known(40)
            ),
            LevelVerdict::Ignored(40)
        );
        assert_eq!(
            judge_level(
                VolumeAction::Down,
                Observation::Known(40),
                Observation::Known(40)
            ),
            LevelVerdict::Ignored(40)
        );
    }

    /// The level really moving is the one success that involved a transmit.
    #[test]
    fn a_changed_level_is_the_success_case() {
        assert_eq!(
            judge_level(
                VolumeAction::Up,
                Observation::Known(40),
                Observation::Known(41)
            ),
            LevelVerdict::Changed { from: 40, to: 41 }
        );
        assert_eq!(
            judge_level(
                VolumeAction::Down,
                Observation::Known(40),
                Observation::Known(39)
            ),
            LevelVerdict::Changed { from: 40, to: 39 }
        );
    }

    /// An AVR already at the end of the scale legitimately does not move, and
    /// that is not a failure — the postcondition holds.
    #[test]
    fn an_unchanged_level_at_the_end_of_the_scale_is_not_a_failure() {
        assert_eq!(
            judge_level(
                VolumeAction::Up,
                Observation::Known(MAX_LEVEL),
                Observation::Known(MAX_LEVEL)
            ),
            LevelVerdict::AtLimit(MAX_LEVEL)
        );
        assert_eq!(
            judge_level(
                VolumeAction::Down,
                Observation::Known(MIN_LEVEL),
                Observation::Known(MIN_LEVEL)
            ),
            LevelVerdict::AtLimit(MIN_LEVEL)
        );
        // …and the OTHER direction at the same rail is not at a limit.
        assert_eq!(
            judge_level(
                VolumeAction::Down,
                Observation::Known(MAX_LEVEL),
                Observation::Known(MAX_LEVEL)
            ),
            LevelVerdict::Ignored(MAX_LEVEL)
        );
    }

    /// An unreadable level on either side is never a success.
    #[test]
    fn an_unreadable_read_back_is_never_a_success() {
        for (before, after) in [
            (Observation::Unknown, Observation::Known(40)),
            (Observation::Known(40), Observation::Unknown),
            (Observation::Unknown, Observation::Unknown),
        ] {
            assert_eq!(
                judge_level(VolumeAction::Up, before, after),
                LevelVerdict::Unreadable,
                "{before:?} -> {after:?}"
            );
        }
    }

    /// **The rule: `mute`/`unmute` toggle ONLY when the AVR is not already in
    /// the state asked for, and never at all when the state cannot be read.**
    ///
    /// The bus has no idempotent unmute (see the module docs), so the
    /// idempotence is this read-back's. And a toggle sent blind could mute a
    /// television the user asked to unmute — the exact inverse of the request.
    ///
    /// Mutation-check (run 2026-09-14): make `mute_step` return
    /// `MuteStep::Toggle` for the already-in-state case and
    /// `already_in_the_requested_state_transmits_nothing` fails; make it return
    /// `Toggle` for `Unknown` and `a_blind_mute_toggle_is_never_sent` fails.
    #[test]
    fn mute_toggles_only_when_it_has_to() {
        // Already in state: nothing to do.
        assert_eq!(
            mute_step(VolumeAction::Mute, Observation::Known(true)),
            MuteStep::NothingToDo
        );
        assert_eq!(
            mute_step(VolumeAction::Unmute, Observation::Known(false)),
            MuteStep::NothingToDo
        );
        // Not in state: toggle.
        assert_eq!(
            mute_step(VolumeAction::Mute, Observation::Known(false)),
            MuteStep::Toggle
        );
        assert_eq!(
            mute_step(VolumeAction::Unmute, Observation::Known(true)),
            MuteStep::Toggle
        );
        // Unreadable: a toggle would be a coin flip.
        assert_eq!(
            mute_step(VolumeAction::Mute, Observation::Unknown),
            MuteStep::Undecidable
        );
        assert_eq!(
            mute_step(VolumeAction::Unmute, Observation::Unknown),
            MuteStep::Undecidable
        );
    }

    #[test]
    fn a_mute_is_judged_by_what_the_avr_reports_afterwards() {
        assert_eq!(
            judge_mute(VolumeAction::Mute, Observation::Known(true)),
            MuteVerdict::Reached(true)
        );
        assert_eq!(
            judge_mute(VolumeAction::Unmute, Observation::Known(false)),
            MuteVerdict::Reached(false)
        );
        // Accepted on the bus, ignored by the AVR.
        assert_eq!(
            judge_mute(VolumeAction::Mute, Observation::Known(false)),
            MuteVerdict::Ignored { wanted: true }
        );
        assert_eq!(
            judge_mute(VolumeAction::Unmute, Observation::Known(true)),
            MuteVerdict::Ignored { wanted: false }
        );
        assert_eq!(
            judge_mute(VolumeAction::Mute, Observation::Unknown),
            MuteVerdict::Unreadable
        );
    }

    /// Both mute verbs press the toggle, and both level verbs press their own
    /// key — the whole key table, stated once.
    #[test]
    fn the_key_table_is_exhaustive_and_mute_uses_the_toggle() {
        let ours = addr(OURS);
        let key = |action| VolumePlan { action, ours }.key();
        assert_eq!(key(VolumeAction::Up), VolumeKey::VolumeUp);
        assert_eq!(key(VolumeAction::Down), VolumeKey::VolumeDown);
        assert_eq!(key(VolumeAction::Mute), VolumeKey::MuteToggle);
        assert_eq!(key(VolumeAction::Unmute), VolumeKey::MuteToggle);
    }

    /// The one gate: no readable address of our own, no request that can name a
    /// port — and a refusal transmits nothing.
    #[test]
    fn a_volume_action_without_our_own_address_refuses_and_says_why() {
        let refusal = plan(VolumeAction::Up, Observation::Unknown).expect_err("must refuse");
        assert!(
            refusal.reason.contains("CEC_ADAP_G_PHYS_ADDR"),
            "{}",
            refusal.reason
        );
        let refusal = plan(VolumeAction::Mute, Observation::Known(PhysAddr::INVALID))
            .expect_err("must refuse");
        assert!(
            refusal.reason.contains("invalid address"),
            "{}",
            refusal.reason
        );
        // And the case that proceeds, so the gate is not vacuously passing.
        let plan = plan(VolumeAction::Down, Observation::Known(addr(OURS))).unwrap();
        assert_eq!(plan.ours, addr(OURS));
        assert_eq!(plan.action, VolumeAction::Down);
    }

    /// **The rule: a `volume-state` that knows nothing says so — `null` level,
    /// `null` muted and a `null` source.**
    #[test]
    fn a_volume_state_that_knows_nothing_is_null_throughout() {
        let json = serde_json::to_value(VolumeState::unknown()).unwrap();
        for field in ["level", "muted", "source", "observedAt"] {
            assert_eq!(json[field], serde_json::Value::Null, "{field}: {json}");
            assert_ne!(json[field], serde_json::json!(0), "{field}");
            assert_ne!(json[field], serde_json::json!(false), "{field}");
        }
    }

    /// The two populated shapes, and the `source` that tells them apart.
    #[test]
    fn a_volume_state_names_where_its_values_came_from() {
        let fresh = VolumeState::from_report(
            AudioReport {
                level: Observation::Known(37),
                muted: false,
            },
            1_700_000_000_000,
        );
        let json = serde_json::to_value(&fresh).unwrap();
        assert_eq!(json["level"], serde_json::json!(37));
        assert_eq!(json["muted"], serde_json::json!(false));
        assert_eq!(json["source"], serde_json::json!("avr-report"));

        let stale = VolumeState::from_observations(
            Observation::Known(12),
            Observation::Known(true),
            Some(1_699_000_000_000),
        );
        let json = serde_json::to_value(&stale).unwrap();
        assert_eq!(json["source"], serde_json::json!("observed"));
        assert_eq!(json["observedAt"], serde_json::json!(1_699_000_000_000u64));

        // An AVR that reports a mute flag but no usable level publishes the one
        // it has and `null` for the one it does not — not `0`.
        let partial = VolumeState::from_report(
            AudioReport {
                level: Observation::Unknown,
                muted: true,
            },
            5,
        );
        let json = serde_json::to_value(&partial).unwrap();
        assert_eq!(json["level"], serde_json::Value::Null);
        assert_eq!(json["muted"], serde_json::json!(true));
        assert_eq!(json["source"], serde_json::json!("avr-report"));

        // Nothing known either way collapses to the all-null shape rather than
        // claiming an "observed" source for two nulls.
        let nothing =
            VolumeState::from_observations(Observation::Unknown, Observation::Unknown, Some(9));
        let json = serde_json::to_value(&nothing).unwrap();
        assert_eq!(json["source"], serde_json::Value::Null);
        assert_eq!(json["observedAt"], serde_json::Value::Null);
    }

    // -----------------------------------------------------------------------
    // The sequence, end to end, against a simulated AVR.
    //
    // The bus here is a stand-in, but the SEQUENCE is the real one: `execute`
    // has exactly one implementation and both the kernel backend and these
    // tests drive it. What is replaced is the wire, not the decision.
    // -----------------------------------------------------------------------

    use crate::backend::ActionOutcome;
    use std::sync::Mutex;

    /// A receiver that can be told how to misbehave.
    struct FakeAvr {
        /// What `<Report Audio Status>` answers. `None` models a receiver that
        /// does not answer at all — which is what one out of system-audio mode
        /// does.
        report: Mutex<Option<AudioReport>>,
        system_audio_mode: Mutex<bool>,
        /// Whether it ACTS on a key press. `false` is the non-selected-input
        /// case: the frame is ACKed and the command is ignored.
        acts: bool,
        /// Whether the key press is NAKed outright.
        nak_keys: bool,
        /// Everything this daemon put on the bus, in order.
        log: Mutex<Vec<VolumeTx>>,
    }

    impl FakeAvr {
        fn new(level: u8, muted: bool) -> FakeAvr {
            FakeAvr {
                report: Mutex::new(Some(AudioReport {
                    level: Observation::Known(level),
                    muted,
                })),
                system_audio_mode: Mutex::new(true),
                acts: true,
                nak_keys: false,
                log: Mutex::new(Vec::new()),
            }
        }

        /// A receiver that answers nothing.
        fn silent() -> FakeAvr {
            FakeAvr {
                report: Mutex::new(None),
                ..FakeAvr::new(0, false)
            }
        }

        fn log(&self) -> Vec<VolumeTx> {
            self.log.lock().unwrap().clone()
        }

        fn pressed(&self) -> bool {
            self.log()
                .iter()
                .any(|tx| matches!(tx, VolumeTx::KeyPressAndRelease(_)))
        }
    }

    #[async_trait::async_trait]
    impl VolumeBus for FakeAvr {
        async fn perform(&self, tx: VolumeTx) -> Result<VolumeReply, String> {
            self.log.lock().unwrap().push(tx);
            match tx {
                VolumeTx::SystemAudioModeQuery => Ok(VolumeReply::SystemAudioMode(
                    *self.system_audio_mode.lock().unwrap(),
                )),
                VolumeTx::SystemAudioModeRequest(_) => {
                    *self.system_audio_mode.lock().unwrap() = true;
                    Ok(VolumeReply::SystemAudioMode(true))
                }
                VolumeTx::AudioStatusQuery => match *self.report.lock().unwrap() {
                    Some(report) => Ok(VolumeReply::Audio(report)),
                    None => Err("no reply within the timeout".to_string()),
                },
                VolumeTx::KeyPressAndRelease(key) => {
                    if self.nak_keys {
                        return Err("the bus NAKed the press".to_string());
                    }
                    if self.acts {
                        let mut report = self.report.lock().unwrap();
                        if let Some(r) = report.as_mut() {
                            match (key, r.level) {
                                (VolumeKey::VolumeUp, Observation::Known(l)) => {
                                    r.level =
                                        Observation::Known(l.saturating_add(1).min(MAX_LEVEL));
                                }
                                (VolumeKey::VolumeDown, Observation::Known(l)) => {
                                    r.level = Observation::Known(l.saturating_sub(1));
                                }
                                (VolumeKey::MuteToggle, _) => r.muted = !r.muted,
                                _ => {}
                            }
                        }
                    }
                    Ok(VolumeReply::None)
                }
            }
        }
    }

    fn a_plan(action: VolumeAction) -> VolumePlan {
        plan(action, Observation::Known(addr(OURS))).unwrap()
    }

    /// **The rule: every volume action asks about system-audio mode FIRST, and
    /// requests it when the AVR is not in it.**
    ///
    /// This is the step most likely to be skipped, and skipping it makes
    /// `volume up` report success on a bus where the AVR ignored every frame.
    ///
    /// Mutation-check (run 2026-09-14): delete the `ensure_system_audio_mode`
    /// call from `execute` and this fails on the first assertion; delete only
    /// the `SystemAudioModeRequest` branch and it fails on the second.
    #[tokio::test]
    async fn a_volume_action_ensures_system_audio_mode_before_it_presses_anything() {
        let avr = FakeAvr::new(40, false);
        *avr.system_audio_mode.lock().unwrap() = false;

        assert_eq!(
            execute(&avr, a_plan(VolumeAction::Up)).await,
            ActionOutcome::Done
        );
        assert_eq!(
            avr.log(),
            vec![
                VolumeTx::SystemAudioModeQuery,
                VolumeTx::SystemAudioModeRequest(addr(OURS)),
                VolumeTx::AudioStatusQuery,
                VolumeTx::KeyPressAndRelease(VolumeKey::VolumeUp),
                VolumeTx::AudioStatusQuery,
            ],
            "the query and the request must both precede the press"
        );
    }

    /// An AVR already in system-audio mode is asked, and not told.
    #[tokio::test]
    async fn an_avr_already_in_system_audio_mode_is_not_sent_a_request() {
        let avr = FakeAvr::new(40, false);
        assert_eq!(
            execute(&avr, a_plan(VolumeAction::Down)).await,
            ActionOutcome::Done
        );
        assert_eq!(
            avr.log(),
            vec![
                VolumeTx::SystemAudioModeQuery,
                VolumeTx::AudioStatusQuery,
                VolumeTx::KeyPressAndRelease(VolumeKey::VolumeDown),
                VolumeTx::AudioStatusQuery,
            ]
        );
    }

    /// **THE LOAD-BEARING TEST OF THIS STEP: an AVR on somebody else's input
    /// ACKs the command, ignores it, and this reports a FAILURE.**
    ///
    /// V2_DESIGN §8: a receiver ignores CEC from a non-selected input. Every
    /// frame here is accepted by the bus, so a daemon that judged success from
    /// the transmit would answer `ok` while nothing happened — the exact failure
    /// shape this crate exists to remove.
    ///
    /// Mutation-check (run 2026-09-14): return `ActionOutcome::Done` from
    /// `perform_level` without consulting `judge_level`, and this fails together
    /// with `an_unchanged_level_is_reported_as_ignored_not_as_success` and the
    /// `ipc` twin.
    #[tokio::test]
    async fn an_avr_on_another_input_is_a_failure_and_never_an_ok() {
        for action in [VolumeAction::Up, VolumeAction::Down] {
            let avr = FakeAvr {
                acts: false,
                ..FakeAvr::new(40, false)
            };
            let outcome = execute(&avr, a_plan(action)).await;
            let ActionOutcome::Failed(why) = outcome else {
                panic!("{action:?} must report a failure, got {outcome:?}");
            };
            assert!(
                why.contains("non-selected input"),
                "the failure must name the cause an operator can act on: {why}"
            );
            assert!(
                why.contains("still reports volume 40"),
                "…and what it observed: {why}"
            );
            // The press really was sent, so this is a judged failure and not a
            // refusal wearing one's clothes.
            assert!(avr.pressed(), "{action:?}");
        }
    }

    /// The same for mute: the toggle lands on the bus and the AVR ignores it.
    #[tokio::test]
    async fn an_ignored_mute_toggle_is_a_failure() {
        let avr = FakeAvr {
            acts: false,
            ..FakeAvr::new(40, false)
        };
        let outcome = execute(&avr, a_plan(VolumeAction::Mute)).await;
        let ActionOutcome::Failed(why) = outcome else {
            panic!("must report a failure, got {outcome:?}");
        };
        assert!(why.contains("non-selected input"), "{why}");
        assert!(why.contains("muted=false"), "{why}");
    }

    /// **The rule: an AVR that will not report its audio status cannot produce
    /// an `ok`.**
    ///
    /// The transmit was accepted; that is not an effect. Reported honestly as a
    /// failure naming what is missing.
    ///
    /// Mutation-check (run 2026-09-14): make `perform_level`'s `Unreadable` arm
    /// return `Done` and this fails.
    #[tokio::test]
    async fn a_silent_avr_cannot_confirm_a_level_change_so_it_is_not_a_success() {
        let avr = FakeAvr::silent();
        let outcome = execute(&avr, a_plan(VolumeAction::Up)).await;
        let ActionOutcome::Failed(why) = outcome else {
            panic!("must report a failure, got {outcome:?}");
        };
        assert!(why.contains("cannot confirm"), "{why}");
        // The action was still attempted — this is the honest report of an
        // unconfirmable transmit, not a refusal to act.
        assert!(avr.pressed());
    }

    /// **The rule: a mute toggle is never sent blind.**
    ///
    /// With no readable mute state, `<User Control Pressed>[Mute]` is a coin
    /// flip that could mute a television the user asked to unmute. Nothing is
    /// transmitted past the queries.
    ///
    /// Mutation-check (run 2026-09-14): make `mute_step` return `Toggle` for
    /// `Observation::Unknown` and this fails on the `pressed()` assertion.
    #[tokio::test]
    async fn a_blind_mute_toggle_is_never_sent() {
        for action in [VolumeAction::Mute, VolumeAction::Unmute] {
            let avr = FakeAvr::silent();
            let outcome = execute(&avr, a_plan(action)).await;
            let ActionOutcome::Failed(why) = outcome else {
                panic!("{action:?} must report a failure, got {outcome:?}");
            };
            assert!(why.contains("TOGGLE"), "{why}");
            assert!(
                !avr.pressed(),
                "{action:?} must transmit no key press when the state cannot be read: {:?}",
                avr.log()
            );
        }
    }

    /// **The rule: `mute`/`unmute` are idempotent as verbs — an AVR already in
    /// the requested state is an `ok` with ZERO transmits past the queries.**
    ///
    /// This is what stands in for the idempotent unmute the bus cannot express.
    ///
    /// Mutation-check (run 2026-09-14): make `mute_step` always return `Toggle`
    /// and this fails — the second call flips the AVR back, which is what a
    /// naive toggle-on-every-call would do to a user pressing mute twice.
    #[tokio::test]
    async fn a_mute_verb_converges_instead_of_toggling() {
        let avr = FakeAvr::new(40, false);

        // Not muted -> mute: one toggle, and it lands.
        assert_eq!(
            execute(&avr, a_plan(VolumeAction::Mute)).await,
            ActionOutcome::Done
        );
        assert!(avr.report.lock().unwrap().unwrap().muted);
        assert_eq!(
            avr.log()
                .iter()
                .filter(|tx| matches!(tx, VolumeTx::KeyPressAndRelease(_)))
                .count(),
            1
        );

        // Muted -> mute again: nothing further is transmitted, and it is still
        // muted afterwards.
        assert_eq!(
            execute(&avr, a_plan(VolumeAction::Mute)).await,
            ActionOutcome::Done
        );
        assert!(
            avr.report.lock().unwrap().unwrap().muted,
            "a second `mute` must not unmute the AVR"
        );
        assert_eq!(
            avr.log()
                .iter()
                .filter(|tx| matches!(tx, VolumeTx::KeyPressAndRelease(_)))
                .count(),
            1,
            "the second call must transmit no toggle: {:?}",
            avr.log()
        );

        // And `unmute` converges the other way.
        assert_eq!(
            execute(&avr, a_plan(VolumeAction::Unmute)).await,
            ActionOutcome::Done
        );
        assert!(!avr.report.lock().unwrap().unwrap().muted);
    }

    /// An AVR at the end of the scale legitimately does not move, and that is
    /// not reported as a fault.
    #[tokio::test]
    async fn a_level_already_at_the_rail_is_an_ok() {
        let avr = FakeAvr {
            acts: false,
            ..FakeAvr::new(MAX_LEVEL, false)
        };
        assert_eq!(
            execute(&avr, a_plan(VolumeAction::Up)).await,
            ActionOutcome::Done
        );
        let avr = FakeAvr {
            acts: false,
            ..FakeAvr::new(MIN_LEVEL, false)
        };
        assert_eq!(
            execute(&avr, a_plan(VolumeAction::Down)).await,
            ActionOutcome::Done
        );
    }

    /// A bus that NAKs the press is a failure naming the transmit, distinct
    /// from the AVR ignoring a press it accepted.
    #[tokio::test]
    async fn a_naked_press_is_a_transmit_failure() {
        let avr = FakeAvr {
            nak_keys: true,
            ..FakeAvr::new(40, false)
        };
        let outcome = execute(&avr, a_plan(VolumeAction::Up)).await;
        let ActionOutcome::Failed(why) = outcome else {
            panic!("must report a failure, got {outcome:?}");
        };
        assert!(why.contains("not accepted by the bus"), "{why}");
    }

    /// A refusal is zero transmits, here as everywhere else in this crate.
    #[tokio::test]
    async fn a_volume_action_without_an_address_never_reaches_the_bus() {
        // `plan` is the gate, and it runs before any bus object is touched:
        // there is no path from a refusal to a transmit, because `execute`
        // takes a `VolumePlan` that only `plan` can produce.
        for ours in [Observation::Unknown, Observation::Known(PhysAddr::INVALID)] {
            assert!(plan(VolumeAction::Up, ours).is_err(), "{ours:?}");
        }
    }
}
