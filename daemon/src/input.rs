//! Linux input runtime: the single owner of all mutable daemon state.
//!
//! Runs on its own OS thread (a current-thread tokio runtime, set up in
//! `main.rs`) so real-time input timing — 60 Hz mouse, stick auto-repeat,
//! hold/combo timers — stays off the IPC server's scheduler. The IPC server
//! messages this runtime over the `Control` channel; timers are spawned tasks
//! that post `Internal` messages back to the one select loop, which keeps state
//! mutation single-owner (no `Arc<Mutex>` across `.await`).
//!
//! ## Fleet model (Phase 4)
//!
//! The runtime owns a [`Fleet`] of physical pads keyed by raw fd. Each
//! [`PadDevice`] carries its own input state — held buttons, stick calibration,
//! per-pad timers, a stable player slot (#101) — so multiple pads run
//! independently. Shared resources (the virtual uinput keyboard/mouse, the
//! broadcast bus, the controller DB, the remap table, the capture state) live in
//! [`Shared`], borrowed by every per-pad method.
//!
//! Combo / Home-hold detection is **per-pad-complete** (`[critic P1]`): a combo
//! fires only when a *single* pad holds the full key set. The fleet-level timers
//! check, at fire time, whether *any* pad still holds the complete combo, so two
//! pads pressing Home simultaneously produce one `intent:home-hold`, never a
//! double-fire, and two pads each holding *half* a combo never trigger it. For a
//! single connected pad this is byte-identical to the pre-fleet behavior.
//!
//! This was ported from the former `input/gamepad-input.py` (since deleted —
//! this daemon is now the sole backend). The wire-facing strings
//! and pure decision logic live in `protocol`/`config`/`state` (and are tested
//! on any host); this module is the evdev/uinput glue, exercised by CI on Linux
//! and on the target device.

use crate::config as cfg;
use crate::config::{self, Binding};
use crate::daemon_config::{InputContract, InputContracts};
use crate::device::{self, ControllerDb, SlotAllocator, VirtualRegistry};
use crate::protocol::{
    is_known_intent, pad_connected_json, resp_cancelled, resp_captured, resp_invalid_button,
    resp_ok, resp_pad_battery_not_present, resp_pad_battery_present, resp_pad_not_found,
    resp_pad_rumble_status, resp_pads, resp_status, resp_timeout, resp_unknown_action,
    resp_unknown_intent, resp_unknown_key, Event, InputMode,
};
use crate::state::{self, Control, Reply};
use evdev::uinput::VirtualDevice;
use evdev::{
    AbsoluteAxisCode, AttributeSet, Device, EventStream, EventType, FFEffect, FFEffectCode,
    FFEffectData, FFEffectKind, FFReplay, FFTrigger, InputEvent, KeyCode, RelativeAxisCode,
    UinputAbsSetup,
};
use futures::future::FutureExt;
use std::collections::{HashMap, HashSet};
use std::os::fd::{AsRawFd, RawFd};
use std::panic::AssertUnwindSafe;
use std::path::PathBuf;
use std::time::Duration;
use tokio::sync::{broadcast, mpsc, watch};
use tokio::task::JoinHandle;
use tracing::{debug, error, info, trace, warn};

const EV_KEY: u16 = config::EV_KEY;
const EV_REL: u16 = config::EV_REL;
/// `EV_LED` event type (kernel `input-event-codes.h`). Used to drive a
/// controller's player-indicator LED (#101). Cap-gated by the pad advertising
/// `EventType::LED`; absent on pads without controllable LEDs (most wired pads).
const EV_LED: u16 = 0x11;

/// Duration (ms) of the short haptic pulse fired when a pad connects (#99).
/// Tasteful and brief — a single confirmation buzz, not a sustained rumble.
const CONNECT_RUMBLE_MS: u16 = 150;

/// Follow-focus settle debounce (ms). Hyprland churns the focused window during
/// an app launch/close (the focus flaps empty↔toplevel, and between the shell
/// layer and the app, several times over a fraction of a second). Waiting this
/// long for focus to *settle* before applying the presenter transition collapses
/// that churn into a single net transition — and, the point, avoids tearing down
/// and rebuilding the per-player virtual pad on every flap, which Steam
/// re-enumerates as a controller reconnect (audible/visible, and worse than the
/// original bug). 300ms sits mid-range in the 200–500ms window: long enough to
/// swallow a launch flap, short enough that a deliberate focus change still feels
/// immediate. Only follow-focus is debounced; explicit `grab`/`release`/`handoff`
/// / overlay-focus IPC still applies instantly.
const FOCUS_SETTLE_MS: u64 = 300;

/// Messages posted back into the select loop by timer/reader tasks.
///
/// Each timer message carries a `gen` (generation) token, and the pad-scoped
/// ones also carry the originating pad's `fd`. A timer task may already have sent
/// its message into the channel by the time it is aborted (e.g. a hold fires the
/// same instant the button is released); the handler ignores any message whose
/// `gen` no longer matches the live generation for that timer slot — or whose
/// `fd` is no longer in the fleet — so a stale tick can't double-fire or
/// mis-attribute to a later press or a different pad.
#[derive(Debug)]
enum Internal {
    /// The given left-stick arrow key is repeating on pad `fd`: emit up+down.
    StickRepeat {
        fd: RawFd,
        axis: Axis,
        key: u16,
        generation: u64,
    },
    /// 60 Hz mouse poll tick for pad `fd`.
    MouseTick { fd: RawFd },
    /// Home button held past the threshold on pad `fd`.
    HomeHoldFired { fd: RawFd, generation: u64 },
    /// Home + B held past the threshold on pad `fd`.
    ComboEndSessionFired { fd: RawFd, generation: u64 },
    /// The combo settle window elapsed on pad `fd` with no combo matched: the
    /// buffered combo-participant presses were not a combo, so replay them to the
    /// focused app (see [`PadDevice::combo_buffer`]).
    ComboGuardFired { fd: RawFd, generation: u64 },
    /// A pending `capture-next` timed out (fleet-level).
    CaptureTimeout(u64),
    /// The follow-focus settle debounce elapsed (fleet-level): apply the latest
    /// pending focused-window class if this tick is still the live one. Carries a
    /// `generation` so a superseded settle timer (focus moved again before it
    /// fired) is ignored — only the newest pending focus is applied.
    FocusSettle { generation: u64 },
}

/// Which presenter the fleet is currently driving (Phase 5). Mode is toggled by
/// the `grab`/`release` IPC; the daemon **keeps the physical EVIOCGRAB in both
/// modes** so no controller input ever leaks to the compositor.
///
/// * [`Presenter::Shell`] — the shell home screen owns input. Each pad's
///   buttons/d-pad/left-stick map to keyboard nav keys on the shared virtual
///   keyboard; the right stick drives the shared virtual mouse; gamepad Home
///   becomes `intent:home-tap`/`intent:home-hold`. This is the menu-navigation
///   presenter (#7: all pads share one focus).
/// * [`Presenter::Keyboard`] — a focused external app with a **keyboard input
///   contract** (e.g. `tv.plex.Plex`) owns input. Mechanically identical to
///   [`Presenter::Shell`] — the same shell key-map is emitted onto the shared
///   virtual keyboard/mouse — but the events land on the *focused app* (which
///   holds Wayland focus), not the shell, so a key-driven HTPC UI is d-pad
///   drivable. The distinction from `Shell` is which surface has focus, not the
///   handler; kept a separate variant so follow-focus, `status`, and the logs can
///   tell "shell home focused" from "keyboard-contract app focused". Critically,
///   like `Shell` it holds **no virtual pad**, so Steam has nothing to
///   exclusive-grab (the fix for the always-alive-vpad bug).
/// * [`Presenter::Game`] — a streamed/launched app owns input. Each pad is
///   re-presented as one clean per-player virtual gamepad (`PadDevice.virtual_pad`,
///   #6) carrying the physical pad's events verbatim **except Home**, which is
///   always intercepted into `intent:home-*` so the shell overlay can come up
///   over a running game (substrate for #75). The gamepad-only safety combos
///   (force-quit / suspend / end-session) still run.
/// * [`Presenter::Handoff`] — the Moonlight stream presenter (#221). The physical
///   pad is **UNGRABBED** (EVIOCGRAB released) so SDL/Moonlight reads the real
///   evdev node directly — a true handoff, no virtual twin. The daemon still
///   receives events (the session stays active), but watches **only** the gamepad
///   safety combos (force-quit / suspend / end-session); Home is **not**
///   intercepted, so remote Steam sees the Guide button. Contrast with
///   [`Presenter::Game`], which keeps the grab + a virtual twin (the old
///   `release` path that left SDL seeing a live virtual pad *and* a silently
///   grabbed physical node).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Presenter {
    Shell,
    Keyboard,
    Game,
    Handoff,
}

/// Resources shared across every pad in the fleet: the virtual output devices,
/// the broadcast bus, identity tables, the remap map, and the fleet-level
/// capture/input-mode/generation state. Per-pad input state lives in
/// [`PadDevice`]; methods there borrow this for emission and publishing.
struct Shared {
    events: broadcast::Sender<Event>,
    internal_tx: mpsc::Sender<Internal>,
    kb: VirtualDevice,
    mouse: VirtualDevice,
    db: ControllerDb,
    /// fds of the uinput devices we own, so discovery never re-grabs them.
    reg: VirtualRegistry,

    bindings: Vec<Binding>,
    button_map: HashMap<u16, u16>,

    /// Fleet-aggregate input mode (any pad's most recent mode change). One
    /// source among several once #45 lands; here it's the gamepad source.
    input_mode: InputMode,

    // Monotonic generation allocator for timer messages (see `Internal`).
    gen_seq: u64,

    // Capture (keybinding reassignment) — fleet-level: the next remappable
    // press on ANY grabbed pad resolves it.
    pending_capture: Option<Reply>,
    capture_timeout_task: Option<JoinHandle<()>>,
    capture_gen: u64,

    /// Fleet-level Home-hold dedup latch. Set when any pad's Home-hold timer
    /// fires; `intent:home-hold` is published only on the 0->1 edge, so two pads
    /// holding Home simultaneously emit the single `intent:home-hold` once
    /// (followup: "simultaneous Home-hold fires once"). Cleared when no pad
    /// holds `BTN_MODE` anymore. Single-pad: the latch toggles exactly with the
    /// one pad, so behavior is identical to the pre-fleet single fire.
    home_hold_active: bool,

    /// The active (base) presenter (Phase 5). Starts in [`Presenter::Shell`]
    /// (the shell boots focused). `release`/`grab`/`handoff` flip it; per-pad
    /// event routing (`handle_event`) branches on it — unless `overlay_focus`
    /// is set, in which case the SHELL handler runs regardless (see below).
    presenter: Presenter,

    /// Whether a modal shell overlay is open over a running app (`overlay-focus
    /// on|off`, #262). While `true`, [`Self::presenter`] is left untouched (it
    /// remembers the base presenter) but pad events route to the SHELL key-map
    /// via [`route_presenter`] and every pad is force-grabbed via
    /// [`should_grab`], so the app stops seeing raw events — critical for
    /// [`Presenter::Handoff`], where the pad is normally ungrabbed. Turning it
    /// off restores the base presenter's routing + grab exactly. Defaults
    /// `false`; in-memory only.
    overlay_focus: bool,

    /// Whether our logind session is the foreground (active) one. Maintained by
    /// `Control::SetSessionActive` (the `session` actor). While `false` the
    /// physical `EVIOCGRAB` is dropped on every pad and their events are ignored,
    /// so a VT-switched-to session (Plasma/Bigscreen) owns the controller.
    /// Orthogonal to `presenter`. Defaults `true`.
    session_active: bool,

    /// Per-app input-contract resolver (built-in defaults + `[input.contracts]`
    /// overrides from config.toml). Cloned once at startup from
    /// [`crate::daemon_config`]; follow-focus consults it to pick the presenter
    /// for a focused window class (gamepad→Game, keyboard→Keyboard,
    /// handoff→Handoff). Read-only after startup — contracts change on restart.
    contracts: InputContracts,

    /// True only while [`Presenter::Handoff`] was entered by the explicit
    /// `handoff` IPC (the Moonlight stream path, #221). A pinned Handoff is
    /// **never** overridden by follow-focus — the streamed window holds
    /// compositor focus for the whole stream, and only the shell's explicit
    /// `grab` ends it. A Handoff reached instead via a `handoff` *contract*
    /// (follow-focus) is NOT pinned, so focus moving to another app/the shell
    /// arbitrates normally (no stranding). Set by `handoff_all`; cleared by every
    /// other presenter transition.
    handoff_pinned: bool,

    /// Latest focused-window class awaiting the follow-focus settle debounce
    /// ([`FOCUS_SETTLE_MS`]). A focus change stores the class here and (re)arms
    /// the settle timer; the transition is applied only when
    /// [`Internal::FocusSettle`] fires for the live [`Self::focus_settle_gen`].
    /// `None` when no focus change is pending.
    pending_focus_class: Option<String>,
    /// Generation of the in-flight follow-focus settle timer. Bumped on every
    /// focus change so a superseded timer's `FocusSettle` is ignored — only the
    /// newest pending focus is applied.
    focus_settle_gen: u64,
    /// Handle to the in-flight settle timer task, so a re-arm aborts the prior
    /// one. Bounds the live settle tasks to exactly one during a focus-churn
    /// burst instead of accumulating sleeping tasks (each superseded by the
    /// `focus_settle_gen` check anyway; this just frees them eagerly). `None`
    /// when no settle is pending.
    focus_settle_task: Option<JoinHandle<()>>,

    /// Cached `rumbleEnabled` setting (#108) — refreshed on `set-config` via
    /// `Control::ConfigChanged` instead of re-reading settings.json on every
    /// rumble.
    rumble_enabled: bool,

    /// Per-player binding overrides (#104). Keyed by player slot (0..=3).
    /// Refreshed on `Control::ConfigChanged`.
    per_player_bindings: HashMap<u8, HashMap<&'static str, u16>>,
    /// Per-game binding overrides (#104). Keyed by game id string.
    /// Refreshed on `Control::ConfigChanged`.
    per_game_bindings: HashMap<String, HashMap<&'static str, u16>>,
    /// The currently active game id for per-game binding lookup (#104).
    /// Set via `set-active-game <id>` IPC; in-memory only (not persisted).
    active_game: Option<String>,

    /// Shared observability counters. Incremented on the relevant events
    /// (intents, transitions, pad join/leave) and read by the metrics exporter.
    metrics: std::sync::Arc<crate::metrics::Metrics>,

    /// Meta (BTN_MODE / Guide) tap-vs-hold threshold. Read once at startup from
    /// `daemon_config::global().input.meta_hold_ms` (like `contracts`); the Meta
    /// hold timer sleeps this long before firing the shell escape. Replaces the
    /// former hard-coded `config::HOME_HOLD_SECS`.
    meta_hold: Duration,
    /// Combo settle window. Read once at startup from
    /// `daemon_config::global().input.combo_guard_ms`; the combo buffer replays a
    /// buffered participant to the app if no combo completes within this window.
    combo_guard: Duration,
}

impl Shared {
    fn publish(&self, ev: Event) {
        // One chokepoint for every broadcast event — `RUST_LOG=…input=debug`
        // turns this into a full event tracer (intents, combos, pad:*,
        // input-mode, controller-wake, status pushes).
        debug!(event = %ev, "publish");
        // Observability: this is also the single chokepoint for the
        // intent/pad-join/pad-leave counters — every intent (IPC, HTTP, MCP,
        // gamepad Home-tap/hold) and every pad connect/disconnect funnels here.
        match &ev {
            Event::Intent(_) => self.metrics.inc_intents(),
            Event::PadConnected(_) => self.metrics.inc_pad_joins(),
            Event::PadDisconnected(_) => self.metrics.inc_pad_leaves(),
            _ => {}
        }
        let _ = self.events.send(ev);
    }

    fn emit_key(&mut self, key: u16, value: i32) {
        // value: 1=press, 0=release, 2=autorepeat. The nav keys/intents the
        // shell sees originate here; trace-level so a stick auto-repeat burst
        // doesn't drown the debug stream.
        trace!(key, value, "emit_key");
        let _ = self.kb.emit(&[InputEvent::new(EV_KEY, key, value)]);
    }

    fn emit_mouse_button(&mut self, button: u16, value: i32) {
        trace!(button, value, "emit_mouse_button");
        let _ = self.mouse.emit(&[InputEvent::new(EV_KEY, button, value)]);
    }

    fn emit_mouse_move(&mut self, dx: i32, dy: i32) {
        let mut evs: Vec<InputEvent> = Vec::with_capacity(2);
        if dx != 0 {
            evs.push(InputEvent::new(EV_REL, cfg::REL_X, dx));
        }
        if dy != 0 {
            evs.push(InputEvent::new(EV_REL, cfg::REL_Y, dy));
        }
        if !evs.is_empty() {
            let _ = self.mouse.emit(&evs);
        }
    }

    fn send_moonlight_quit(&mut self) {
        let keys = [
            cfg::KEY_LEFTCTRL,
            cfg::KEY_LEFTALT,
            cfg::KEY_LEFTSHIFT,
            cfg::KEY_Q,
        ];
        for &k in &keys {
            self.emit_key(k, 1);
        }
        for &k in keys.iter().rev() {
            self.emit_key(k, 0);
        }
        info!("Sent Ctrl+Alt+Shift+Q to quit Moonlight");
    }

    fn set_input_mode(&mut self, mode: InputMode) {
        if mode == self.input_mode {
            return;
        }
        self.input_mode = mode;
        self.publish(Event::InputMode(mode));
    }

    /// Allocate the next monotonic generation token for a timer.
    fn next_generation(&mut self) -> u64 {
        self.gen_seq += 1;
        self.gen_seq
    }

    fn rebuild_button_map(&mut self) {
        self.button_map.clear();
        for b in &self.bindings {
            self.button_map.insert(b.button, b.key);
        }
    }

    /// Resolve the keyboard key for a remappable button press from the layered
    /// bindings (#104). Resolution order: game override → player override →
    /// global. Returns `None` when `button` is not an assigned action button.
    ///
    /// Only call this for remappable buttons on a keydown event — the hot path
    /// is allocation-light (≤4 actions via `config::resolve_button_key`).
    fn resolved_key(&self, slot: u8, button: u16) -> Option<u16> {
        let player = self.per_player_bindings.get(&slot);
        let game = self
            .active_game
            .as_deref()
            .and_then(|id| self.per_game_bindings.get(id));
        config::resolve_button_key(&self.bindings, player, game, button)
    }
}

/// One physical pad in the fleet: its grabbed event stream, stable player slot,
/// and all per-pad input state. The fd (the event stream's raw fd) is the
/// in-process key; `wire_id` is the stable cross-reconnect id used in `pad:*`
/// payloads.
struct PadDevice {
    fd: RawFd,
    event_stream: EventStream,
    wire_id: String,
    name: String,
    path: PathBuf,
    player_slot: u8,
    grabbed: bool,

    held_keys: HashSet<u16>,
    /// Digital buttons that were physically held at the moment this pad entered
    /// the Game presenter (the shell→app flip). Populated from `held_keys` in
    /// [`PadDevice::enter_game`] on the transition that builds the virtual pad;
    /// cleared on [`PadDevice::enter_shell`] and whenever the virtual pad is
    /// (re)built. In the Game-presenter forwarding path a masked code is
    /// **swallowed** (never forwarded to the virtual pad) until it is released
    /// and pressed fresh — so the single physical press that launched an app
    /// from the shell does not also leak into the newly-focused app (the
    /// Steam-BPM A-leak, #295 follow-up). Kept SEPARATE from `held_keys`, which
    /// the safety combos read directly and must remain untouched.
    masked_keys: HashSet<u16>,
    left_trigger_held: bool,
    right_trigger_held: bool,

    // Left stick
    stick_x_key: Option<u16>,
    stick_y_key: Option<u16>,
    stick_x_repeat: Option<JoinHandle<()>>,
    stick_y_repeat: Option<JoinHandle<()>>,
    stick_x_gen: u64,
    stick_y_gen: u64,
    stick_center_x: i32,
    stick_threshold_x: i32,
    stick_center_y: i32,
    stick_threshold_y: i32,

    // Right stick
    rstick_center_x: i32,
    rstick_threshold_x: i32,
    rstick_half_range_x: i32,
    rstick_center_y: i32,
    rstick_threshold_y: i32,
    rstick_half_range_y: i32,
    rstick_raw_x: i32,
    rstick_raw_y: i32,
    rstick_x_dir: Option<&'static str>,
    rstick_y_dir: Option<&'static str>,
    mouse_task: Option<JoinHandle<()>>,

    // Hold/combo timers (per-pad: each pad detects its own complete combo).
    home_hold_task: Option<JoinHandle<()>>,
    home_hold_gen: u64,
    combo_task: Option<JoinHandle<()>>,
    combo_gen: u64,

    /// Combo-safety buffer (#escape-contract). In an APP presenter
    /// (Keyboard/Game) a combo-participant press is buffered here instead of
    /// forwarded, so a *partial* safety combo (e.g. the first two of
    /// Back+Home+LB+RB) never leaks into the focused app as a stray media key.
    /// The buffer is replayed to the app (in order) if the sequence is proven not
    /// to be a combo, or discarded if a combo completes. Holds `(code, value)`
    /// pairs in arrival order. Empty + disarmed in the Shell/Handoff presenters
    /// and reset on every presenter/session/overlay transition so a partial
    /// sequence can never strand across a context change.
    combo_buffer: Vec<(u16, i32)>,
    /// Whether [`Self::combo_buffer`] is actively buffering (armed). Armed by the
    /// first (Keyboard) / second (Game) held participant; cleared on
    /// swallow/replay/guard-timeout and every transition.
    combo_armed: bool,
    /// The combo settle-window timer (`combo_guard_ms`), armed when buffering
    /// starts. On fire it replays the buffer to the app (the "no combo arrived in
    /// time" disqualifier). `None` when not buffering.
    combo_guard_task: Option<JoinHandle<()>>,
    /// Generation token for [`Self::combo_guard_task`], so a stale fire (the guard
    /// disarmed/rearmed before its message was drained) is ignored.
    combo_guard_gen: u64,

    // --- Fleet outputs (ride-along, Phase 5.5) ---
    /// One clean virtual gamepad per player in game-presenter mode (Phase 5).
    /// `None` in shell mode. Registered in `Shared.reg` at creation, dropped +
    /// unregistered on leave.
    virtual_pad: Option<VirtualDevice>,
    /// Latest battery snapshot (#100). `None` until first read or for a wired
    /// pad with no reported battery. Updated by the sysfs battery poll; a change
    /// emits `pad:battery:{id,level,charging}`.
    battery: Option<BatteryState>,
    /// Player LED index lit at slot allocation (#101), if the pad is
    /// `EV_LED`-capable. `None` for pads without a controllable LED.
    led_index: Option<u8>,
    /// The uploaded rumble (FF_RUMBLE) effect (#99), if the pad supports force
    /// feedback. Uploaded lazily on the first `rumble` and kept alive here (its
    /// `Drop` erases the kernel effect), re-uploaded when the requested duration
    /// changes. `None` for pads without `EV_FF`/`FF_RUMBLE`.
    ff_effect: Option<FFEffect>,
    /// The replay length (ms) of the currently-uploaded `ff_effect`, so a repeat
    /// rumble at the same duration replays the cached effect instead of
    /// re-uploading it.
    ff_length_ms: u16,
}

/// Battery snapshot for a pad (#100). `None` on a pad means "no reported
/// battery" (a wired pad). Compared across polls to emit `pad:battery:*` only on
/// change.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct BatteryState {
    /// Charge percentage 0..=100.
    level: u8,
    charging: bool,
}

impl PadDevice {
    fn new(
        fd: RawFd,
        stream: EventStream,
        wire_id: String,
        name: String,
        path: PathBuf,
        slot: u8,
    ) -> PadDevice {
        PadDevice {
            fd,
            event_stream: stream,
            wire_id,
            name,
            path,
            player_slot: slot,
            grabbed: false,
            held_keys: HashSet::new(),
            masked_keys: HashSet::new(),
            left_trigger_held: false,
            right_trigger_held: false,
            stick_x_key: None,
            stick_y_key: None,
            stick_x_repeat: None,
            stick_y_repeat: None,
            stick_x_gen: 0,
            stick_y_gen: 0,
            stick_center_x: 0,
            stick_threshold_x: 0,
            stick_center_y: 0,
            stick_threshold_y: 0,
            rstick_center_x: 0,
            rstick_threshold_x: 0,
            rstick_half_range_x: 1,
            rstick_center_y: 0,
            rstick_threshold_y: 0,
            rstick_half_range_y: 1,
            rstick_raw_x: 0,
            rstick_raw_y: 0,
            rstick_x_dir: None,
            rstick_y_dir: None,
            mouse_task: None,
            home_hold_task: None,
            home_hold_gen: 0,
            combo_task: None,
            combo_gen: 0,
            combo_buffer: Vec::new(),
            combo_armed: false,
            combo_guard_task: None,
            combo_guard_gen: 0,
            virtual_pad: None,
            battery: None,
            led_index: None,
            ff_effect: None,
            ff_length_ms: 0,
        }
    }

    // --- held-buttons notification --------------------------------------

    fn notify_held_buttons(&self, sh: &Shared) {
        if sh.events.receiver_count() == 0 {
            return;
        }
        let mut held: Vec<u16> = self.held_keys.iter().copied().collect();
        held.sort_unstable();
        let payload = state::build_buttons_payload(
            &held,
            self.left_trigger_held,
            self.right_trigger_held,
            self.stick_x_key,
            self.stick_y_key,
            self.rstick_x_dir,
            self.rstick_y_dir,
            |code| Some(format!("{:?}", KeyCode::new(code))),
        );
        sh.publish(Event::Buttons(payload));
    }

    // --- grab lifecycle --------------------------------------------------

    fn grab(&mut self, sh: &mut Shared) {
        if self.grabbed {
            return;
        }
        // Don't take the physical grab while our session is backgrounded — the
        // foreground DE owns the controller then. Re-grabbed on reactivation via
        // `Control::SetSessionActive(true)`.
        if !sh.session_active {
            return;
        }
        match self.event_stream.device_mut().grab() {
            Ok(()) => {
                self.grabbed = true;
                self.cancel_combo_unconditional(sh);
                self.held_keys.clear();
                self.reset_triggers();
                sh.set_input_mode(InputMode::Controller);
                info!("Grabbed pad slot {} ({})", self.player_slot, self.wire_id);
            }
            Err(e) => error!("Failed to grab pad {}: {e}", self.wire_id),
        }
    }

    fn ungrab(&mut self, sh: &mut Shared) {
        self.reset_stick_state(sh);
        if !self.grabbed {
            return;
        }
        match self.event_stream.device_mut().ungrab() {
            Ok(()) => {
                self.grabbed = false;
                self.cancel_combo_unconditional(sh);
                self.held_keys.clear();
                self.reset_triggers();
                info!("Released pad slot {} ({})", self.player_slot, self.wire_id);
            }
            Err(e) => error!("Failed to ungrab pad {}: {e}", self.wire_id),
        }
    }

    fn calibrate(&mut self) {
        let Ok(absinfo) = self.event_stream.device().get_absinfo() else {
            return;
        };
        for (axis, info) in absinfo {
            let center = (info.minimum() + info.maximum()) / 2;
            let half_range = (info.maximum() - info.minimum()) / 2;
            let threshold = (half_range as f64 * config::STICK_DEADZONE) as i32;
            match axis.0 {
                cfg::ABS_X => {
                    self.stick_center_x = center;
                    self.stick_threshold_x = threshold;
                }
                cfg::ABS_Y => {
                    self.stick_center_y = center;
                    self.stick_threshold_y = threshold;
                }
                cfg::ABS_RX => {
                    self.rstick_center_x = center;
                    self.rstick_threshold_x = threshold;
                    self.rstick_half_range_x = half_range;
                }
                cfg::ABS_RY => {
                    self.rstick_center_y = center;
                    self.rstick_threshold_y = threshold;
                    self.rstick_half_range_y = half_range;
                }
                _ => {}
            }
        }
        info!(
            "Pad slot {} calibration: X center={} threshold={}, Y center={} threshold={}",
            self.player_slot,
            self.stick_center_x,
            self.stick_threshold_x,
            self.stick_center_y,
            self.stick_threshold_y
        );
    }

    fn reset_triggers(&mut self) {
        self.left_trigger_held = false;
        self.right_trigger_held = false;
    }

    fn reset_stick_state(&mut self, sh: &mut Shared) {
        if let Some(key) = self.stick_x_key.take() {
            sh.emit_key(key, 0);
        }
        self.cancel_stick_repeat_x(sh);
        if let Some(key) = self.stick_y_key.take() {
            sh.emit_key(key, 0);
        }
        self.cancel_stick_repeat_y(sh);
        self.rstick_x_dir = None;
        self.rstick_y_dir = None;
        if let Some(t) = self.mouse_task.take() {
            t.abort();
        }
    }

    /// Abort every spawned task this pad owns (stick repeats, mouse poll,
    /// hold/combo timers). Called on leave so a dropped pad leaves no orphan
    /// tasks posting `Internal` messages for a fd that's gone.
    fn abort_all_tasks(&mut self) {
        if let Some(t) = self.stick_x_repeat.take() {
            t.abort();
        }
        if let Some(t) = self.stick_y_repeat.take() {
            t.abort();
        }
        if let Some(t) = self.mouse_task.take() {
            t.abort();
        }
        if let Some(t) = self.home_hold_task.take() {
            t.abort();
        }
        if let Some(t) = self.combo_task.take() {
            t.abort();
        }
        // The combo-buffer guard timer + any buffered participants. No generation
        // bump here (unlike `disarm_combo_guard`): this runs on pad-leave/shutdown
        // where the pad is dropped immediately after, so a stale queued fire is
        // reaped by the fd-miss guard in the `ComboGuardFired` handler.
        if let Some(t) = self.combo_guard_task.take() {
            t.abort();
        }
        self.combo_buffer.clear();
        self.combo_armed = false;
    }

    // --- event handling --------------------------------------------------

    fn handle_event(&mut self, sh: &mut Shared, ev: InputEvent) {
        // Deepest debug level: every raw evdev event from the physical pad.
        // `RUST_LOG=game_shell_input::input=trace` shows the full input stream
        // (slot + type/code/value) for diagnosing a misbehaving button/axis.
        trace!(
            slot = self.player_slot,
            ev_type = ev.event_type().0,
            code = ev.code(),
            value = ev.value(),
            "pad event"
        );
        // Route by the *effective* presenter, not the physical grab: the pad
        // stays grabbed in both Shell/Game modes (Phase 5), so `grabbed` no
        // longer discriminates. `route_presenter` folds in overlay-focus (#262)
        // — a modal shell overlay open over a running app forces the Shell
        // handler over any base presenter so the pad drives the overlay, not the
        // app; the base presenter stays remembered in `sh.presenter`.
        match route_presenter(sh.overlay_focus, sh.presenter) {
            // Keyboard shares the shell key-map handler — the difference between
            // "shell home focused" and "keyboard-contract app focused" is which
            // surface holds Wayland focus (where the emitted keys land), not how
            // the pad is translated. Neither holds a virtual pad.
            Presenter::Shell | Presenter::Keyboard => self.handle_shell(sh, ev),
            Presenter::Game => self.handle_game(sh, ev),
            Presenter::Handoff => self.handle_handoff(sh, ev),
        }
    }

    /// Shell presenter: map the pad to keyboard nav + mouse on the shared virtual
    /// devices, and turn gamepad Home into `intent:home-tap`/`intent:home-hold`.
    fn handle_shell(&mut self, sh: &mut Shared, ev: InputEvent) {
        let et = ev.event_type();
        let code = ev.code();
        let value = ev.value();

        if et == EventType::KEY {
            // Capture mode resolves on keydown of a remappable button (any pad).
            if sh.pending_capture.is_some() && value == 1 {
                if config::is_remappable(code) {
                    resolve_capture(sh, code);
                }
                return;
            }

            if value == 1 {
                self.held_keys.insert(code);
                self.check_combo_start(sh);
                self.check_quit_combo(sh);
                self.check_suspend_combo(sh);
                self.notify_held_buttons(sh);
                if code == cfg::BTN_TL || code == cfg::BTN_TR {
                    sh.set_input_mode(InputMode::Mouse);
                } else if config::is_remappable(code) || code == cfg::BTN_SELECT {
                    sh.set_input_mode(InputMode::Controller);
                }
            } else if value == 0 {
                self.held_keys.remove(&code);
                self.cancel_combo(sh);
                self.notify_held_buttons(sh);
            }

            // Meta (BTN_MODE / Guide): tap-vs-hold split, buffered delivery. Never
            // forwarded live while discriminating (no partial-press leak); the tap
            // is delivered per presenter and the hold is the reserved escape. See
            // `handle_meta`. Handled the same in the Shell home and a Keyboard-
            // contract app (both routed here) — only the tap *destination* differs
            // by presenter, which `handle_meta` resolves from `routed`.
            let routed = route_presenter(sh.overlay_focus, sh.presenter);
            if code == cfg::BTN_MODE {
                self.handle_meta(sh, value, routed);
                return;
            }

            // A Keyboard-contract app is a focused external app (Plex): route its
            // combo participants through the combo buffer so a partial safety
            // combo never leaks in as a media key (the bug this fixes). The Shell
            // home has no app to leak into, so it forwards immediately (unchanged).
            if routed == Presenter::Keyboard {
                self.gate_and_forward_shell_key(sh, code, value);
            } else {
                self.shell_emit_key_event(sh, code, value);
            }
        } else if et == EventType::ABSOLUTE {
            self.handle_abs(sh, code, value);
        }
    }

    /// Emit the shell key-map side effects for one KEY event: the View/Select
    /// `overlay:session` deep-link (#218), LB/RB → mouse clicks, and the layered
    /// button→keyboard map (#104). This is "forward a button to the app" in the
    /// Shell/Keyboard presenter — used both for immediate forwarding (Shell) and
    /// for replaying a buffered participant (Keyboard). Split out so the combo
    /// buffer's replay reproduces exactly the same side effects as a live press.
    ///
    /// View/Select is deliberately NOT mirrored in `handle_game`, so it never
    /// interferes with the in-game force-quit combo (Back+Home+LB+RB, which also
    /// uses BTN_SELECT) or game input. BTN_SELECT has no default key binding, so
    /// nothing else consumes its press.
    fn shell_emit_key_event(&mut self, sh: &mut Shared, code: u16, value: i32) {
        if code == cfg::BTN_SELECT && value == 1 {
            sh.publish(Event::Intent("overlay:session".into()));
        }
        if code == cfg::BTN_TL {
            sh.emit_mouse_button(cfg::BTN_LEFT, value);
        } else if code == cfg::BTN_TR {
            sh.emit_mouse_button(cfg::BTN_RIGHT, value);
        }
        if let Some(mapped) = sh.resolved_key(self.player_slot, code) {
            sh.emit_key(mapped, value);
        }
    }

    /// Game presenter (Phase 5): re-present this physical pad as the clean
    /// per-player virtual gamepad, forwarding its events verbatim **except Home**.
    ///
    /// The physical pad stays grabbed, so nothing leaks to the compositor; the
    /// game reads `game-shell-virtual-pad-<slot>` instead. Home (`BTN_MODE`) is
    /// always intercepted into `intent:home-tap`/`intent:home-hold` (never
    /// forwarded) so the shell overlay can come up over a running game. The
    /// gamepad-only safety combos (force-quit / suspend / end-session) still run
    /// off `held_keys`, exactly as before.
    fn handle_game(&mut self, sh: &mut Shared, ev: InputEvent) {
        let et = ev.event_type();
        let code = ev.code();
        let value = ev.value();

        if et == EventType::KEY {
            // Track held keys for the gamepad-only safety combos (force-quit /
            // suspend / end-session). These watch raw button state — including
            // `BTN_MODE` — and must keep working over a running game. Ordering
            // mirrors `handle_shell` so a combo arms identically regardless of
            // which button (Home or the other) is pressed last.
            if value == 1 {
                self.held_keys.insert(code);
                self.check_combo_start(sh);
                self.check_quit_combo(sh);
                self.check_suspend_combo(sh);
            } else if value == 0 {
                self.held_keys.remove(&code);
                self.cancel_combo(sh);
            }

            // Meta (BTN_MODE / Guide): tap-vs-hold split. Never forwarded live;
            // a TAP replays a real Guide press+release to the vpad (so the game/
            // Steam sees a Guide tap), a HOLD is the reserved shell escape (never
            // reaches the game). See `handle_meta`.
            if code == cfg::BTN_MODE {
                self.handle_meta(sh, value, Presenter::Game);
                return; // never forward the live Home to the game
            }

            // Route the remaining buttons through the combo buffer (so a partial
            // safety combo doesn't leak into the game) then, on forward/replay,
            // through the flip-mask onto the clean virtual pad.
            self.gate_and_forward_game_key(sh, code, value);
        } else if et == EventType::ABSOLUTE {
            // Forward sticks/triggers/d-pad verbatim — the game wants raw axes.
            self.forward_to_virtual_pad(ev);
        }
    }

    /// Handoff presenter (#221): the physical pad is UNGRABBED, so SDL/Moonlight
    /// reads the real evdev node directly (true handoff — no virtual twin). The
    /// daemon still receives every event because we keep the read half of the fd,
    /// but it only watches the gamepad safety combos off `held_keys`. There is no
    /// virtual pad, no key mapping, and **no Home interception** — Home flows
    /// straight through to the game so remote Steam sees the Guide button.
    fn handle_handoff(&mut self, sh: &mut Shared, ev: InputEvent) {
        let et = ev.event_type();
        let code = ev.code();
        let value = ev.value();

        if et == EventType::KEY {
            // Watch only the gamepad-only safety combos (force-quit / suspend /
            // end-session). No virtual-pad forwarding, no key map, no Home
            // intercept — the ungrabbed node is what the game reads.
            if value == 1 {
                self.held_keys.insert(code);
                self.check_combo_start(sh);
                self.check_quit_combo(sh);
                self.check_suspend_combo(sh);
            } else if value == 0 {
                self.held_keys.remove(&code);
                self.cancel_combo(sh);
            }

            // Unpinned (contract-driven) Handoff: best-effort Meta HOLD escape.
            // The node is UNGRABBED, so full swallowing is impossible — the app
            // reads the raw Meta press directly (it may see Guide for up to the
            // hold threshold). But the daemon still receives events, so a HOLD past
            // the threshold publishes the escape intent (`home-tap` → the shell's
            // controllable overlay). A tap-length press is left entirely to the app
            // (no intent). A PINNED Handoff (the Moonlight `handoff` IPC, #221) is
            // untouched: Home flows straight through so remote Steam sees Guide.
            if code == cfg::BTN_MODE && !sh.handoff_pinned {
                if value == 1 {
                    self.start_home_hold(sh);
                } else if value == 0 {
                    if let Some(t) = self.home_hold_task.take() {
                        t.abort();
                        // Release before the hold fired: no escape, and no tap
                        // intent (the ungrabbed app already saw the raw press —
                        // publishing a tap would double-act). Invalidate any queued
                        // HomeHoldFired so a near-miss can't still fire the escape.
                        self.home_hold_gen = sh.next_generation();
                    }
                }
            }
        }
        // ABS (sticks/triggers/d-pad) are intentionally ignored: the game reads
        // them off the ungrabbed physical node directly.
    }

    /// Emit one event onto this pad's clean virtual gamepad, if it has one.
    /// A no-op when `virtual_pad` is `None` (e.g. a transient race during a
    /// presenter flip); the event is simply dropped rather than leaked.
    fn forward_to_virtual_pad(&mut self, ev: InputEvent) {
        if let Some(vpad) = self.virtual_pad.as_mut() {
            let _ = vpad.emit(&[ev]);
        }
    }

    /// Forward one KEY event to the game's virtual pad, honoring the flip-mask
    /// (#295 follow-up): a button held at the shell→app flip is swallowed until
    /// released and pressed fresh. "Forward a button to the app" in the Game
    /// presenter — used both for immediate forwarding and for replaying a buffered
    /// combo participant. ABS axes are never masked and never routed here.
    fn game_forward_key(&mut self, code: u16, value: i32) {
        if mask_forward_decision(&mut self.masked_keys, code, value) {
            self.forward_to_virtual_pad(InputEvent::new(EV_KEY, code, value));
        }
    }

    /// Meta (BTN_MODE / Guide) tap-vs-hold split with buffered delivery.
    ///
    /// Net semantic change vs the pre-escape-contract daemon: the Meta press is
    /// NEVER forwarded live (no partial-press leak while we discriminate). On
    /// press we arm the hold timer; the button is "buffered" (withheld). On
    /// release we know it was a TAP (timer still pending) or, if the timer already
    /// fired, a HOLD whose escape was already published — so the release is
    /// swallowed. Per presenter (`routed`):
    ///
    /// * **TAP** (release before the threshold):
    ///   - Shell → publish `intent:home-tap` (the drawer; no app to forward to).
    ///   - Game → deliver the buffered Guide to the app as a real tap: forward
    ///     BTN_MODE press then release to the vpad (the game/Steam sees a Guide
    ///     tap).
    ///   - Keyboard → deliver NOTHING: a keyboard-contract app (Plex) has no Guide
    ///     concept and a synthetic keyboard chord would be arbitrary — the user's
    ///     escape here is the HOLD.
    /// * **HOLD** (timer fired, [`Internal::HomeHoldFired`]): the escape intent is
    ///   published by that handler (fleet-deduped). In an APP presenter that intent
    ///   is `intent:home-tap` (the controllable overlay drawer over the app — a
    ///   non-destructive everyday escape); only the Shell presenter's hold publishes
    ///   `intent:home-hold` (the idle reset-to-clean-home). Either way Meta is fully
    ///   swallowed — the app never sees it (except the Game-TAP replay above).
    ///
    /// This is the shell's reserved escape in every grabbed presenter; the app is
    /// never shown the button while we discriminate, so there is no leak window.
    fn handle_meta(&mut self, sh: &mut Shared, value: i32, routed: Presenter) {
        if value == 1 {
            // Press: arm the hold timer, buffer the Guide (do NOT forward).
            self.start_home_hold(sh);
        } else if value == 0 {
            // Release: if the hold timer is still pending this is a TAP; if it
            // already fired the HOLD path published the escape and we swallow.
            if let Some(t) = self.home_hold_task.take() {
                t.abort();
                // Invalidate any queued HomeHoldFired so a tap never also fires a
                // hold (race: released the instant the timer's message is in flight).
                self.home_hold_gen = sh.next_generation();
                match meta_tap_action(routed) {
                    MetaTapAction::HomeTap => sh.publish(Event::Intent("home-tap".into())),
                    MetaTapAction::ReplayToPad => {
                        self.forward_to_virtual_pad(InputEvent::new(EV_KEY, cfg::BTN_MODE, 1));
                        self.forward_to_virtual_pad(InputEvent::new(EV_KEY, cfg::BTN_MODE, 0));
                    }
                    MetaTapAction::Swallow => { /* keyboard-contract app: nothing */ }
                }
            }
            // else: the hold already fired — swallow the release (escape published).
        }
    }

    // --- combo buffer (per-presenter partial-combo suppression) -----------

    /// Gate one KEY event in the **Keyboard** presenter through the combo buffer
    /// (`arm_threshold = 1`: buffer from the first held participant — a media app
    /// tolerates the latency and this fully prevents media-key leaks). Forwards
    /// via [`Self::shell_emit_key_event`].
    fn gate_and_forward_shell_key(&mut self, sh: &mut Shared, code: u16, value: i32) {
        let action = state::combo_buffer_action(
            self.combo_armed,
            config::is_combo_participant(code),
            value,
            state::participant_held_count(&self.held_keys),
            1,
            state::any_combo_matched(&self.held_keys),
        );
        self.apply_combo_action(sh, Presenter::Keyboard, action, code, value);
    }

    /// Gate one KEY event in the **Game** presenter through the combo buffer
    /// (`arm_threshold = 2`: buffer only once a second participant is held, so
    /// single-button gameplay stays latency-free; the first participant of a pair
    /// may forward before arming, which a game tolerates). Forwards via
    /// [`Self::game_forward_key`].
    fn gate_and_forward_game_key(&mut self, sh: &mut Shared, code: u16, value: i32) {
        let action = state::combo_buffer_action(
            self.combo_armed,
            config::is_combo_participant(code),
            value,
            state::participant_held_count(&self.held_keys),
            2,
            state::any_combo_matched(&self.held_keys),
        );
        self.apply_combo_action(sh, Presenter::Game, action, code, value);
    }

    /// Carry out a [`state::ComboBufferAction`] for the given routed app presenter.
    /// The pure decision lives in `state`; this applies its side effects — the
    /// buffered-event replay into uinput/the vpad (the only non-pure part).
    fn apply_combo_action(
        &mut self,
        sh: &mut Shared,
        routed: Presenter,
        action: state::ComboBufferAction,
        code: u16,
        value: i32,
    ) {
        use state::ComboBufferAction as A;
        match action {
            A::Forward => self.forward_app_key(sh, routed, code, value),
            A::Buffer => {
                let was_armed = self.combo_armed;
                self.combo_buffer.push((code, value));
                self.combo_armed = true;
                // Arm the settle window only on the disarmed→armed edge — a fixed
                // window from the first buffered press bounds the latency.
                if !was_armed {
                    self.start_combo_guard(sh);
                }
            }
            A::Swallow => self.reset_combo_buffer(sh),
            A::ReplayThenForward => {
                // Disqualified by a non-participant: flush the buffer, then forward
                // the current (non-participant) event after it.
                self.replay_combo_buffer(sh, routed);
                self.disarm_combo_guard(sh);
                self.forward_app_key(sh, routed, code, value);
            }
            A::ReplayIncludingEvent => {
                // Disqualified by a participant release: the current event is part
                // of the (non-combo) sequence, so replay it with the rest.
                self.combo_buffer.push((code, value));
                self.replay_combo_buffer(sh, routed);
                self.disarm_combo_guard(sh);
            }
        }
    }

    /// Forward one buffered/live KEY event to the app via the routed presenter's
    /// path. Shell/Handoff are never armed, so they never reach here for a replay;
    /// guarded defensively.
    fn forward_app_key(&mut self, sh: &mut Shared, routed: Presenter, code: u16, value: i32) {
        match routed {
            Presenter::Keyboard => self.shell_emit_key_event(sh, code, value),
            Presenter::Game => self.game_forward_key(code, value),
            Presenter::Shell | Presenter::Handoff => {}
        }
    }

    /// Replay the buffered participants to the app (in arrival order) via the
    /// routed presenter's forward path, draining the buffer. Leaves `combo_armed`
    /// / the guard for the caller to clear (they always [`Self::disarm_combo_guard`]
    /// right after).
    fn replay_combo_buffer(&mut self, sh: &mut Shared, routed: Presenter) {
        let buffered = std::mem::take(&mut self.combo_buffer);
        for (code, value) in buffered {
            self.forward_app_key(sh, routed, code, value);
        }
    }

    /// Abort + invalidate the combo guard timer and mark the buffer disarmed. Does
    /// NOT touch buffer contents (callers either drained via replay or clear via
    /// [`Self::reset_combo_buffer`]).
    fn disarm_combo_guard(&mut self, sh: &mut Shared) {
        self.combo_armed = false;
        if let Some(t) = self.combo_guard_task.take() {
            t.abort();
        }
        // Bump the generation so any already-queued ComboGuardFired is ignored.
        self.combo_guard_gen = sh.next_generation();
    }

    /// Fully reset combo-buffer state: drop any buffered participants, disarm, and
    /// invalidate the guard timer. Called on a combo swallow and on every
    /// presenter/session/overlay transition so a partial sequence can never strand
    /// across a context change.
    fn reset_combo_buffer(&mut self, sh: &mut Shared) {
        self.combo_buffer.clear();
        self.disarm_combo_guard(sh);
    }

    fn handle_abs(&mut self, sh: &mut Shared, code: u16, value: i32) {
        match code {
            cfg::ABS_HAT0X => {
                if value == -1 {
                    sh.emit_key(cfg::KEY_LEFT, 1);
                    self.held_keys.insert(cfg::KEY_LEFT);
                } else if value == 1 {
                    sh.emit_key(cfg::KEY_RIGHT, 1);
                    self.held_keys.insert(cfg::KEY_RIGHT);
                } else {
                    sh.emit_key(cfg::KEY_LEFT, 0);
                    sh.emit_key(cfg::KEY_RIGHT, 0);
                    self.held_keys.remove(&cfg::KEY_LEFT);
                    self.held_keys.remove(&cfg::KEY_RIGHT);
                }
                if value != 0 {
                    sh.set_input_mode(InputMode::Controller);
                }
                self.notify_held_buttons(sh);
            }
            cfg::ABS_HAT0Y => {
                if value == -1 {
                    sh.emit_key(cfg::KEY_UP, 1);
                    self.held_keys.insert(cfg::KEY_UP);
                } else if value == 1 {
                    sh.emit_key(cfg::KEY_DOWN, 1);
                    self.held_keys.insert(cfg::KEY_DOWN);
                } else {
                    sh.emit_key(cfg::KEY_UP, 0);
                    sh.emit_key(cfg::KEY_DOWN, 0);
                    self.held_keys.remove(&cfg::KEY_UP);
                    self.held_keys.remove(&cfg::KEY_DOWN);
                }
                if value != 0 {
                    sh.set_input_mode(InputMode::Controller);
                }
                self.notify_held_buttons(sh);
            }
            cfg::ABS_X => self.handle_stick_axis(sh, value, Axis::X, cfg::KEY_LEFT, cfg::KEY_RIGHT),
            cfg::ABS_Y => self.handle_stick_axis(sh, value, Axis::Y, cfg::KEY_UP, cfg::KEY_DOWN),
            cfg::ABS_RX => self.handle_rstick_axis(sh, value, Axis::X),
            cfg::ABS_RY => self.handle_rstick_axis(sh, value, Axis::Y),
            cfg::ABS_Z => {
                let was = self.left_trigger_held;
                self.left_trigger_held = value > 100;
                if was != self.left_trigger_held {
                    self.notify_held_buttons(sh);
                }
            }
            cfg::ABS_RZ => {
                let was = self.right_trigger_held;
                self.right_trigger_held = value > 100;
                if was != self.right_trigger_held {
                    self.notify_held_buttons(sh);
                }
            }
            _ => {}
        }
    }

    fn handle_stick_axis(
        &mut self,
        sh: &mut Shared,
        value: i32,
        axis: Axis,
        neg_key: u16,
        pos_key: u16,
    ) {
        let (center, threshold) = match axis {
            Axis::X => (self.stick_center_x, self.stick_threshold_x),
            Axis::Y => (self.stick_center_y, self.stick_threshold_y),
        };
        let new_key = state::left_stick_target(value, center, threshold, neg_key, pos_key);
        let current = match axis {
            Axis::X => self.stick_x_key,
            Axis::Y => self.stick_y_key,
        };
        if new_key == current {
            return;
        }
        if let Some(k) = current {
            sh.emit_key(k, 0);
            self.cancel_stick_repeat(sh, axis);
        }
        match axis {
            Axis::X => self.stick_x_key = new_key,
            Axis::Y => self.stick_y_key = new_key,
        }
        if let Some(k) = new_key {
            sh.emit_key(k, 1);
            self.start_stick_repeat(sh, axis, k);
            sh.set_input_mode(InputMode::Controller);
        }
        self.notify_held_buttons(sh);
    }

    fn handle_rstick_axis(&mut self, sh: &mut Shared, value: i32, axis: Axis) {
        match axis {
            Axis::X => {
                self.rstick_raw_x = value;
                let new_dir =
                    state::rstick_x_dir(value, self.rstick_center_x, self.rstick_threshold_x);
                if new_dir != self.rstick_x_dir {
                    let old = self.rstick_x_dir;
                    self.rstick_x_dir = new_dir;
                    if new_dir.is_some() && old.is_none() {
                        sh.set_input_mode(InputMode::Mouse);
                    }
                    self.notify_held_buttons(sh);
                }
            }
            Axis::Y => {
                self.rstick_raw_y = value;
                let new_dir =
                    state::rstick_y_dir(value, self.rstick_center_y, self.rstick_threshold_y);
                if new_dir != self.rstick_y_dir {
                    let old = self.rstick_y_dir;
                    self.rstick_y_dir = new_dir;
                    if new_dir.is_some() && old.is_none() {
                        sh.set_input_mode(InputMode::Mouse);
                    }
                    self.notify_held_buttons(sh);
                }
            }
        }

        if self.has_rstick_deflection() {
            let running = self.mouse_task.as_ref().is_some_and(|t| !t.is_finished());
            if !running {
                let tx = sh.internal_tx.clone();
                let fd = self.fd;
                self.mouse_task = Some(tokio::spawn(async move {
                    loop {
                        if tx.send(Internal::MouseTick { fd }).await.is_err() {
                            break;
                        }
                        tokio::time::sleep(Duration::from_millis(config::MOUSE_POLL_MS)).await;
                    }
                }));
            }
        }
    }

    fn has_rstick_deflection(&self) -> bool {
        self.rstick_x_dir.is_some() || self.rstick_y_dir.is_some()
    }

    /// Compute and emit one mouse-poll tick. Returns false if the deflection has
    /// ended (the caller aborts the poll task).
    fn mouse_tick(&mut self, sh: &mut Shared) -> bool {
        if !self.has_rstick_deflection() {
            if let Some(t) = self.mouse_task.take() {
                t.abort();
            }
            return false;
        }
        let dx = state::compute_mouse_velocity(
            self.rstick_raw_x,
            self.rstick_center_x,
            self.rstick_threshold_x,
            self.rstick_half_range_x,
        );
        let dy = state::compute_mouse_velocity(
            self.rstick_raw_y,
            self.rstick_center_y,
            self.rstick_threshold_y,
            self.rstick_half_range_y,
        );
        sh.emit_mouse_move(dx, dy);
        true
    }

    fn start_stick_repeat(&mut self, sh: &mut Shared, axis: Axis, key: u16) {
        self.cancel_stick_repeat(sh, axis);
        let generation = sh.next_generation();
        match axis {
            Axis::X => self.stick_x_gen = generation,
            Axis::Y => self.stick_y_gen = generation,
        }
        let tx = sh.internal_tx.clone();
        let fd = self.fd;
        let handle = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(config::STICK_INITIAL_DELAY_MS)).await;
            loop {
                if tx
                    .send(Internal::StickRepeat {
                        fd,
                        axis,
                        key,
                        generation,
                    })
                    .await
                    .is_err()
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(config::STICK_REPEAT_INTERVAL_MS)).await;
            }
        });
        match axis {
            Axis::X => self.stick_x_repeat = Some(handle),
            Axis::Y => self.stick_y_repeat = Some(handle),
        }
    }

    fn cancel_stick_repeat(&mut self, sh: &mut Shared, axis: Axis) {
        match axis {
            Axis::X => self.cancel_stick_repeat_x(sh),
            Axis::Y => self.cancel_stick_repeat_y(sh),
        }
    }

    fn cancel_stick_repeat_x(&mut self, sh: &mut Shared) {
        if let Some(t) = self.stick_x_repeat.take() {
            t.abort();
            self.stick_x_gen = sh.next_generation();
        }
    }

    fn cancel_stick_repeat_y(&mut self, sh: &mut Shared) {
        if let Some(t) = self.stick_y_repeat.take() {
            t.abort();
            self.stick_y_gen = sh.next_generation();
        }
    }

    // --- combos (per-pad-complete; see module docs [critic P1]) ----------

    fn check_combo_start(&mut self, sh: &mut Shared) {
        if state::subset_held(&config::COMBO_KEYS, &self.held_keys) && self.combo_task.is_none() {
            let generation = sh.next_generation();
            self.combo_gen = generation;
            let tx = sh.internal_tx.clone();
            let fd = self.fd;
            self.combo_task = Some(tokio::spawn(async move {
                tokio::time::sleep(Duration::from_secs_f64(config::COMBO_HOLD_SECS)).await;
                let _ = tx
                    .send(Internal::ComboEndSessionFired { fd, generation })
                    .await;
            }));
        }
    }

    fn cancel_combo(&mut self, sh: &mut Shared) {
        if self.combo_task.is_some() && !state::subset_held(&config::COMBO_KEYS, &self.held_keys) {
            if let Some(t) = self.combo_task.take() {
                t.abort();
                self.combo_gen = sh.next_generation();
            }
        }
    }

    fn cancel_combo_unconditional(&mut self, sh: &mut Shared) {
        if let Some(t) = self.combo_task.take() {
            t.abort();
            self.combo_gen = sh.next_generation();
        }
    }

    fn check_quit_combo(&mut self, sh: &mut Shared) {
        if state::subset_held(&config::QUIT_COMBO_KEYS, &self.held_keys) {
            info!("Force-quit combo detected (Back+Home+LB+RB)");
            // Whenever a focused app owns the screen — a stream (Game/Handoff) or
            // a keyboard-contract app like Plex (Keyboard) — also send the quit
            // chord on the shared virtual keyboard, so a couch user who can't
            // reach a keyboard still has a controller escape from an app that
            // captured input. Only the Shell home screen has no app to quit.
            // (Pre-Phase-5 this keyed off the grab; the pad now stays grabbed in
            // every non-Handoff presenter, so it keys off the presenter instead.)
            // The chord is best-effort per app; the authoritative escape is the
            // `ComboForceQuit` event below, which the shell turns into a
            // force-quit/return-to-shell regardless of app.
            if presenter_owns_app(sh.presenter) {
                sh.send_moonlight_quit();
            }
            sh.publish(Event::ComboForceQuit);
        }
    }

    fn check_suspend_combo(&mut self, sh: &mut Shared) {
        if state::subset_held(&config::SUSPEND_COMBO_KEYS, &self.held_keys)
            && !state::subset_held(&config::QUIT_COMBO_KEYS, &self.held_keys)
        {
            info!("Suspend combo detected (LB+RB+Start)");
            sh.publish(Event::ComboSuspendStream);
        }
    }

    /// Arm the Meta (BTN_MODE / Guide) hold timer: sleep `meta_hold` then post
    /// [`Internal::HomeHoldFired`]. This is the tap-vs-hold discriminator for the
    /// Meta gesture in every grabbed presenter (and best-effort in an unpinned
    /// Handoff). The threshold is the `[input].meta_hold_ms` knob, not the old
    /// 2 s constant.
    fn start_home_hold(&mut self, sh: &mut Shared) {
        if let Some(t) = self.home_hold_task.take() {
            t.abort();
        }
        let generation = sh.next_generation();
        self.home_hold_gen = generation;
        let tx = sh.internal_tx.clone();
        let fd = self.fd;
        let hold = sh.meta_hold;
        self.home_hold_task = Some(tokio::spawn(async move {
            tokio::time::sleep(hold).await;
            let _ = tx.send(Internal::HomeHoldFired { fd, generation }).await;
        }));
    }

    /// Arm the combo settle-window timer (`combo_guard`): sleep then post
    /// [`Internal::ComboGuardFired`]. Started when the combo buffer arms; on fire
    /// the still-buffered participants (no combo completed) are replayed to the
    /// app. A fixed window from the arm point (not reset per buffered event), so
    /// the buffered-input latency is bounded by `combo_guard_ms`.
    fn start_combo_guard(&mut self, sh: &mut Shared) {
        if let Some(t) = self.combo_guard_task.take() {
            t.abort();
        }
        let generation = sh.next_generation();
        self.combo_guard_gen = generation;
        let tx = sh.internal_tx.clone();
        let fd = self.fd;
        let guard = sh.combo_guard;
        self.combo_guard_task = Some(tokio::spawn(async move {
            tokio::time::sleep(guard).await;
            let _ = tx.send(Internal::ComboGuardFired { fd, generation }).await;
        }));
    }

    // --- presenter entry/exit (Phase 5) ----------------------------------

    /// Enter the game presenter: create this pad's clean virtual gamepad (if it
    /// doesn't already have one) and register its fd so discovery never grabs it.
    /// The physical pad stays grabbed; subsequent events route through
    /// [`PadDevice::handle_game`] onto the virtual pad. Idempotent.
    fn enter_game(&mut self, sh: &mut Shared) {
        // A presenter transition: drop any partial combo sequence buffered under
        // the previous presenter so it can't strand or replay into the wrong app.
        self.reset_combo_buffer(sh);
        if self.virtual_pad.is_some() {
            return;
        }
        match build_virtual_pad(self.event_stream.device(), self.player_slot) {
            Ok(mut vpad) => {
                register_vpad_devnodes(&mut sh.reg, &mut vpad);
                // Capture the digital buttons held at this flip so they're
                // swallowed until released and pressed fresh (#295 follow-up).
                // Done ONLY on the transition that actually creates the vpad —
                // the early-return above guards the idempotent case, so a
                // redundant `enter_game` can't re-snapshot or clobber this. A
                // fresh (re)build starts from a clean mask before recapturing.
                self.masked_keys.clear();
                self.masked_keys.extend(self.held_keys.iter().copied());
                // Invariant: the flip-mask is only ever populated FROM held_keys,
                // so every masked code must be currently held. Holds by
                // construction here (clear + extend(held_keys)); the debug_assert
                // guards a future edit that seeds masked_keys from another source.
                // debug-only, matching the grab-invariant discipline: panics in
                // dev/test, compiled out in release (no error!/metric for this one).
                debug_assert!(
                    self.masked_keys.iter().all(|c| self.held_keys.contains(c)),
                    "flip-mask must be a subset of held_keys (masked={:?}, held={:?})",
                    self.masked_keys,
                    self.held_keys,
                );
                info!(
                    "Created virtual pad game-shell-virtual-pad-{} for slot {} ({})",
                    self.player_slot, self.player_slot, self.wire_id
                );
                self.virtual_pad = Some(vpad);
            }
            Err(e) => error!(
                "Failed to create virtual pad for slot {} ({}): {e}",
                self.player_slot, self.wire_id
            ),
        }
    }

    /// Leave the game presenter: drop this pad's virtual gamepad and forget its
    /// fd. The physical pad stays grabbed; events route back through the shell
    /// presenter ([`PadDevice::handle_shell`]). Idempotent.
    fn enter_shell(&mut self, sh: &mut Shared) {
        // Clear the flip-mask unconditionally so no masked state can leak across
        // a Game→Shell transition (#295 follow-up); the next `enter_game`
        // recaptures from `held_keys` at that flip.
        self.masked_keys.clear();
        // A presenter transition: drop any partial combo sequence buffered under
        // the previous presenter (Keyboard/Game) so it can't strand.
        self.reset_combo_buffer(sh);
        if let Some(mut vpad) = self.virtual_pad.take() {
            unregister_vpad_devnodes(&mut sh.reg, &mut vpad);
            info!(
                "Dropped virtual pad for slot {} ({})",
                self.player_slot, self.wire_id
            );
        }
    }

    // --- fleet outputs: LED / rumble / battery (ride-along, Phase 5.5) -----

    /// Light this pad's player-indicator LED to match its slot (#101 LED).
    ///
    /// Two backends, tried in order:
    ///   1. **EV_LED** — when the pad advertises `EV_LED` with an LED code equal
    ///      to the player slot (`LED_0..` semantics — LED code == player index).
    ///   2. **sysfs fallback** — xpad (Xbox 360) pads expose their player ring
    ///      via `/sys/class/leds/xpad*`, **not** `EV_LED`, so the EV_LED write is
    ///      a silent no-op on them. When EV_LED is unsupported we walk the pad's
    ///      `/sys/class/input/eventN/device` tree up to a `leds/` dir and write
    ///      the xpad brightness convention to `<ledsnode>/brightness`.
    ///
    /// Most other wired pads expose neither, so this still silently does nothing
    /// for them. On either success records `led_index` and publishes
    /// `pad:index:{id,index}`. Best-effort: a failed write never fails the pad.
    /// Idempotent for the same slot.
    fn set_player_led(&mut self, sh: &mut Shared) {
        if self.led_index == Some(self.player_slot) {
            return; // already lit for this slot
        }
        // Cap-gate: the pad must support EV_LED with an LED code == player slot.
        let ev_led_supported = self
            .event_stream
            .device()
            .supported_leds()
            .is_some_and(|leds| leds.iter().any(|l| l.0 == self.player_slot as u16));
        if ev_led_supported {
            // Light the LED whose code == the player slot (LED_0.. convention).
            let ev = InputEvent::new(EV_LED, self.player_slot as u16, 1);
            match self.event_stream.device_mut().send_events(&[ev]) {
                Ok(()) => {
                    self.led_index = Some(self.player_slot);
                    info!(
                        "Lit player LED {} for slot {} ({})",
                        self.player_slot, self.player_slot, self.wire_id
                    );
                    sh.publish(Event::PadIndex(crate::protocol::pad_index_json(
                        &self.wire_id,
                        self.player_slot,
                    )));
                }
                Err(e) => warn!("Failed to set player LED on {}: {e}", self.wire_id),
            }
            return;
        }

        // EV_LED unsupported: try the sysfs xpad fallback (Xbox 360 ring).
        match set_player_led_sysfs(&self.path, self.player_slot) {
            Ok(Some(node)) => {
                self.led_index = Some(self.player_slot);
                info!(
                    "player LED set via sysfs {} for slot {} ({})",
                    node.display(),
                    self.player_slot,
                    self.wire_id
                );
                sh.publish(Event::PadIndex(crate::protocol::pad_index_json(
                    &self.wire_id,
                    self.player_slot,
                )));
            }
            // No leds node found for this pad: a true no-op (most pads). Quiet at
            // debug so the common case doesn't spam logs.
            Ok(None) => debug!(
                "no sysfs leds node for pad slot {} ({}); player LED unset",
                self.player_slot, self.wire_id
            ),
            Err(e) => warn!(
                "Failed to set player LED via sysfs on {}: {e}",
                self.wire_id
            ),
        }
    }

    /// Fire a rumble (FF_RUMBLE) effect for `ms` milliseconds (#99).
    ///
    /// Cap-gated: a no-op unless the pad advertises `EV_FF` with `FF_RUMBLE`.
    /// The effect is uploaded lazily and cached; a repeat at the same duration
    /// replays the cached effect, a different duration re-uploads. A zero `ms`
    /// is treated as a no-op (nothing to play). Failures are logged and
    /// swallowed — rumble is best-effort and must never wedge input handling.
    fn rumble(&mut self, ms: u16) {
        if ms == 0 {
            return;
        }
        // Cap-gate: require EV_FF advertising FF_RUMBLE.
        let supports_rumble = self
            .event_stream
            .device()
            .supported_ff()
            .is_some_and(|ff| ff.contains(FFEffectCode::FF_RUMBLE));
        if !supports_rumble {
            return; // no force feedback on this pad -> no-op
        }
        // (Re)upload the effect if we don't have one or the duration changed.
        if self.ff_effect.is_none() || self.ff_length_ms != ms {
            let data = rumble_effect_data(ms);
            match self.event_stream.device_mut().upload_ff_effect(data) {
                Ok(effect) => {
                    self.ff_effect = Some(effect);
                    self.ff_length_ms = ms;
                }
                Err(e) => {
                    warn!("Failed to upload rumble effect on {}: {e}", self.wire_id);
                    return;
                }
            }
        }
        if let Some(effect) = self.ff_effect.as_mut() {
            if let Err(e) = effect.play(1) {
                warn!("Failed to play rumble on {}: {e}", self.wire_id);
            }
        }
    }

    /// Re-read this pad's battery from sysfs and, if it changed, emit
    /// `pad:battery:{id,level,charging}` (#100). A no-op for wired pads (no
    /// matching `power_supply`). Used on the (synchronous) pad-join path; the
    /// periodic poll instead offloads the sysfs read and calls [`apply_battery`].
    fn poll_battery(&mut self, sh: &mut Shared) {
        self.apply_battery(sh, read_pad_battery(&self.name));
    }

    /// Apply an already-read battery state: if it changed, emit
    /// `pad:battery:{id,level,charging}` (#100). `None` (wired pad / no supply)
    /// is left as the current state and emits nothing. Pure of I/O so the sysfs
    /// read can be offloaded to the blocking pool by the periodic poll arm.
    fn apply_battery(&mut self, sh: &mut Shared, state: Option<BatteryState>) {
        let Some(state) = state else {
            return; // no battery reported (wired pad) -> nothing to emit
        };
        if self.battery == Some(state) {
            return; // unchanged
        }
        self.battery = Some(state);
        sh.publish(Event::PadBattery(crate::protocol::pad_battery_json(
            &self.wire_id,
            state.level,
            state.charging,
        )));
    }
}

/// Build the `FFEffectData` for a tasteful one-shot rumble of `ms` milliseconds.
/// A medium dual-motor rumble (both the heavy and light motors) that auto-stops
/// after `length`; the effect plays once via `FFEffect::play(1)`.
fn rumble_effect_data(ms: u16) -> FFEffectData {
    FFEffectData {
        direction: 0,
        trigger: FFTrigger::default(),
        replay: FFReplay {
            length: ms,
            delay: 0,
        },
        kind: FFEffectKind::Rumble {
            strong_magnitude: 0x8000,
            weak_magnitude: 0x8000,
        },
    }
}

/// Best-effort sysfs battery read for a gamepad whose device name is `pad_name`.
///
/// Scans `/sys/class/power_supply/*` for a `Battery`-type supply that looks like
/// a game controller (its `model_name` matches the pad name, or its supply name
/// carries a controller marker). Returns `(level, charging)` or `None` when no
/// matching battery is found — the normal case for a wired pad. Conservative on
/// purpose: a non-match degrades to "no battery", never a wrong reading.
fn read_pad_battery(pad_name: &str) -> Option<BatteryState> {
    let dir = std::fs::read_dir("/sys/class/power_supply").ok()?;
    let pad_lower = pad_name.to_lowercase();
    for entry in dir.flatten() {
        let base = entry.path();
        // Only battery-type supplies.
        let kind = read_sysfs_trimmed(&base.join("type")).unwrap_or_default();
        if !kind.eq_ignore_ascii_case("Battery") {
            continue;
        }
        let supply_name = entry.file_name().to_string_lossy().to_lowercase();
        let model = read_sysfs_trimmed(&base.join("model_name"))
            .unwrap_or_default()
            .to_lowercase();
        // Match on the pad's model name, or a controller marker in the supply
        // name (joydev/xpad/sony/nintendo power supplies use such names).
        let looks_like_pad = (!model.is_empty()
            && (model.contains(&pad_lower) || pad_lower.contains(&model)))
            || [
                "controller",
                "gamepad",
                "xpad",
                "sony",
                "nintendo",
                "joypad",
            ]
            .iter()
            .any(|m| supply_name.contains(m));
        if !looks_like_pad {
            continue;
        }
        let Some(capacity) =
            read_sysfs_trimmed(&base.join("capacity")).and_then(|s| s.parse::<u32>().ok())
        else {
            continue;
        };
        let status = read_sysfs_trimmed(&base.join("status")).unwrap_or_default();
        let charging =
            status.eq_ignore_ascii_case("Charging") || status.eq_ignore_ascii_case("Full");
        return Some(BatteryState {
            level: capacity.min(100) as u8,
            charging,
        });
    }
    None
}

/// Read a sysfs attribute file and return its trimmed contents, or `None` on any
/// I/O error (an absent attribute is common and not worth logging).
fn read_sysfs_trimmed(path: &std::path::Path) -> Option<String> {
    std::fs::read_to_string(path)
        .ok()
        .map(|s| s.trim().to_string())
}

/// LED driver convention inferred from a sysfs leds node name.
///
/// Classified by the NAME of the directory under `leds/` (the node itself, not
/// its parent), using substring matching so both the simple `xpad0` form and the
/// BDADDR-prefixed `0005:054C:0CE6.000D:white:player-1` form are handled
/// correctly across kernel versions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LedConvention {
    /// Xbox / xpad ring-LED brightness: write `6 + slot` to `brightness`.
    Xpad,
    /// Sony DualSense/DualShock4 `*:white:player-N` per-slot player indicators.
    /// Write `1` to the matching `player-<slot+1>` sibling, `0` to the others.
    SonyPlayer,
    /// Sony DualSense RGB lightbar (`*:rgb:indicator`).
    /// Write a per-slot solid colour via `multi_intensity` (R G B).
    SonyRgb,
}

/// Classify a leds node by its directory name.
///
/// This is a pure function with no I/O so it can be unit-tested without real
/// sysfs. Matching is by substring so it is robust across kernel naming variants
/// (short `xpad0`, BDADDR-prefixed `0005:054C:...:white:player-1`, etc.).
fn classify_led_node(node_name: &str) -> LedConvention {
    if node_name.contains(":white:player-") {
        LedConvention::SonyPlayer
    } else if node_name.contains(":rgb:indicator") {
        LedConvention::SonyRgb
    } else {
        // Covers bare `xpad0`, `xpad1`, and any plain-brightness ring node.
        LedConvention::Xpad
    }
}

/// Per-slot RGB colour table for the Sony RGB lightbar.
///
/// P1 blue, P2 red, P3 green, P4 magenta — matches the DualSense player
/// colour conventions used by hid-playstation when the OS does not override
/// them. Written as `R G B` (space-separated, decimal) to `multi_intensity`.
const SONY_RGB_COLORS: [(u8, u8, u8); 4] = [
    (0, 0, 255),   // P1 blue
    (255, 0, 0),   // P2 red
    (0, 255, 0),   // P3 green
    (255, 0, 255), // P4 magenta
];

/// Choose the best-correlated leds node for a specific pad from a set of
/// candidates found under a shared `leds/` dir.
///
/// Selection criterion: the candidate whose canonicalized path shares the
/// **longest common prefix** with `pad_device_path` is the physically nearest
/// node for that pad. Ties are broken by sorted node name (lexicographic
/// ascending), so `xpad0` beats `xpad1` when prefixes are equal, and the
/// choice is always deterministic regardless of `read_dir` order.
///
/// Returns `None` when `candidates` is empty.
///
/// This is a pure function with no I/O so it can be unit-tested without real
/// sysfs. All path resolution must be done by the caller before passing in.
fn pick_leds_node(pad_device_path: &std::path::Path, candidates: &[PathBuf]) -> Option<PathBuf> {
    if candidates.is_empty() {
        return None;
    }

    // Count the number of path components shared between `a` and `b`.
    fn shared_prefix_len(a: &std::path::Path, b: &std::path::Path) -> usize {
        a.components()
            .zip(b.components())
            .take_while(|(x, y)| x == y)
            .count()
    }

    candidates
        .iter()
        .max_by(|a, b| {
            let la = shared_prefix_len(pad_device_path, a);
            let lb = shared_prefix_len(pad_device_path, b);
            la.cmp(&lb)
                // Tie-break: sorted name — choose the *smaller* name (xpad0 < xpad1)
                // by reversing the secondary comparison (max_by picks the larger).
                .then_with(|| {
                    let na = a.file_name().unwrap_or_default();
                    let nb = b.file_name().unwrap_or_default();
                    nb.cmp(na) // reversed so max_by picks the lexicographically smallest
                })
        })
        .cloned()
}

/// sysfs player-LED fallback for pads that expose their player ring via
/// `/sys/class/leds` instead of `EV_LED`.
///
/// Two driver families are handled:
///
/// - **xpad (Xbox 360/One)** — a single `xpad<N>` node under `leds/` with a
///   plain `brightness` attribute. Write `6 + slot.min(3)` for SOLID P1..P4.
/// - **Sony DualSense/DualShock4** — either `*:white:player-N` per-slot nodes
///   (write `1` to the matching player node, `0` to siblings) or an
///   `*:rgb:indicator` lightbar node (write a per-slot colour to
///   `multi_intensity`).
///
/// Multi-pad correlation: when multiple candidates exist under `leds/`, the
/// one with the longest shared canonical-path prefix with the pad's own sysfs
/// device path is chosen (ties broken by sorted name), so two identical xpads
/// each light their own ring instead of racing on the first entry.
///
/// Returns `Ok(Some(node))` on a successful primary write, `Ok(None)` when no
/// usable leds node/convention is found, or `Err` on an unexpected FS error
/// (the caller logs and swallows it).
fn set_player_led_sysfs(devnode: &std::path::Path, slot: u8) -> std::io::Result<Option<PathBuf>> {
    // /dev/input/eventN -> "eventN"
    let Some(event_name) = devnode.file_name().and_then(|n| n.to_str()) else {
        return Ok(None);
    };
    if !event_name.starts_with("event") {
        return Ok(None);
    }
    // /sys/class/input/eventN/device is a symlink into the device tree; resolve
    // it so we can walk real parent dirs.
    let device_link = PathBuf::from("/sys/class/input")
        .join(event_name)
        .join("device");
    let Ok(pad_device_path) = std::fs::canonicalize(&device_link) else {
        return Ok(None);
    };
    let mut dir = pad_device_path.clone();
    // Walk up to a parent containing a `leds/` subdir. Bound the climb so a
    // surprising tree can't loop forever.
    let (leds_dir, chosen_node) = loop {
        let leds = dir.join("leds");
        if leds.is_dir() {
            // Collect ALL entries, sort by name for determinism, then delegate
            // correlation to the pure helper.
            let mut candidates: Vec<PathBuf> = std::fs::read_dir(&leds)
                .ok()
                .into_iter()
                .flatten()
                .filter_map(|e| e.ok())
                .map(|e| e.path())
                .collect();
            candidates.sort_by(|a, b| {
                a.file_name()
                    .unwrap_or_default()
                    .cmp(b.file_name().unwrap_or_default())
            });
            match pick_leds_node(&pad_device_path, &candidates) {
                Some(node) => break (leds, node),
                None => return Ok(None), // empty leds dir
            }
        }
        match dir.parent() {
            // Stop at the sysfs root or filesystem root.
            Some(p) if p != std::path::Path::new("/sys") && p != std::path::Path::new("/") => {
                dir = p.to_path_buf();
            }
            _ => return Ok(None),
        }
    };

    let node_name = chosen_node
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("");
    match classify_led_node(node_name) {
        LedConvention::Xpad => {
            // 6..=9 = SOLID P1..P4; cap the slot at P4.
            let brightness = 6u8 + slot.min(3);
            std::fs::write(chosen_node.join("brightness"), brightness.to_string())?;
            Ok(Some(chosen_node))
        }
        LedConvention::SonyPlayer => {
            // Write `1` to the player-<slot+1> sibling and `0` to the others.
            // Build the target player index (1-based, capped at 4).
            let target_player = slot.min(3) + 1;
            // Scan all `*:white:player-N` siblings in the same leds/ dir.
            let siblings: Vec<PathBuf> = std::fs::read_dir(&leds_dir)
                .ok()
                .into_iter()
                .flatten()
                .filter_map(|e| e.ok())
                .map(|e| e.path())
                .filter(|p| {
                    p.file_name()
                        .and_then(|n| n.to_str())
                        .map(|n| n.contains(":white:player-"))
                        .unwrap_or(false)
                })
                .collect();
            if siblings.is_empty() {
                return Ok(None);
            }
            let mut primary_node: Option<PathBuf> = None;
            for sibling in &siblings {
                let sib_name = sibling.file_name().and_then(|n| n.to_str()).unwrap_or("");
                // Extract the player number from the suffix `player-N`.
                let player_num: Option<u8> = sib_name
                    .rfind(":white:player-")
                    .and_then(|pos| sib_name[pos + ":white:player-".len()..].parse().ok());
                let value = if player_num == Some(target_player) {
                    primary_node = Some(sibling.clone());
                    "1"
                } else {
                    "0"
                };
                // Best-effort: ignore individual write errors.
                let _ = std::fs::write(sibling.join("brightness"), value);
            }
            Ok(primary_node)
        }
        LedConvention::SonyRgb => {
            let (r, g, b) = SONY_RGB_COLORS[slot.min(3) as usize];
            // Try `multi_intensity` first (the standard RGB LED class
            // attribute); fall back gracefully for older kernels.
            let mi_path = chosen_node.join("multi_intensity");
            if mi_path.exists() {
                let _ = std::fs::write(&mi_path, format!("{r} {g} {b}"));
                Ok(Some(chosen_node))
            } else {
                // Unrecognized interface: leave the lightbar unset.
                Ok(None)
            }
        }
    }
}

#[cfg(test)]
mod led_tests {
    use super::*;
    use std::path::PathBuf;

    // ---- classify_led_node -------------------------------------------------

    #[test]
    fn classify_xpad_bare() {
        assert_eq!(classify_led_node("xpad0"), LedConvention::Xpad);
        assert_eq!(classify_led_node("xpad1"), LedConvention::Xpad);
        // Plain brightness node with no recognized suffix -> Xpad default.
        assert_eq!(classify_led_node("input5::green:led0"), LedConvention::Xpad);
    }

    #[test]
    fn classify_sony_player() {
        // Short form (some kernel versions).
        assert_eq!(
            classify_led_node("input5::white:player-1"),
            LedConvention::SonyPlayer
        );
        // BDADDR-prefixed form (hid-playstation / kernel >= 6.3).
        assert_eq!(
            classify_led_node("0005:054C:0CE6.000D:white:player-1"),
            LedConvention::SonyPlayer
        );
        assert_eq!(
            classify_led_node("0005:054C:0CE6.000D:white:player-4"),
            LedConvention::SonyPlayer
        );
    }

    #[test]
    fn classify_sony_rgb() {
        assert_eq!(
            classify_led_node("input5::rgb:indicator"),
            LedConvention::SonyRgb
        );
        assert_eq!(
            classify_led_node("0005:054C:0CE6.000D:rgb:indicator"),
            LedConvention::SonyRgb
        );
    }

    // ---- pick_leds_node ----------------------------------------------------

    #[test]
    fn pick_empty_candidates_returns_none() {
        let pad = PathBuf::from("/sys/devices/pci0000:00/usb1/1-1/input/input5");
        assert_eq!(pick_leds_node(&pad, &[]), None);
    }

    #[test]
    fn pick_single_candidate_is_returned() {
        let pad = PathBuf::from("/sys/devices/pci0000:00/usb1/1-1/input/input5");
        let candidates = vec![PathBuf::from("/sys/devices/pci0000:00/usb1/1-1/leds/xpad0")];
        assert_eq!(
            pick_leds_node(&pad, &candidates),
            Some(PathBuf::from("/sys/devices/pci0000:00/usb1/1-1/leds/xpad0",))
        );
    }

    /// Two identical xpads on different USB ports: pad1 at usb1/1-1, pad2 at
    /// usb1/1-2. The leds nodes live under each pad's own USB subtree.
    #[test]
    fn pick_correlates_to_correct_pad() {
        let pad = PathBuf::from("/sys/devices/pci0000:00/usb1/1-1/input/input5");
        let candidates = vec![
            // This node is under a DIFFERENT USB port (1-2).
            PathBuf::from("/sys/devices/pci0000:00/usb1/1-2/leds/xpad1"),
            // This node is under the SAME USB port (1-1).
            PathBuf::from("/sys/devices/pci0000:00/usb1/1-1/leds/xpad0"),
        ];
        assert_eq!(
            pick_leds_node(&pad, &candidates),
            Some(PathBuf::from("/sys/devices/pci0000:00/usb1/1-1/leds/xpad0",))
        );
    }

    /// When both candidates share an equal-length prefix with the pad path
    /// (e.g. they live under a shared ancestor), the lexicographically first
    /// name wins (xpad0 before xpad1).
    #[test]
    fn pick_tiebreak_by_sorted_name() {
        let pad = PathBuf::from("/sys/devices/pci0000:00/usb1/1-1/input/input5");
        let candidates = vec![
            PathBuf::from("/sys/devices/pci0000:00/usb1/leds/xpad1"),
            PathBuf::from("/sys/devices/pci0000:00/usb1/leds/xpad0"),
        ];
        assert_eq!(
            pick_leds_node(&pad, &candidates),
            Some(PathBuf::from("/sys/devices/pci0000:00/usb1/leds/xpad0",))
        );
    }

    /// The single-xpad common case (PR #103 regression guard): one candidate,
    /// one pad -- must still be selected.
    #[test]
    fn pick_single_xpad_regression() {
        let pad = PathBuf::from("/sys/devices/pci0000:00/usb1/1-3/input/input2");
        let candidates = vec![PathBuf::from("/sys/devices/pci0000:00/usb1/1-3/leds/xpad0")];
        let result = pick_leds_node(&pad, &candidates);
        assert!(
            result.is_some(),
            "single-xpad path must be selected, got None"
        );
        assert_eq!(
            result.unwrap().file_name().and_then(|n| n.to_str()),
            Some("xpad0")
        );
    }
}

/// Build a clean per-player virtual gamepad mirroring the physical pad's
/// capabilities (Phase 5). The virtual device advertises the same key set,
/// absolute axes (with the source's calibration), and `input_id` so a game
/// recognizes `game-shell-virtual-pad-<slot>` as the same controller model —
/// minus the daemon's internal Home/combo synthesis, which never reaches it.
///
/// We deliberately copy Home (`BTN_MODE`) into the *capability* set so the
/// virtual pad's profile matches the physical one, but `handle_game` never
/// forwards a Home event, so the game still never sees a Home press.
fn build_virtual_pad(src: &Device, slot: u8) -> std::io::Result<VirtualDevice> {
    let name = format!("game-shell-virtual-pad-{slot}");

    let keys: AttributeSet<KeyCode> = src
        .supported_keys()
        .into_iter()
        .flat_map(|set| set.iter())
        .collect();

    let mut builder = VirtualDevice::builder()?
        .name(&name)
        .input_id(src.input_id())
        .with_keys(&keys)?;

    // Copy each absolute axis with the source's absinfo (calibration), so the
    // virtual pad reports identical ranges/deadzones to the game.
    if let Ok(absinfo) = src.get_absinfo() {
        for (code, info) in absinfo {
            let setup = UinputAbsSetup::new(AbsoluteAxisCode(code.0), info);
            builder = builder.with_absolute_axis(&setup)?;
        }
    }

    let vpad = builder.build()?;
    Ok(vpad)
}

/// Register a virtual pad's `/dev/input/eventN` devnode(s) as daemon-owned so
/// fleet discovery skips it. The devnode is the identity that survives
/// `evdev::enumerate` reopening the device (the raw fd is not), which is what
/// discovery actually compares against — a virtual pad copies the physical
/// pad's `input_id`, so without this it would pass the DB gate and be grabbed
/// as a bogus pad on the next discovery poll.
fn register_vpad_devnodes(reg: &mut VirtualRegistry, vpad: &mut VirtualDevice) {
    // The kernel may not have created /dev/input/eventN the instant build()
    // returns; without the node we can't claim ownership and the 2s discovery
    // poll could briefly see the new pad as a candidate (#108). Retry briefly
    // (~100ms total) until the node appears.
    for attempt in 0..10 {
        match vpad.enumerate_dev_nodes_blocking() {
            Ok(nodes) => {
                let mut any = false;
                for node in nodes.flatten() {
                    reg.register(node);
                    any = true;
                }
                if any {
                    return;
                }
            }
            Err(e) => {
                warn!("could not enumerate virtual pad devnodes for ownership: {e}");
                return;
            }
        }
        if attempt < 9 {
            std::thread::sleep(Duration::from_millis(10));
        }
    }
    warn!("virtual pad devnode not present after retries; discovery may briefly see it as a candidate");
}

/// Forget a virtual pad's devnode(s) on teardown (re-enumerates the same paths
/// while the device is still alive).
fn unregister_vpad_devnodes(reg: &mut VirtualRegistry, vpad: &mut VirtualDevice) {
    if let Ok(nodes) = vpad.enumerate_dev_nodes_blocking() {
        for node in nodes.flatten() {
            reg.unregister(&node);
        }
    }
}

/// The gamepad fleet: physical pads keyed by raw fd, plus the stable player-slot
/// allocator (#101).
struct Fleet {
    pads: HashMap<RawFd, PadDevice>,
    slots: SlotAllocator,
}

impl Fleet {
    fn new() -> Fleet {
        Fleet {
            pads: HashMap::new(),
            slots: SlotAllocator::new(),
        }
    }

    /// Fleet aggregate for the `status` reply: connected if any pad is present;
    /// the second field reflects the **presenter** (`grab`→shell→`grabbed`,
    /// `release`→game→`released`), NOT the physical EVIOCGRAB — the pad now stays
    /// grabbed in both modes (Phase 5), so keying `status` off the physical grab
    /// would always report `grabbed` and break the `release` UI semantics that
    /// `ControllerSettings.qml` reads. For a single pad in the shell presenter
    /// this is byte-identical to the pre-fleet `connected:grabbed`.
    ///
    /// [`Presenter::Keyboard`] reports `grabbed` too: like Shell, the daemon is
    /// actively translating the pad (to keyboard/mouse for the focused app) and
    /// holds no virtual pad — the controller drives a UI, it is not "released" to
    /// a game. Only [`Presenter::Game`]/[`Presenter::Handoff`] report released.
    fn status_string(&self, presenter: Presenter) -> String {
        let connected = !self.pads.is_empty();
        let grabbed = matches!(presenter, Presenter::Shell | Presenter::Keyboard);
        resp_status(connected, grabbed)
    }

    /// True if any pad currently holds the Home (`BTN_MODE`) button. Used to
    /// clear the fleet-level Home-hold latch.
    fn any_holds_home(&self) -> bool {
        self.pads
            .values()
            .any(|p| p.held_keys.contains(&cfg::BTN_MODE))
    }

    /// Find a pad by its stable wire id (for the `rumble` command, #99). Linear
    /// scan — the fleet is tiny (a handful of pads) so a map keyed by wire id
    /// isn't worth the extra bookkeeping alongside the fd-keyed map.
    fn find_by_wire_id_mut(&mut self, wire_id: &str) -> Option<&mut PadDevice> {
        self.pads.values_mut().find(|p| p.wire_id == wire_id)
    }

    /// The `get-pads` reply: one JSON object per pad in ascending player-slot
    /// order.
    fn pads_json(&self) -> String {
        let mut pads: Vec<&PadDevice> = self.pads.values().collect();
        pads.sort_by_key(|p| p.player_slot);
        let rows: Vec<(String, u8, String, bool)> = pads
            .iter()
            .map(|p| (p.wire_id.clone(), p.player_slot, p.name.clone(), p.grabbed))
            .collect();
        resp_pads(&rows)
    }
}

/// Parse `/proc/bus/input/devices` into a map from event-node name (e.g.
/// `event18`) to that device's full handler list (e.g. `["event18", "js0"]`).
///
/// The file is blocks separated by blank lines; the `H: Handlers=...` line lists
/// the device's handlers (event node, `js*`, `mouseN`, `kbd`, …). We key each
/// block by its `eventN` handler so the evdev enumeration (which yields devnodes)
/// can recover the `js*` handlers a bare evdev `Device` doesn't expose. A missing
/// or unreadable file yields an empty map (the enumerator still lists event-node
/// handlers it derives directly).
fn parse_proc_input_handlers() -> HashMap<String, Vec<String>> {
    let mut map = HashMap::new();
    let Ok(text) = std::fs::read_to_string("/proc/bus/input/devices") else {
        return map;
    };
    for block in text.split("\n\n") {
        let mut handlers: Vec<String> = Vec::new();
        for line in block.lines() {
            if let Some(rest) = line.strip_prefix("H: Handlers=") {
                handlers = rest.split_whitespace().map(|h| h.to_string()).collect();
            }
        }
        if let Some(event) = handlers.iter().find(|h| h.starts_with("event")).cloned() {
            map.insert(event, handlers);
        }
    }
    map
}

/// Build the `list-input-devices` reply (#97): EVERY controller-like input
/// device on the host — anything that advertises `BTN_SOUTH` or carries a `js*`
/// handler — as a compact JSON array, including ungrabbed and virtual devices.
///
/// This is a diagnostics enumerator (it replaces `ControllerSettings`'
/// `/proc/bus/input/devices` python reader), distinct from `get-pads` (the
/// grabbed fleet only). `grabbed` is `true` only for devices whose devnode path
/// the fleet currently owns. Devices are returned in ascending devnode-path
/// order for a stable wire.
///
/// Called via `spawn_blocking` from the `Control::ListInputDevices` arm (#108)
/// so `evdev::enumerate()` does not stall the input runtime. The caller collects
/// the fleet's grabbed paths before the blocking boundary and passes them in.
fn list_input_devices_with(grabbed_paths: HashSet<PathBuf>) -> String {
    let proc_handlers = parse_proc_input_handlers();

    let mut devices: Vec<(PathBuf, Device)> = evdev::enumerate().collect();
    devices.sort_by(|a, b| a.0.cmp(&b.0));

    let mut rows: Vec<crate::protocol::InputDeviceInfo> = Vec::new();
    for (path, dev) in devices {
        let event_name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_string();
        // Handlers from /proc keyed by the event node; fall back to just the
        // event node name when /proc didn't have the block.
        let handlers = proc_handlers.get(&event_name).cloned().unwrap_or_else(|| {
            if event_name.is_empty() {
                Vec::new()
            } else {
                vec![event_name.clone()]
            }
        });
        let has_btn_south = dev
            .supported_keys()
            .is_some_and(|keys| keys.contains(KeyCode::BTN_SOUTH));
        let has_js = handlers.iter().any(|h| h.starts_with("js"));
        // Controller-like: BTN_SOUTH OR a js* handler. Ungrabbed + virtual ones
        // are intentionally included (this is a diagnostics enumerator).
        if !has_btn_south && !has_js {
            continue;
        }
        let id = dev.input_id();
        rows.push(crate::protocol::InputDeviceInfo {
            name: dev.name().unwrap_or("unknown").to_string(),
            path: path.to_string_lossy().to_string(),
            vendor: id.vendor(),
            product: id.product(),
            phys: dev.physical_path().unwrap_or("").to_string(),
            handlers,
            grabbed: grabbed_paths.contains(&path),
        });
    }
    crate::protocol::resp_input_devices(&rows)
}

/// Supervise the input runtime: run the event loop, and on a panic respawn a
/// fresh runtime a bounded number of times before staying **dead but detectable**.
///
/// A panic in [`run`]'s select loop used to silently kill the input OS thread and
/// drop the moved `control_rx`, after which every input-runtime IPC command
/// degraded with no recovery short of a whole-daemon restart — the "stuck
/// controller, only a daemon restart fixes it" class. The supervisor breaks that:
///
/// * `control_rx` (the single-owner control channel) plus the shared handles
///   (`events`, `config_changed`, `metrics`, `active_window_rx`) are RETAINED here
///   and re-passed on each attempt, so a panic never drops the control channel.
/// * Each attempt builds a fresh [`Fleet`] INSIDE [`run`]. A panic that unwinds out
///   of the loop drops that `Fleet` (and every [`PadDevice`]); dropping a pad closes
///   its evdev fd, and the kernel `EVIOCGRAB` is tied to the fd — so an fd close is
///   an implicit ungrab. That is what releases the grabs on panic; the respawned
///   attempt re-discovers and re-grabs cleanly. (There is no `panic="abort"` in
///   Cargo.toml, so unwinding — hence [`std::panic::catch_unwind`] — works.)
///
/// Bounded retries with a short growing backoff; after they are exhausted the
/// supervisor sets the up-gauge to 0 and returns rather than looping forever or
/// aborting the process — the daemon stays alive so `/metrics` and the IPC wire
/// (`error:input-runtime-down`) keep reporting the death. A clean (non-panic)
/// return from [`run`] is a normal shutdown (`Control::Shutdown` or a closed
/// control channel) and exits the supervisor WITHOUT counting a restart.
pub async fn run_supervised(
    mut control_rx: mpsc::Receiver<Control>,
    events: broadcast::Sender<Event>,
    config_changed: std::sync::Arc<tokio::sync::Notify>,
    metrics: std::sync::Arc<crate::metrics::Metrics>,
    active_window_rx: watch::Receiver<String>,
) {
    // Bounded respawns: enough to ride out a transient fault, few enough that a
    // hard-looping panic gives up quickly and stays visibly down.
    const MAX_RESTARTS: u32 = 3;
    let mut restarts: u32 = 0;
    loop {
        metrics.set_runtime_up(true);
        // `AssertUnwindSafe`: `run` borrows `&mut control_rx` and clones the shared
        // handles, none of which observe a broken invariant across the unwind — the
        // Fleet is created fresh inside `run` and dropped by the unwind, and the
        // control channel is just a queue. A clean return = normal shutdown.
        let outcome = AssertUnwindSafe(run(
            &mut control_rx,
            events.clone(),
            std::sync::Arc::clone(&config_changed),
            std::sync::Arc::clone(&metrics),
            active_window_rx.clone(),
        ))
        .catch_unwind()
        .await;

        match outcome {
            Ok(()) => {
                // Normal shutdown (Control::Shutdown / control channel closed).
                metrics.set_runtime_up(false);
                return;
            }
            Err(payload) => {
                // The loop body panicked and unwound: the Fleet was dropped, so
                // every pad's fd is closed and its EVIOCGRAB released. The up-gauge
                // drops to 0 on EVERY caught panic (correct in both cases below).
                let msg = panic_payload_str(payload.as_ref());
                metrics.set_runtime_up(false);
                restarts += 1;
                if restarts > MAX_RESTARTS {
                    error!(
                        panic = %msg,
                        restarts,
                        "input runtime panicked and exhausted restarts; staying down (up-gauge 0, IPC replies error:input-runtime-down)"
                    );
                    // Terminal panic: no respawn happens here, so do NOT count it
                    // as a restart — the counter means actual respawns, not panics.
                    // Stay dead but detectable — do NOT abort the process.
                    return;
                }
                // A respawn WILL occur (attempts remain): count it now, so
                // `game_shell_input_runtime_restarts_total` advances exactly once
                // per actual re-invocation of `run` (never on the terminal panic).
                metrics.inc_runtime_restarts();
                // Growing backoff: 200ms, 400ms, 800ms.
                let backoff = Duration::from_millis(200u64 << (restarts - 1));
                error!(
                    panic = %msg,
                    restart = restarts,
                    backoff_ms = backoff.as_millis() as u64,
                    "input runtime panicked; respawning after backoff"
                );
                tokio::time::sleep(backoff).await;
            }
        }
    }
}

/// Extract a human-readable message from a caught panic payload (the `Err` of
/// [`std::panic::catch_unwind`]). Panics usually carry a `&str` or `String`; a
/// non-string payload degrades to a placeholder rather than being lost.
fn panic_payload_str(payload: &(dyn std::any::Any + Send)) -> String {
    if let Some(s) = payload.downcast_ref::<&str>() {
        (*s).to_string()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else {
        "<non-string panic payload>".to_string()
    }
}

/// Entry point: build the daemon and run the event loop until `Shutdown`.
///
/// Called via [`run_supervised`], which owns `control_rx` and re-invokes this on a
/// panic; hence the `&mut` borrow of the receiver (it must survive a respawn) and
/// the by-value shared handles (cloned fresh per attempt by the supervisor). A
/// clean return (`Control::Shutdown` or a closed `control_rx`) means normal
/// shutdown; a panic unwinds out to the supervisor.
///
/// Receives external `config:changed` signals from the file-watch actor via a
/// dedicated [`tokio::sync::Notify`] (not the global broadcast bus) so that
/// the input runtime does not hold a permanent broadcast receiver — which
/// would prevent `receiver_count()==0` fast-paths like `notify_held_buttons()`.
/// Both paths (IPC `set-config` → `Control::ConfigChanged` and file-watch →
/// `config_changed` notification) converge on the same `apply_config_changed`
/// helper, so cache state is always consistent (#163).
///
/// Compositor focus changes arrive over `active_window_rx` (a coalescing
/// [`tokio::sync::watch`]) rather than the control channel — focus is state, so
/// latest-wins is correct and a busy loop can never drop or back up a change.
pub async fn run(
    control_rx: &mut mpsc::Receiver<Control>,
    events: broadcast::Sender<Event>,
    config_changed: std::sync::Arc<tokio::sync::Notify>,
    metrics: std::sync::Arc<crate::metrics::Metrics>,
    mut active_window_rx: watch::Receiver<String>,
) {
    let (internal_tx, mut internal_rx) = mpsc::channel::<Internal>(256);

    let bindings = config::load_bindings(&config::settings_path());
    let mut button_map = HashMap::new();
    for b in &bindings {
        button_map.insert(b.button, b.key);
    }
    // Parse per-player/per-game binding overrides (#104) and rumbleEnabled from a
    // single settings.json read, offloaded to the blocking pool so the input
    // runtime's startup isn't stalled on disk I/O (consistent with
    // apply_config_changed's later reads).
    let startup_doc = read_settings_doc().await;
    let per_player_bindings = config::parse_per_player_bindings(&startup_doc);
    let per_game_bindings = config::parse_per_game_bindings(&startup_doc);
    let rumble_enabled = config::rumble_enabled_from(&startup_doc);

    let (kb, mouse) = match build_uinput(&button_map) {
        Ok(v) => v,
        Err(e) => {
            error!("uinput init failed (need /dev/uinput access): {e}");
            return;
        }
    };

    // Device-ownership registry (replaces the old `is_synthetic` name match).
    // Only per-player virtual pads need registering — they advertise BTN_SOUTH
    // and copy the physical pad's input_id, so discovery would otherwise grab
    // them. The virtual keyboard/mouse don't advertise BTN_SOUTH, so discovery's
    // has_btn_south filter already excludes them; no need to track them here.
    let reg = VirtualRegistry::new();

    let db = device::load_db();
    if db.is_empty() {
        warn!("controller DB is empty; non-pinned discovery will reject all pads — set GAME_SHELL_GAMECONTROLLERDB or pin GAMEPAD_VENDOR/GAMEPAD_PRODUCT");
    } else {
        info!("controller DB loaded: {} known models", db.len());
    }

    let mut sh = Shared {
        events,
        internal_tx,
        kb,
        mouse,
        db,
        reg,
        bindings,
        button_map,
        per_player_bindings,
        per_game_bindings,
        active_game: None,
        input_mode: InputMode::Controller,
        gen_seq: 0,
        pending_capture: None,
        capture_timeout_task: None,
        capture_gen: 0,
        home_hold_active: false,
        presenter: Presenter::Shell,
        overlay_focus: false,
        session_active: true,
        // Per-app contracts + the Meta/combo timing knobs are read once at
        // startup (like every other config.toml section); `global()` is populated
        // in main before this thread spawns, and re-read cleanly on a supervisor
        // respawn.
        contracts: crate::daemon_config::global().input_contracts(),
        meta_hold: Duration::from_millis(crate::daemon_config::global().input.meta_hold_ms),
        combo_guard: Duration::from_millis(crate::daemon_config::global().input.combo_guard_ms),
        handoff_pinned: false,
        pending_focus_class: None,
        focus_settle_gen: 0,
        focus_settle_task: None,
        // rumble_enabled is derived from the single offloaded startup settings
        // read (M3), superseding origin/main's inline config::rumble_enabled call.
        rumble_enabled,
        metrics,
    };
    let mut fleet = Fleet::new();

    // Guards the `active_window_rx.changed()` arm: once every watch sender is
    // dropped (shutdown) `changed()` resolves `Err` immediately on every poll, so
    // we disable the arm to avoid a busy-spin. In normal operation `Shutdown`
    // arrives over `control_rx` first and breaks the loop.
    let mut focus_watch_live = true;

    loop {
        tokio::select! {
            // Control commands from the IPC server.
            ctrl = control_rx.recv() => {
                match ctrl {
                    Some(c) => if !handle_control(&mut sh, &mut fleet, c).await { break },
                    None => break,
                }
            }
            // Timer/reader callbacks.
            Some(internal) = internal_rx.recv() => {
                handle_internal(&mut sh, &mut fleet, internal);
            }
            // Live gamepad events from any pad in the fleet.
            Some((fd, res)) = next_fleet_event(&mut fleet), if !fleet.pads.is_empty() => {
                match res {
                    Ok(ev) => {
                        // Observability: count every raw evdev event we read from
                        // the fleet (the hot path), whether or not it is acted on.
                        sh.metrics.inc_input_events();
                        // While our session is backgrounded the pad is ungrabbed
                        // and the foreground compositor also receives this event;
                        // read-and-drop so nothing is injected via uinput.
                        if sh.session_active {
                            if let Some(pad) = fleet.pads.get_mut(&fd) {
                                pad.handle_event(&mut sh, ev);
                            }
                        }
                    }
                    Err(_) => on_pad_leave(&mut sh, &mut fleet, fd),
                }
                // Clear the fleet-level Home-hold latch once no pad holds Home,
                // re-arming the 0->1 edge for the next coordinated hold.
                if !fleet.any_holds_home() {
                    sh.home_hold_active = false;
                }
            }
            // Reconnect / hot-join poll: discover any new pad not already owned,
            // and refresh each pad's battery (#100) — a change emits
            // `pad:battery:*`. A no-op for wired pads.
            _ = tokio::time::sleep(Duration::from_secs(2)) => {
                try_join(&mut sh, &mut fleet);
                // Offload the per-pad sysfs battery scan to the blocking pool so the
                // input loop stays responsive while walking /sys/class/power_supply.
                let names: Vec<(RawFd, String)> = fleet
                    .pads
                    .iter()
                    .map(|(fd, pad)| (*fd, pad.name.clone()))
                    .collect();
                if !names.is_empty() {
                    let results = tokio::task::spawn_blocking(move || {
                        names
                            .into_iter()
                            .map(|(fd, name)| (fd, read_pad_battery(&name)))
                            .collect::<Vec<_>>()
                    })
                    .await
                    .unwrap_or_default();
                    for (fd, state) in results {
                        if let Some(pad) = fleet.pads.get_mut(&fd) {
                            pad.apply_battery(&mut sh, state);
                        }
                    }
                }
            }
            // External config:changed signal (#163): the file-watch actor
            // detected an external edit to settings.json and notified via the
            // dedicated Notify channel. Refresh in-memory caches exactly as
            // the IPC set-config path does via Control::ConfigChanged, so
            // rumbleEnabled and per-player/per-game bindings stay current
            // without a daemon restart. Using a Notify instead of subscribing
            // to the global broadcast bus avoids a permanent receiver that
            // would defeat receiver_count()==0 fast-paths.
            _ = config_changed.notified() => {
                tracing::debug!("input: config_changed notified, refreshing caches");
                apply_config_changed(&mut sh).await;
            }
            // Coalesced compositor focus (latest-wins) from the Hyprland actor.
            // `borrow_and_update` marks the value seen so `changed()` re-fires only
            // on a genuinely newer focus. Instead of applying immediately, arm the
            // settle debounce (schedule_focus_change) so a burst of launch/close
            // focus flaps collapses to one net transition and never thrashes the
            // virtual pad (which Steam re-enumerates on each create/destroy). The
            // startup "" never fires (watch signals only post-construction values),
            // and even a spurious "" just settles to a no-op. See #221/#294.
            res = active_window_rx.changed(), if focus_watch_live => {
                if res.is_ok() {
                    let class = active_window_rx.borrow_and_update().to_string();
                    schedule_focus_change(&mut sh, &class);
                } else {
                    // All watch senders dropped (shutdown in progress); stop
                    // polling this arm so it can't busy-spin.
                    focus_watch_live = false;
                }
            }
        }
    }

    // On shutdown release each pad's kernel grab (kept in both presenters during
    // normal operation), reset stick state, and abort tasks. Releasing the grab
    // lets the next daemon/compositor own the pad cleanly.
    let fds: Vec<RawFd> = fleet.pads.keys().copied().collect();
    for fd in fds {
        if let Some(mut pad) = fleet.pads.remove(&fd) {
            pad.ungrab(&mut sh); // also resets stick state
            pad.abort_all_tasks();
        }
    }
    info!("Shutting down");
}

/// Await the next event from any pad's stream, tagged with its fd. Pends forever
/// when the fleet is empty (the select arm is guarded on `!pads.is_empty()`).
async fn next_fleet_event(fleet: &mut Fleet) -> Option<(RawFd, std::io::Result<InputEvent>)> {
    use futures::stream::{FuturesUnordered, StreamExt};
    if fleet.pads.is_empty() {
        return std::future::pending().await;
    }
    // Race every pad's next_event; the first to resolve wins this tick. Each
    // future borrows one pad mutably and yields its fd alongside the result.
    let mut futs = FuturesUnordered::new();
    for (&fd, pad) in fleet.pads.iter_mut() {
        futs.push(async move { (fd, pad.event_stream.next_event().await) });
    }
    futs.next().await
}

fn build_uinput(button_map: &HashMap<u16, u16>) -> std::io::Result<(VirtualDevice, VirtualDevice)> {
    // Keyboard: all mapped keys (deduped) + the arrows, modifiers, and Q used
    // for d-pad/left-stick and the Moonlight force-quit chord. Enter/Esc are
    // fixed members too (not just transitively via `button_map`) so the
    // `key select`/`key back` IPC always has an advertised keycode to emit,
    // independent of how `select`/`back` are bound — a uinput device silently
    // drops events for keycodes it never declared.
    let mut mapped: Vec<u16> = button_map.values().copied().collect();
    mapped.sort_unstable();
    mapped.dedup();
    let extra = [
        cfg::KEY_UP,
        cfg::KEY_DOWN,
        cfg::KEY_LEFT,
        cfg::KEY_RIGHT,
        cfg::KEY_ENTER,
        cfg::KEY_ESC,
        cfg::KEY_LEFTCTRL,
        cfg::KEY_LEFTALT,
        cfg::KEY_LEFTSHIFT,
        cfg::KEY_Q,
    ];
    let keys: AttributeSet<KeyCode> = mapped
        .iter()
        .chain(extra.iter())
        .map(|&k| KeyCode::new(k))
        .collect();
    let kb = VirtualDevice::builder()?
        .name("game-shell-virtual-kb")
        .with_keys(&keys)?
        .build()?;
    info!("uinput keyboard device created");

    let mkeys: AttributeSet<KeyCode> = [cfg::BTN_LEFT, cfg::BTN_RIGHT, cfg::BTN_MIDDLE]
        .into_iter()
        .map(KeyCode::new)
        .collect();
    // RelativeAxisCode is a tuple struct `RelativeAxisCode(pub u16)` (no `new`).
    let axes: AttributeSet<RelativeAxisCode> =
        [cfg::REL_X, cfg::REL_Y, cfg::REL_WHEEL, cfg::REL_HWHEEL]
            .into_iter()
            .map(RelativeAxisCode)
            .collect();
    let mouse = VirtualDevice::builder()?
        .name("game-shell-virtual-mouse")
        .with_keys(&mkeys)?
        .with_relative_axes(&axes)?
        .build()?;
    info!("uinput mouse device created");

    Ok((kb, mouse))
}

// --- control handling ----------------------------------------------------

/// Read and parse `settings.json` off the input runtime's event loop.
///
/// The actual disk read + JSON parse runs on Tokio's blocking pool via
/// `spawn_blocking`, so the single input task stays responsive to gamepad
/// events and timers while waiting. A missing/garbage file degrades to an empty
/// object (matching the prior inline `unwrap_or` behavior). If the blocking task
/// somehow fails to join (runtime shutting down), it also degrades to empty.
async fn read_settings_doc() -> serde_json::Value {
    tokio::task::spawn_blocking(|| {
        std::fs::read_to_string(config::settings_path())
            .ok()
            .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
            .unwrap_or_else(|| serde_json::Value::Object(serde_json::Map::new()))
    })
    .await
    .unwrap_or_else(|_| serde_json::Value::Object(serde_json::Map::new()))
}

/// Apply an already-read settings document to the input runtime's in-memory
/// caches (#163, #108). Pure (no I/O) so the disk read can be offloaded by the
/// caller; both refresh paths converge here so they apply identical logic.
fn apply_config_doc(sh: &mut Shared, doc: &serde_json::Value) {
    sh.rumble_enabled = config::rumble_enabled_from(doc);
    sh.per_player_bindings = config::parse_per_player_bindings(doc);
    sh.per_game_bindings = config::parse_per_game_bindings(doc);
}

/// Refresh the input runtime's caches from disk, reading `settings.json` off the
/// event loop. Called by both the `Control::ConfigChanged` arm (IPC `set-config`
/// path) and the `config_changed.notified()` select arm (file-watch path).
async fn apply_config_changed(sh: &mut Shared) {
    let doc = read_settings_doc().await;
    apply_config_doc(sh, &doc);
}

/// Returns false to stop the loop (shutdown).
async fn handle_control(sh: &mut Shared, fleet: &mut Fleet, ctrl: Control) -> bool {
    match ctrl {
        Control::Grab(r) => {
            grab_all(sh, fleet);
            let _ = r.send(resp_ok());
        }
        Control::Release(r) => {
            release_all(sh, fleet);
            let _ = r.send(resp_ok());
        }
        Control::Handoff(r) => {
            // Replies `ok` unconditionally, mirroring Grab/Release above:
            // ungrab()/enter_shell() are best-effort and log internally on the
            // rare failure (the ungrab ioctl can only fail on an already-dead
            // fd, which on_pad_leave reaps). A stuck grab is self-correcting —
            // the shell re-emits `grab` on every stream-exit path (see the
            // handoff section in docs/IPC_PROTOCOL.md).
            handoff_all(sh, fleet);
            let _ = r.send(resp_ok());
        }
        Control::Status(r) => {
            let _ = r.send(fleet.status_string(sh.presenter));
        }
        Control::GetPads(r) => {
            let _ = r.send(fleet.pads_json());
        }
        Control::ListInputDevices(r) => {
            // #97 diagnostics enumerator. evdev::enumerate() is synchronous and can be
            // slow on a host with many input devices, so run it off the input runtime
            // (#108) — the fleet's grabbed paths are the only runtime state it needs.
            let grabbed_paths: HashSet<PathBuf> = fleet
                .pads
                .values()
                .filter(|p| p.grabbed)
                .map(|p| p.path.clone())
                .collect();
            tokio::task::spawn_blocking(move || {
                let _ = r.send(list_input_devices_with(grabbed_paths));
            });
        }
        Control::GetBindings(r) => {
            let ordered: Vec<(String, String)> = sh
                .bindings
                .iter()
                .map(|b| (b.action.to_string(), config::button_code_to_name(b.button)))
                .collect();
            let _ = r.send(crate::protocol::resp_bindings(&ordered));
        }
        Control::SetBinding {
            action,
            button,
            reply,
        } => {
            let resp = do_set_binding(sh, &action, &button);
            let _ = reply.send(resp);
        }
        Control::CaptureNext(r) => arm_capture(sh, r),
        Control::CaptureCancel(r) => {
            cancel_capture(sh);
            let _ = r.send(resp_ok());
        }
        Control::Intent { name, reply } => {
            // Pure broadcast: validate against the closed vocabulary and, if
            // valid, re-emit `intent:<name>` to all subscribers. Touches no
            // device — the control surface for keyboard global-escape and
            // automation.
            if is_known_intent(&name) {
                sh.publish(Event::Intent(name));
                let _ = reply.send(resp_ok());
            } else {
                let _ = reply.send(resp_unknown_intent(&name));
            }
        }
        Control::Rumble { id, ms, reply } => {
            // Fire a rumble on the pad with this wire id (#99). Gated by the
            // persisted `rumbleEnabled` setting and the pad's FF capability;
            // a missing pad / disabled setting / no-FF pad is a clean no-op that
            // still replies `ok` (the shell shouldn't treat "no rumble hardware"
            // as an error). `ms` is clamped to u16 (kernel FF replay length).
            if sh.rumble_enabled {
                if let Some(pad) = fleet.find_by_wire_id_mut(&id) {
                    pad.rumble(ms.min(u16::MAX as u32) as u16);
                }
            }
            let _ = reply.send(resp_ok());
        }
        Control::Key { name, reply } => {
            // Synthesize a keystroke (press+release) on the shared virtual
            // keyboard — the headless counterpart to a gamepad d-pad/A/B tap or
            // a `wtype -k`, reaching whatever surface holds Wayland focus. Unlike
            // `intent`, this deliberately touches the device. Unknown names are
            // rejected without emitting.
            match config::key_for_action(&name) {
                Some(code) => {
                    sh.emit_key(code, 1);
                    sh.emit_key(code, 0);
                    let _ = reply.send(resp_ok());
                }
                None => {
                    let _ = reply.send(resp_unknown_key(&name));
                }
            }
        }
        Control::ConfigChanged => {
            // set-config or file-watch: refresh cached settings (#108, #104, #163).
            apply_config_changed(sh).await;
        }
        Control::SetActiveGame { id, reply } => {
            // Set or clear the active game for per-game binding lookup (#104).
            // In-memory only — not persisted to settings.json.
            sh.active_game = id;
            let _ = reply.send(resp_ok());
        }
        Control::OverlayFocus { on, reply } => {
            // #262: a modal shell overlay opened/closed over a running app.
            // Flip routing to the shell key-map + force-grab (on) / restore the
            // base presenter's grab (off) without touching `sh.presenter`.
            set_overlay_focus(sh, fleet, on);
            let _ = reply.send(resp_ok());
        }
        Control::PadBatteryQuery { id, reply } => {
            // #160: reply with battery state for the pad identified by wire id.
            // A wired pad (no battery sysfs entry) replies `present:false`;
            // an unknown id replies `error:pad not found '<id>'`.
            match fleet.find_by_wire_id_mut(&id) {
                None => {
                    let _ = reply.send(resp_pad_not_found(&id));
                }
                Some(pad) => {
                    let resp = match pad.battery {
                        None => resp_pad_battery_not_present(&id),
                        Some(bs) => resp_pad_battery_present(&id, bs.level, bs.charging),
                    };
                    let _ = reply.send(resp);
                }
            }
        }
        Control::PadRumbleStatus { id, reply } => {
            // #160: reply with rumble capability and whether it is currently
            // enabled (the `rumbleEnabled` setting). `supported=true` iff the
            // pad has an uploaded `ff_effect`.
            match fleet.find_by_wire_id_mut(&id) {
                None => {
                    let _ = reply.send(resp_pad_not_found(&id));
                }
                Some(pad) => {
                    let supported = pad.ff_effect.is_some();
                    let resp = resp_pad_rumble_status(&id, supported, sh.rumble_enabled);
                    let _ = reply.send(resp);
                }
            }
        }
        Control::ControllerDbRefreshed { reply } => {
            // #159: the IPC layer fetched a fresh upstream DB and updated the
            // on-disk cache; re-read the merged DB (bundled baseline + cache +
            // env override) and merge it into the live runtime so new
            // controllers are identified without a daemon restart. The runtime's
            // initial DB comes from `device::load_db()` (baseline + env); the
            // cached upstream entries are folded in here, on refresh.
            let (fresh, _source) = crate::controllerdb::load_merged_db();
            sh.db.merge(&fresh);
            let _ = reply.send(resp_ok());
        }
        Control::SetSessionActive(active) => {
            if active != sh.session_active {
                sh.session_active = active;
                if active {
                    info!(
                        pads = fleet.pads.len(),
                        "session active -> re-grabbing pads"
                    );
                    for pad in fleet.pads.values_mut() {
                        pad.grab(sh);
                    }
                } else {
                    info!(
                        pads = fleet.pads.len(),
                        "session inactive -> releasing pads"
                    );
                    for pad in fleet.pads.values_mut() {
                        pad.ungrab(sh);
                        // Drop any in-flight combo buffer: a backgrounded session's
                        // events are ignored, so a partial sequence must not linger
                        // and replay when the session reactivates.
                        pad.reset_combo_buffer(sh);
                    }
                }
            }
        }

        Control::Shutdown => return false,
    }
    true
}

/// Switch the fleet to the **shell presenter** (the `grab` IPC, and — since
/// follow-focus — a compositor focus change back to the shell home; see
/// [`focus_presenter_target`]). Per-fleet mode toggle (Phase 5): set the mode,
/// ensure every pad is physically grabbed, and tear down any per-player
/// virtual gamepads. The physical grab is *kept* — the shell presenter routes
/// pad input to nav keys + `intent:*` on the shared virtual keyboard/mouse.
fn grab_all(sh: &mut Shared, fleet: &mut Fleet) {
    info!(pads = fleet.pads.len(), "presenter -> Shell (grab)");
    sh.metrics.inc_transitions();
    sh.presenter = Presenter::Shell;
    sh.handoff_pinned = false; // leaving any Handoff clears the Moonlight pin
    for pad in fleet.pads.values_mut() {
        pad.grab(sh); // no-op if already grabbed; re-grabs if somehow released
        pad.enter_shell(sh);
    }
    check_grab_invariant(sh, fleet);
}

/// Switch the fleet to the **keyboard presenter** (a follow-focus transition to a
/// window with a `keyboard` input contract, e.g. `tv.plex.Plex`). Mechanically
/// this is [`grab_all`] with a different presenter label: keep every pad
/// physically grabbed (nothing leaks to the compositor) and tear down any
/// per-player virtual pad — the focused app is driven by the shell key-map on the
/// shared virtual keyboard/mouse (`handle_shell`), NOT a virtual gamepad. Dropping
/// the virtual pad here is the fix's core: with no virtual pad alive, Steam has
/// nothing to exclusive-grab, so a focused Plex actually receives the d-pad.
fn keyboard_all(sh: &mut Shared, fleet: &mut Fleet) {
    info!(pads = fleet.pads.len(), "presenter -> Keyboard (contract)");
    sh.metrics.inc_transitions();
    sh.presenter = Presenter::Keyboard;
    sh.handoff_pinned = false;
    for pad in fleet.pads.values_mut() {
        pad.grab(sh); // keep the physical grab (shell-style key emulation)
        pad.enter_shell(sh); // drop any virtual pad — no gamepad in this context
    }
    check_grab_invariant(sh, fleet);
}

/// Switch the fleet to the **game presenter** (the `release` IPC, and — since
/// follow-focus — a compositor focus change to a real app toplevel; see
/// [`focus_presenter_target`]). Per-fleet mode toggle (Phase 5): set the mode,
/// **keep** the physical grab (so nothing leaks to the compositor), and create
/// one clean virtual gamepad per pad. The game reads the virtual pads; Home is
/// intercepted into `intent:home-*`.
fn release_all(sh: &mut Shared, fleet: &mut Fleet) {
    info!(pads = fleet.pads.len(), "presenter -> Game (release)");
    sh.metrics.inc_transitions();
    sh.presenter = Presenter::Game;
    sh.handoff_pinned = false; // leaving any Handoff clears the Moonlight pin
    for pad in fleet.pads.values_mut() {
        // Keep the physical grab; only ensure it's grabbed (it is, post-join).
        pad.grab(sh);
        pad.enter_game(sh);
    }
    check_grab_invariant(sh, fleet);
}

/// The **`handoff` IPC** entry point (#221): hand the physical pads directly to a
/// Moonlight stream. Delegates to [`enter_handoff`] with `pinned = true` so
/// follow-focus never overrides it while the stream window holds focus — only the
/// shell's explicit `grab` ends it. (A `handoff` *contract* reaches the same
/// presenter via [`enter_handoff`] with `pinned = false`, from `apply_focus_change`.)
fn handoff_all(sh: &mut Shared, fleet: &mut Fleet) {
    // The explicit `handoff` IPC is the Moonlight-stream path: PIN it so
    // follow-focus never overrides it while the stream window holds focus.
    enter_handoff(sh, fleet, true);
}

/// Enter the **handoff presenter**: drop any virtual twin and, unless an overlay
/// forces the grab, release the physical `EVIOCGRAB` so SDL/Moonlight reads the
/// real evdev node. `pinned` records *why* Handoff was entered:
///
/// * `true` — the explicit `handoff` IPC (Moonlight stream, #221). Follow-focus
///   must not override it (the streamed window holds focus for the whole stream);
///   only the shell's explicit `grab` ends it.
/// * `false` — a `handoff` **input contract** matched by follow-focus. This one
///   *does* follow focus: moving to another app (or back to the shell) arbitrates
///   normally, so a contract-driven handoff can never strand the pads ungrabbed.
fn enter_handoff(sh: &mut Shared, fleet: &mut Fleet, pinned: bool) {
    info!(
        pads = fleet.pads.len(),
        pinned, "presenter -> Handoff (handoff)"
    );
    sh.metrics.inc_transitions();
    sh.presenter = Presenter::Handoff;
    sh.handoff_pinned = pinned;
    // Reconcile the grab against `should_grab` rather than unconditionally
    // ungrabbing: with an overlay focused, the invariant is that the physical
    // pad stays grabbed even over Handoff (#262), so the app can't read the raw
    // evdev node while a shell overlay is open. Mirrors `set_overlay_focus`.
    let grab = should_grab(sh.overlay_focus, sh.presenter);
    for pad in fleet.pads.values_mut() {
        pad.enter_shell(sh); // drop any virtual pad
        if grab {
            pad.grab(sh); // keep the physical grab (overlay focused over Handoff)
        } else {
            pad.ungrab(sh); // release the grab so SDL reads the real node
        }
    }
    check_grab_invariant(sh, fleet);
}

/// Which presenter's handler processes pad events, given the overlay-focus flag
/// and the base presenter (#262). Overlay-focus forces [`Presenter::Shell`] over
/// any base so the pad drives an open modal shell overlay via the shell key-map
/// rather than the app; otherwise the base presenter routes as usual. Pure, so
/// the routing decision is unit-tested without a controller.
fn route_presenter(overlay_focus: bool, presenter: Presenter) -> Presenter {
    if overlay_focus {
        Presenter::Shell
    } else {
        presenter
    }
}

/// Whether every pad should hold the physical `EVIOCGRAB`, given the
/// overlay-focus flag and the base presenter (#262). Grabbed in every state
/// except a [`Presenter::Handoff`] base with no overlay open — the one case
/// where SDL/Moonlight must read the raw evdev node directly. Overlay-focus
/// therefore forces the grab even over Handoff. Pure — the grab transitions in
/// [`set_overlay_focus`] are unit-tested without a controller.
fn should_grab(overlay_focus: bool, presenter: Presenter) -> bool {
    overlay_focus || presenter != Presenter::Handoff
}

/// Whether the current presenter means a focused *app* owns the screen (rather
/// than the shell home). True for [`Presenter::Keyboard`]/[`Presenter::Game`]/
/// [`Presenter::Handoff`], false for [`Presenter::Shell`]. Drives whether the
/// force-quit combo (Back+Home+LB+RB) also emits the app-quit keyboard chord — a
/// controller escape from an app that captured input, for a couch user with no
/// keyboard in reach. The Shell home has no app to quit. `Keyboard` is included
/// (a keyboard-contract app like Plex is receiving our emulated keys and owns the
/// screen); its exclusion was the on-device "locked inside Plex" regression. Pure,
/// so the escape policy is unit-tested without a controller.
fn presenter_owns_app(presenter: Presenter) -> bool {
    !matches!(presenter, Presenter::Shell)
}

/// Decide whether a KEY event should be forwarded to the Game presenter's
/// virtual pad, given the pad's `masked` set (buttons held at the shell→app
/// flip; see [`PadDevice::masked_keys`]) and the event's `code`/`value`.
///
/// A **masked** button must not reach the app until it has been released and
/// pressed again — the app never saw the corresponding down (the physical press
/// was consumed by the shell to launch the app), so a lone up (or the stale
/// held-down / autorepeat) would be a phantom activation (the Steam-BPM A-leak,
/// #295 follow-up). Values follow evdev KEY semantics: `1` press, `0` release,
/// `2` autorepeat.
///
/// * `code` not in `masked` → forward normally (`true`). A fresh press after the
///   user let go is unaffected.
/// * `code` in `masked`, `value == 0` (release) → remove it from `masked` and
///   **swallow** (`false`): the mask is now cleared, so the *next* press of this
///   code forwards, but this lone up itself is dropped.
/// * `code` in `masked`, `value == 1 | 2` (press/repeat) → **swallow**
///   (`false`): it was held across the flip.
///
/// Mutates `masked` (clears the code on its release). Pure otherwise, so the
/// swallow decision is unit-tested without a controller. ABS axes/sticks/d-pad
/// are never masked — only digital buttons leak this way — so this is called
/// only from the KEY arm of [`PadDevice::handle_game`].
fn mask_forward_decision(masked: &mut HashSet<u16>, code: u16, value: i32) -> bool {
    if !masked.contains(&code) {
        return true; // not masked -> forward as usual
    }
    if value == 0 {
        // Release of a masked button: the mask lifts here, but the lone up is
        // swallowed (the app never saw the down).
        masked.remove(&code);
    }
    // Press/repeat/release of a still-masked button: never forward.
    false
}

/// Toggle overlay-focus (#262): a modal shell overlay opened (`on`) or closed
/// (`off`) over a running app. Idempotent (a no-op when already in the requested
/// state). The base presenter in `sh.presenter` is deliberately left untouched —
/// it *remembers* the routing to restore — while the grab is reconciled to match
/// [`should_grab`] for the new (overlay, base) pair:
///
/// * ON → [`should_grab`] is always `true`, so every pad is grabbed. For a
///   `Handoff` base this re-takes the `EVIOCGRAB` the app was reading raw; for
///   `Shell`/`Game` it is an idempotent no-op. `Game`'s virtual pads are left in
///   place (routing goes to the shell key-map, so they simply receive nothing
///   until overlay-focus off, then resume forwarding).
/// * OFF → [`should_grab`] is `false` only for a `Handoff` base, which re-ungrabs
///   so SDL/Moonlight reads the raw node again; every other base keeps the grab.
fn set_overlay_focus(sh: &mut Shared, fleet: &mut Fleet, on: bool) {
    if sh.overlay_focus == on {
        return;
    }
    sh.overlay_focus = on;
    let grab = should_grab(sh.overlay_focus, sh.presenter);
    info!(
        overlay_focus = on,
        base = ?sh.presenter,
        grab,
        pads = fleet.pads.len(),
        "overlay-focus toggled"
    );
    for pad in fleet.pads.values_mut() {
        // Overlay open/close flips the routed presenter (an app presenter ⇄ Shell)
        // without an enter_shell/enter_game, so drop any partial combo buffer here
        // too — otherwise a sequence buffered under the app presenter could strand
        // or replay to the wrong surface.
        pad.reset_combo_buffer(sh);
        if grab {
            pad.grab(sh); // idempotent + session-aware
        } else {
            pad.ungrab(sh); // idempotent
        }
    }
    check_grab_invariant(sh, fleet);
}

/// Apply a settled compositor focus change to the presenter (follow-focus).
///
/// Runs from the [`Internal::FocusSettle`] handler once the focus has held for
/// [`FOCUS_SETTLE_MS`] (armed by [`schedule_focus_change`] off the `active_window`
/// watch arm) — the debounce collapses launch/close focus flaps so this applies
/// at most one net transition per settle. `class` is empty when only the shell's
/// own layer-shell surface remains (no toplevel focused). Delegates the decision
/// to [`focus_presenter_target`] (which consults the per-app [`InputContracts`])
/// and routes through the same presenter transitions as an explicit `grab`/`release`
/// (each of which asserts [`check_grab_invariant`] on its way out), so the
/// invariant is checked after any transition this triggers.
fn apply_focus_change(sh: &mut Shared, fleet: &mut Fleet, class: &str) {
    if let Some(target) =
        focus_presenter_target(sh.presenter, sh.handoff_pinned, &sh.contracts, class)
    {
        info!(
            class = %class,
            from = ?sh.presenter,
            to = ?target,
            "presenter follow-focus"
        );
        match target {
            Presenter::Shell => grab_all(sh, fleet),
            Presenter::Keyboard => keyboard_all(sh, fleet),
            Presenter::Game => release_all(sh, fleet),
            // A `handoff` *contract* matched by follow-focus. Enter Handoff
            // UNpinned so it still follows focus away again — unlike the Moonlight
            // `handoff` IPC, which pins (handoff_all).
            Presenter::Handoff => enter_handoff(sh, fleet, false),
        }
    }
}

/// Arm (or re-arm) the follow-focus settle debounce for the newest focused-window
/// `class` ([`FOCUS_SETTLE_MS`]). Stores the pending class, bumps the settle
/// generation, and spawns a one-shot timer that posts [`Internal::FocusSettle`];
/// the handler applies the transition only if its generation is still live, so a
/// burst of focus flaps during an app launch/close collapses to the single last
/// change. Every focus event re-arms (superseding any prior timer), so a rapid
/// flap can never tear down + rebuild the virtual pad mid-transition. Explicit
/// IPC (`grab`/`release`/`handoff`/overlay-focus) bypasses this and applies
/// instantly — only the noisy compositor focus signal is debounced.
fn schedule_focus_change(sh: &mut Shared, class: &str) {
    // Abort any prior in-flight settle timer so at most one is ever alive: a
    // focus-churn burst re-arms rather than piling up sleeping tasks. Correctness
    // still rests on the `focus_settle_gen` check when the timer fires — this only
    // frees the superseded tasks eagerly.
    if let Some(t) = sh.focus_settle_task.take() {
        t.abort();
    }
    sh.pending_focus_class = Some(class.to_string());
    let generation = sh.next_generation();
    sh.focus_settle_gen = generation;
    let tx = sh.internal_tx.clone();
    sh.focus_settle_task = Some(tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(FOCUS_SETTLE_MS)).await;
        let _ = tx.send(Internal::FocusSettle { generation }).await;
    }));
}

/// Pure per-pad grab invariant predicate: while the session is active a pad's
/// physical grab must match the presenter policy `expected`
/// (`should_grab(overlay_focus, presenter)`); while inactive any grab state is
/// acceptable (pads are intentionally all ungrabbed — see [`PadDevice::grab`]'s
/// session early-return and the `SetSessionActive` handling). Factored out so the
/// invariant logic is unit-tested without a `Fleet`/`Shared` (which own uinput
/// devices).
fn grab_ok(session_active: bool, pad_grabbed: bool, expected: bool) -> bool {
    !session_active || pad_grabbed == expected
}

/// Assert the fleet's physical grab state matches the intended presenter policy
/// after a transition, catching silent grab-state drift (a stuck or leaked
/// controller grab).
///
/// While `session_active`, every pad's `grabbed` flag must equal
/// `should_grab(overlay_focus, presenter)`. `grab_all`/`keyboard_all`/`release_all`
/// call `pad.grab()` unconditionally, which is consistent with `should_grab`
/// because `should_grab(_, Shell)`, `should_grab(_, Keyboard)`, and
/// `should_grab(_, Game)` are all always `true` (only a `Handoff` base with no
/// overlay ungrabs, and `enter_handoff` already routes each pad through
/// `should_grab`) — so the unconditional grab is correct by the truth table, and
/// this asserts exactly that. On a violation it `error!`s (pad id,
/// expected vs actual, presenter, overlay-focus), bumps a metrics counter, and
/// `debug_assert!`s so it panics in dev/test but never in release.
fn check_grab_invariant(sh: &Shared, fleet: &Fleet) {
    let expected = should_grab(sh.overlay_focus, sh.presenter);
    for pad in fleet.pads.values() {
        if !grab_ok(sh.session_active, pad.grabbed, expected) {
            error!(
                pad = %pad.wire_id,
                expected,
                actual = pad.grabbed,
                presenter = ?sh.presenter,
                overlay_focus = sh.overlay_focus,
                "grab invariant violated"
            );
            sh.metrics.inc_grab_invariant_violations();
            debug_assert!(
                grab_ok(sh.session_active, pad.grabbed, expected),
                "grab invariant violated for pad {}: expected grabbed={}, actual grabbed={} (presenter={:?}, overlay_focus={})",
                pad.wire_id,
                expected,
                pad.grabbed,
                sh.presenter,
                sh.overlay_focus,
            );
        }
    }
}

/// Decide whether a Hyprland focused-window class report should change the
/// fleet's presenter, following PR #294's "react continuously to whatever
/// Hyprland now considers active" pattern — applied to the input presenter
/// rather than kiosk fullscreen enforcement.
///
/// `focused_class` is empty when no toplevel is focused — i.e. only the
/// shell's own layer-shell surface remains, which never appears in
/// Hyprland's `activewindow` at all (see `hyprland.rs`'s `needs_fullscreen`
/// doc comment for the same fact used there). An empty class always maps to
/// [`Presenter::Shell`] (the shell owns input). A non-empty class routes through
/// its **input contract** ([`InputContracts::resolve`] → [`contract_presenter`]):
/// `gamepad`→Game (the default for unknown classes, so a class-agnostic app like
/// a Steam Remote Play `streaming_client` window still gets a real gamepad),
/// `keyboard`→Keyboard (e.g. Plex), `handoff`→Handoff.
///
/// Returns `None` when no change is warranted: the resolved target already
/// matches `current`, or `current` is a **pinned** [`Presenter::Handoff`].
/// Pinned Handoff (#221, the Moonlight stream via the explicit `handoff` IPC) is
/// a deliberate exception — follow-focus must never override it while the streamed
/// window holds compositor focus for the whole stream; the shell ends it via the
/// `grab` IPC. A Handoff reached instead via a `handoff` *contract* is NOT pinned
/// and arbitrates like any other presenter, so it can never strand the pads.
fn focus_presenter_target(
    current: Presenter,
    handoff_pinned: bool,
    contracts: &InputContracts,
    focused_class: &str,
) -> Option<Presenter> {
    if current == Presenter::Handoff && handoff_pinned {
        return None;
    }
    let target = if focused_class.is_empty() {
        Presenter::Shell
    } else {
        contract_presenter(contracts.resolve(focused_class))
    };
    if target == current {
        None
    } else {
        Some(target)
    }
}

/// Map a resolved [`InputContract`] to the presenter that honors it. The single
/// point of truth for the contract→presenter correspondence.
fn contract_presenter(contract: InputContract) -> Presenter {
    match contract {
        InputContract::Gamepad => Presenter::Game,
        InputContract::Keyboard => Presenter::Keyboard,
        InputContract::Handoff => Presenter::Handoff,
    }
}

/// What a Meta (BTN_MODE / Guide) TAP delivers, per (routed) presenter. Pure, so
/// the gesture map is unit-tested without a controller.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MetaTapAction {
    /// Publish `intent:home-tap` (open the shell drawer). Shell home.
    HomeTap,
    /// Replay a real Guide press+release onto the game's virtual pad. Game.
    ReplayToPad,
    /// Deliver nothing — a keyboard-contract app has no Guide concept. Keyboard.
    Swallow,
}

/// Resolve a Meta TAP's delivery from the effective (routed) presenter. Handoff
/// is handled inline in `handle_handoff` (never routed here); mapped to
/// `Swallow` for exhaustiveness.
fn meta_tap_action(routed: Presenter) -> MetaTapAction {
    match routed {
        Presenter::Shell => MetaTapAction::HomeTap,
        Presenter::Game => MetaTapAction::ReplayToPad,
        Presenter::Keyboard => MetaTapAction::Swallow,
        Presenter::Handoff => MetaTapAction::Swallow,
    }
}

/// The intent a Meta HOLD fires publishes, per (routed) presenter. Only the Shell
/// home's hold publishes `intent:home-hold` (the idle reset-to-clean-home); an
/// APP presenter's hold publishes `intent:home-tap` — which, while an app is
/// running, toggles the shell's *controllable overlay drawer* + overlay-focus (a
/// non-destructive everyday escape that works regardless of who holds compositor
/// toplevel focus), rather than the heavier `home-hold` full return-to-home.
/// Pure — unit-tested without a controller.
fn hold_fire_intent(routed: Presenter) -> &'static str {
    match routed {
        Presenter::Shell => "home-hold",
        Presenter::Keyboard | Presenter::Game | Presenter::Handoff => "home-tap",
    }
}

#[cfg(test)]
mod presenter_tests {
    use super::*;
    use std::collections::HashMap;

    /// Contracts with only the built-in defaults (no user overrides):
    /// `steam`→gamepad, `tv.plex.Plex`→keyboard, unknown→gamepad.
    fn default_contracts() -> InputContracts {
        InputContracts::default()
    }

    #[test]
    fn shell_to_game_when_gamepad_app_focused() {
        // An unknown class defaults to the gamepad contract -> Game presenter
        // (preserves the pre-contract "any app focused ⇒ virtual pad" behavior).
        assert_eq!(
            focus_presenter_target(
                Presenter::Shell,
                false,
                &default_contracts(),
                "steam_app_12345"
            ),
            Some(Presenter::Game)
        );
    }

    #[test]
    fn shell_to_keyboard_when_plex_focused() {
        // Plex carries the built-in keyboard contract -> Keyboard presenter (no
        // virtual pad, so Steam can't grab it and Plex gets the d-pad).
        assert_eq!(
            focus_presenter_target(
                Presenter::Shell,
                false,
                &default_contracts(),
                "tv.plex.Plex"
            ),
            Some(Presenter::Keyboard)
        );
    }

    #[test]
    fn game_to_keyboard_when_focus_moves_plex() {
        // Steam (Game) focused, user opens Plex: flip Game -> Keyboard, which
        // tears down the virtual pad (breaking Steam's exclusive grab).
        assert_eq!(
            focus_presenter_target(Presenter::Game, false, &default_contracts(), "tv.plex.Plex"),
            Some(Presenter::Keyboard)
        );
    }

    #[test]
    fn user_override_beats_builtin() {
        // A user can force Plex to a gamepad contract via config.
        let mut over = HashMap::new();
        over.insert("tv.plex.Plex".to_string(), InputContract::Gamepad);
        let contracts = InputContracts::new(over);
        assert_eq!(
            focus_presenter_target(Presenter::Shell, false, &contracts, "tv.plex.Plex"),
            Some(Presenter::Game)
        );
    }

    #[test]
    fn game_to_shell_when_focus_empty() {
        assert_eq!(
            focus_presenter_target(Presenter::Game, false, &default_contracts(), ""),
            Some(Presenter::Shell)
        );
    }

    #[test]
    fn no_change_when_target_matches_current() {
        // Already Shell, still no toplevel focused -> no-op (no thrash).
        assert_eq!(
            focus_presenter_target(Presenter::Shell, false, &default_contracts(), ""),
            None
        );
        // Already Game, focus moved to a DIFFERENT gamepad-contract app -> still
        // Game, no-op (switching between two app windows must not flap the
        // presenter, which would thrash the virtual pad).
        assert_eq!(
            focus_presenter_target(Presenter::Game, false, &default_contracts(), "another_app"),
            None
        );
    }

    #[test]
    fn pinned_handoff_is_never_touched_by_follow_focus() {
        // A Moonlight stream (Handoff via the explicit `handoff` IPC) is PINNED:
        // the stream window holding focus for the whole stream must not downgrade
        // it to Game (#221), nor does focus returning to the shell end it — only
        // an explicit `grab` IPC does.
        assert_eq!(
            focus_presenter_target(
                Presenter::Handoff,
                true,
                &default_contracts(),
                "steam_app_moonlight"
            ),
            None
        );
        assert_eq!(
            focus_presenter_target(Presenter::Handoff, true, &default_contracts(), ""),
            None
        );
    }

    #[test]
    fn contract_handoff_follows_focus() {
        // A `handoff` CONTRACT matched by follow-focus is NOT pinned, so it still
        // arbitrates: focus to a raw-node app -> Handoff; back to the shell ->
        // Shell (no stranding). First, a class mapped to handoff enters Handoff.
        let mut over = HashMap::new();
        over.insert("com.example.RawPad".to_string(), InputContract::Handoff);
        let contracts = InputContracts::new(over);
        assert_eq!(
            focus_presenter_target(Presenter::Shell, false, &contracts, "com.example.RawPad"),
            Some(Presenter::Handoff)
        );
        // Now in an UNpinned Handoff, focus returning to the shell moves out.
        assert_eq!(
            focus_presenter_target(Presenter::Handoff, false, &contracts, ""),
            Some(Presenter::Shell)
        );
        // ...and focus to a gamepad app moves to Game.
        assert_eq!(
            focus_presenter_target(Presenter::Handoff, false, &contracts, "steam"),
            Some(Presenter::Game)
        );
    }

    #[test]
    fn contract_presenter_mapping() {
        assert_eq!(contract_presenter(InputContract::Gamepad), Presenter::Game);
        assert_eq!(
            contract_presenter(InputContract::Keyboard),
            Presenter::Keyboard
        );
        assert_eq!(
            contract_presenter(InputContract::Handoff),
            Presenter::Handoff
        );
    }

    #[test]
    fn meta_tap_action_per_presenter() {
        // Shell home: a tap opens the drawer. Game: replay a real Guide to the pad.
        // Keyboard: nothing (a keyboard-contract app has no Guide concept).
        assert_eq!(meta_tap_action(Presenter::Shell), MetaTapAction::HomeTap);
        assert_eq!(meta_tap_action(Presenter::Game), MetaTapAction::ReplayToPad);
        assert_eq!(meta_tap_action(Presenter::Keyboard), MetaTapAction::Swallow);
    }

    #[test]
    fn hold_fire_intent_per_presenter() {
        // Only the Shell home's hold fires the heavy `home-hold` reset; every app
        // presenter's hold fires `home-tap` (the controllable overlay drawer).
        assert_eq!(hold_fire_intent(Presenter::Shell), "home-hold");
        assert_eq!(hold_fire_intent(Presenter::Keyboard), "home-tap");
        assert_eq!(hold_fire_intent(Presenter::Game), "home-tap");
        assert_eq!(hold_fire_intent(Presenter::Handoff), "home-tap");
    }

    #[test]
    fn overlay_focus_routes_to_shell_over_any_base() {
        // ON: the shell handler runs regardless of the base presenter, so the
        // pad drives the modal overlay, not the app (#262).
        assert_eq!(route_presenter(true, Presenter::Game), Presenter::Shell);
        assert_eq!(route_presenter(true, Presenter::Handoff), Presenter::Shell);
        assert_eq!(route_presenter(true, Presenter::Shell), Presenter::Shell);
        assert_eq!(route_presenter(true, Presenter::Keyboard), Presenter::Shell);
        // OFF: the base presenter routes as usual (no behavior change).
        assert_eq!(route_presenter(false, Presenter::Game), Presenter::Game);
        assert_eq!(
            route_presenter(false, Presenter::Handoff),
            Presenter::Handoff
        );
        assert_eq!(route_presenter(false, Presenter::Shell), Presenter::Shell);
        // Keyboard passes through (routed to the shell key-map by handle_event).
        assert_eq!(
            route_presenter(false, Presenter::Keyboard),
            Presenter::Keyboard
        );
    }

    #[test]
    fn overlay_focus_forces_grab_over_handoff() {
        // ON: grabbed regardless of base — critical for Handoff, normally
        // ungrabbed, so the app stops seeing raw events (#262).
        assert!(should_grab(true, Presenter::Handoff));
        assert!(should_grab(true, Presenter::Game));
        assert!(should_grab(true, Presenter::Shell));
        assert!(should_grab(true, Presenter::Keyboard));
        // OFF: the base grab state is restored — only a Handoff base is ungrabbed
        // (re-ungrab so SDL/Moonlight reads the raw node again).
        assert!(!should_grab(false, Presenter::Handoff));
        assert!(should_grab(false, Presenter::Game));
        assert!(should_grab(false, Presenter::Shell));
        // Keyboard keeps the grab (shell-style emulation; no raw-node handoff).
        assert!(should_grab(false, Presenter::Keyboard));
    }

    #[test]
    fn force_quit_chord_reaches_every_app_owning_presenter() {
        // The force-quit combo emits the app-quit keyboard chord whenever a
        // focused app owns the screen — Keyboard (e.g. Plex), Game, or Handoff —
        // so a couch user can always escape an app that captured input. The Shell
        // home has no app to quit. Keyboard's inclusion is the fix for the
        // on-device "locked inside Plex, no controller path back" regression.
        assert!(presenter_owns_app(Presenter::Keyboard));
        assert!(presenter_owns_app(Presenter::Game));
        assert!(presenter_owns_app(Presenter::Handoff));
        assert!(!presenter_owns_app(Presenter::Shell));
    }

    #[test]
    fn handoff_keeps_grab_when_overlay_focused() {
        // Regression (PR #296): switching to Handoff while an overlay is focused
        // must NOT drop the grab — the app would otherwise read the raw pad node
        // while a shell overlay is open (#262). `handoff_all` sets the presenter
        // to Handoff and leaves `overlay_focus` untouched, then reconciles each
        // pad against `should_grab(sh.overlay_focus, sh.presenter)`. With an
        // overlay focused, that pair must resolve to a grab.
        let overlay_focus = true;
        let presenter_after_handoff = Presenter::Handoff;
        assert!(should_grab(overlay_focus, presenter_after_handoff));

        // Without an overlay, Handoff correctly ungrabs (the raw-node case).
        assert!(!should_grab(false, presenter_after_handoff));
    }

    // --- flip-mask (#295 follow-up: swallow buttons held at shell→app flip) ---

    /// A button held at the flip (present in the mask) has its press/repeat
    /// swallowed, then its release clears the mask and is itself swallowed, and
    /// a subsequent fresh press forwards normally.
    #[test]
    fn masked_button_swallowed_until_released_then_fresh_press_forwards() {
        const BTN_A: u16 = cfg::BTN_SOUTH;
        // Simulate `enter_game` capturing BTN_A as held at the flip.
        let mut masked: HashSet<u16> = HashSet::new();
        masked.insert(BTN_A);

        // A stale autorepeat of the still-held A is swallowed (never forwarded).
        assert!(!mask_forward_decision(&mut masked, BTN_A, 2));
        assert!(masked.contains(&BTN_A), "repeat must not clear the mask");

        // The release (value 0) clears the mask AND is swallowed (the app never
        // saw the corresponding down, so a lone up would be a phantom event).
        assert!(!mask_forward_decision(&mut masked, BTN_A, 0));
        assert!(
            !masked.contains(&BTN_A),
            "release must clear the code from the mask"
        );

        // A fresh press after the user let go is no longer masked -> forwards.
        assert!(mask_forward_decision(&mut masked, BTN_A, 1));
        // ...as does its release.
        assert!(mask_forward_decision(&mut masked, BTN_A, 0));
    }

    /// A masked button's *press* (value 1) — the case where the physical button
    /// was released and re-pressed while the mask still stands (e.g. the user
    /// mashed it before letting go cleanly) — is also swallowed; only a value-0
    /// release lifts the mask.
    #[test]
    fn masked_button_press_is_swallowed() {
        const BTN_A: u16 = cfg::BTN_SOUTH;
        let mut masked: HashSet<u16> = HashSet::new();
        masked.insert(BTN_A);
        assert!(!mask_forward_decision(&mut masked, BTN_A, 1));
        assert!(masked.contains(&BTN_A), "press must not clear the mask");
    }

    /// A button that was NOT held at the flip (absent from the mask) forwards
    /// unconditionally — normal post-flip gameplay is unaffected, whatever the
    /// value.
    #[test]
    fn unmasked_button_always_forwards() {
        const BTN_B: u16 = cfg::BTN_EAST;
        let mut masked: HashSet<u16> = HashSet::new();
        // Mask holds a DIFFERENT code; B was not held at the flip.
        masked.insert(cfg::BTN_SOUTH);
        assert!(mask_forward_decision(&mut masked, BTN_B, 1));
        assert!(mask_forward_decision(&mut masked, BTN_B, 2));
        assert!(mask_forward_decision(&mut masked, BTN_B, 0));
        // The unrelated masked code is untouched by decisions about B.
        assert!(masked.contains(&cfg::BTN_SOUTH));
    }

    /// An empty mask (nothing held at the flip — the common launch-with-nothing
    /// -held case) forwards everything.
    #[test]
    fn empty_mask_forwards_everything() {
        let mut masked: HashSet<u16> = HashSet::new();
        assert!(mask_forward_decision(&mut masked, cfg::BTN_SOUTH, 1));
        assert!(mask_forward_decision(&mut masked, cfg::BTN_SOUTH, 0));
        assert!(masked.is_empty());
    }

    #[test]
    fn flip_mask_is_subset_of_held_keys() {
        // `enter_game` snapshots the flip-mask as `clear() + extend(held_keys)`,
        // so the mask is always a subset of the currently-held buttons (the 3a
        // invariant asserted by the debug_assert in `enter_game`). This mirrors
        // that operation without a `PadDevice` (which owns uinput devices).
        let mut held: HashSet<u16> = HashSet::new();
        held.insert(cfg::BTN_SOUTH);
        held.insert(cfg::BTN_EAST);

        let mut masked: HashSet<u16> = HashSet::new();
        masked.clear();
        masked.extend(held.iter().copied());
        assert!(
            masked.iter().all(|c| held.contains(c)),
            "flip-mask must be a subset of held_keys"
        );

        // An empty held set snapshots an empty mask — still a (trivial) subset.
        let empty: HashSet<u16> = HashSet::new();
        let mut masked2: HashSet<u16> = HashSet::new();
        masked2.extend(empty.iter().copied());
        assert!(masked2.iter().all(|c| empty.contains(c)));
        assert!(masked2.is_empty());
    }

    #[test]
    fn grab_invariant_predicate() {
        // Session active: the pad's grab must match the presenter policy.
        assert!(grab_ok(true, true, true)); // grabbed, expected grabbed -> ok
        assert!(grab_ok(true, false, false)); // ungrabbed, expected ungrabbed -> ok
        assert!(!grab_ok(true, false, true)); // ungrabbed but should be grabbed -> drift
        assert!(!grab_ok(true, true, false)); // grabbed but should be ungrabbed -> drift

        // Session inactive: pads are intentionally all ungrabbed, so ANY grab
        // state passes regardless of the presenter policy (the check is scoped to
        // session_active).
        assert!(grab_ok(false, false, true));
        assert!(grab_ok(false, true, false));
        assert!(grab_ok(false, false, false));
        assert!(grab_ok(false, true, true));
    }

    #[test]
    fn grab_invariant_matches_should_grab_after_transitions() {
        // The policy `check_grab_invariant` asserts: after grab_all/keyboard_all/
        // release_all the pads are grabbed (Shell/Keyboard/Game always grab), and
        // should_grab agrees.
        assert!(grab_ok(true, true, should_grab(false, Presenter::Shell)));
        assert!(grab_ok(true, true, should_grab(false, Presenter::Keyboard)));
        assert!(grab_ok(true, true, should_grab(false, Presenter::Game)));
        // Handoff with no overlay ungrabs, and an ungrabbed pad then satisfies it.
        assert!(grab_ok(true, false, should_grab(false, Presenter::Handoff)));
        // Handoff WITH an overlay keeps the grab (should_grab true).
        assert!(grab_ok(true, true, should_grab(true, Presenter::Handoff)));
    }

    #[test]
    fn panic_payload_extracts_str_and_string() {
        // &str payload (the common `panic!("msg")` case).
        let p: Box<dyn std::any::Any + Send> = Box::new("boom");
        assert_eq!(panic_payload_str(p.as_ref()), "boom");
        // String payload (e.g. `panic!("{}", e)`).
        let p: Box<dyn std::any::Any + Send> = Box::new(String::from("boom2"));
        assert_eq!(panic_payload_str(p.as_ref()), "boom2");
        // Non-string payload degrades to a placeholder rather than being lost.
        let p: Box<dyn std::any::Any + Send> = Box::new(42u32);
        assert_eq!(panic_payload_str(p.as_ref()), "<non-string panic payload>");
    }
}

fn do_set_binding(sh: &mut Shared, action: &str, button: &str) -> String {
    if !config::is_default_action(action) {
        return resp_unknown_action(action);
    }
    let Some(code) = config::button_name_to_code(button) else {
        return resp_invalid_button(button);
    };
    if !config::is_remappable(code) {
        return resp_invalid_button(button);
    }
    for b in sh.bindings.iter_mut() {
        if b.action == action {
            b.button = code;
        }
    }
    sh.rebuild_button_map();
    if let Err(e) = config::save_bindings(&config::settings_path(), &sh.bindings) {
        warn!("failed to save bindings: {e}");
    }
    resp_ok()
}

// --- capture (fleet-level) -----------------------------------------------

fn arm_capture(sh: &mut Shared, reply: Reply) {
    // Replacing a pending capture cancels the previous one.
    if let Some(old) = sh.pending_capture.take() {
        let _ = old.send(resp_cancelled());
    }
    if let Some(t) = sh.capture_timeout_task.take() {
        t.abort();
    }
    let generation = sh.next_generation();
    sh.capture_gen = generation;
    sh.pending_capture = Some(reply);
    let tx = sh.internal_tx.clone();
    sh.capture_timeout_task = Some(tokio::spawn(async move {
        tokio::time::sleep(Duration::from_secs(config::CAPTURE_TIMEOUT_SECS)).await;
        let _ = tx.send(Internal::CaptureTimeout(generation)).await;
    }));
}

fn resolve_capture(sh: &mut Shared, code: u16) {
    if let Some(r) = sh.pending_capture.take() {
        let _ = r.send(resp_captured(&config::button_code_to_name(code)));
    }
    if let Some(t) = sh.capture_timeout_task.take() {
        t.abort();
    }
    // Invalidate any already-queued timeout from the resolved capture.
    sh.capture_gen = sh.next_generation();
}

fn cancel_capture(sh: &mut Shared) {
    if let Some(r) = sh.pending_capture.take() {
        let _ = r.send(resp_cancelled());
    }
    if let Some(t) = sh.capture_timeout_task.take() {
        t.abort();
    }
    sh.capture_gen = sh.next_generation();
}

// --- hot-join / leave ----------------------------------------------------

/// Discover any newly-connected pads and add them to the fleet. Skips pads
/// already in the fleet (by **device path** — an already-grabbed pad
/// re-enumerates at the same path but a fresh fd) and our own virtual devices
/// (by fd, inside `find_gamepads`). Each joining pad is grabbed (shell
/// presenter), calibrated, assigned the lowest free slot, and announced via
/// `controller-wake` + `pad:connected:{id,index,name}`.
fn try_join(sh: &mut Shared, fleet: &mut Fleet) {
    // Paths already represented in the fleet, so a re-enumeration of a
    // connected pad doesn't open + grab it a second time.
    let known_paths: HashSet<PathBuf> = fleet.pads.values().map(|p| p.path.clone()).collect();

    for handle in device::find_gamepads(&sh.db, &sh.reg) {
        // Destructure before consuming `device` (into_event_stream moves it).
        let device::GamepadHandle {
            device,
            name,
            path,
            wire_id,
        } = handle;
        if known_paths.contains(&path) {
            continue; // already in the fleet
        }
        let stream = match device.into_event_stream() {
            Ok(s) => s,
            Err(e) => {
                error!("Failed to open event stream for {}: {e}", path.display());
                continue;
            }
        };
        let fd = stream.device().as_raw_fd();
        // A freshly-opened physical pad's fd can't already be in the fleet;
        // guard anyway so a duplicate enumeration never double-inserts.
        if fleet.pads.contains_key(&fd) {
            continue;
        }
        let slot = fleet.slots.alloc();
        info!(
            "Pad joined: {} at {} (id={}, slot={})",
            name,
            path.display(),
            wire_id,
            slot,
        );
        let mut pad = PadDevice::new(fd, stream, wire_id.clone(), name.clone(), path, slot);
        pad.calibrate();
        // Match the joining pad to the fleet's current presenter:
        //   * Shell / Keyboard — grab so its input drives the shell key-map (nav,
        //     or a keyboard-contract app like Plex). No virtual pad in either.
        //   * Game  — grab + clean virtual gamepad (a 2nd player joining a stream
        //     that runs through the virtual-pad path).
        //   * Handoff (#221) — leave it UNGRABBED so SDL/Moonlight reads the real
        //     evdev node directly, exactly like the pads already handed off.
        match sh.presenter {
            Presenter::Shell | Presenter::Keyboard => pad.grab(sh),
            Presenter::Game => {
                pad.grab(sh);
                pad.enter_game(sh);
            }
            Presenter::Handoff => { /* leave ungrabbed — SDL reads it directly */ }
        }
        // Overlay-focus layered on top of the base setup (#262): a modal shell
        // overlay is open over the app, so force the grab even for a Handoff
        // base — the joining pad drives the overlay via the shell key-map
        // (`handle_event` routes it there while `overlay_focus` is on). Idempotent
        // for the Shell/Game arms above; `Game` keeps its virtual pad so the
        // clean overlay-off restore forwards correctly.
        if sh.overlay_focus {
            pad.grab(sh);
        }
        // Fleet outputs (ride-along, Phase 5.5): light the player LED to match
        // the slot (#101 LED) and read the initial battery (#100). Both no-op on
        // pads lacking the capability. A short connect rumble (#99) gives haptic
        // feedback that the pad is live, gated by the `rumbleEnabled` setting and
        // the pad's FF support.
        pad.set_player_led(sh);
        pad.poll_battery(sh);
        if sh.rumble_enabled {
            pad.rumble(CONNECT_RUMBLE_MS);
        }
        fleet.pads.insert(fd, pad);

        // Legacy single-pad signal (any pad still drives `controller-wake` so the
        // existing QML wake path is unchanged), plus the fleet-aware
        // `pad:connected:{id,index,name}`.
        sh.publish(Event::ControllerWake);
        sh.publish(Event::PadConnected(pad_connected_json(
            &wire_id, slot, &name,
        )));
    }
}

/// A pad's stream errored (USB disconnect): drop it from the fleet, free its
/// slot for reuse, abort its tasks, and announce the leave.
fn on_pad_leave(sh: &mut Shared, fleet: &mut Fleet, fd: RawFd) {
    let Some(mut pad) = fleet.pads.remove(&fd) else {
        return;
    };
    warn!(
        "Pad left: slot {} ({}), freeing slot",
        pad.player_slot, pad.wire_id
    );
    pad.reset_stick_state(sh);
    pad.abort_all_tasks();
    // Drop any per-player virtual pad and forget its devnode (Phase 5 game presenter).
    if let Some(mut vpad) = pad.virtual_pad.take() {
        unregister_vpad_devnodes(&mut sh.reg, &mut vpad);
    }
    let slot = pad.player_slot;
    let wire_id = pad.wire_id.clone();
    fleet.slots.free(slot);
    drop(pad);

    // Legacy single-pad signal + fleet-aware `pad:disconnected:{id}`.
    sh.publish(Event::ControllerDisconnected);
    sh.publish(Event::PadDisconnected(wire_id));
}

// --- internal (timer) handling -------------------------------------------

fn handle_internal(sh: &mut Shared, fleet: &mut Fleet, internal: Internal) {
    match internal {
        Internal::StickRepeat {
            fd,
            axis,
            key,
            generation,
        } => {
            let Some(pad) = fleet.pads.get_mut(&fd) else {
                return; // pad left
            };
            // Ignore stale ticks: a tick whose generation no longer matches the
            // axis's live repeat (e.g. the stick re-deflected the same
            // direction) would otherwise repeat without the initial delay.
            let (live_gen, live_key) = match axis {
                Axis::X => (pad.stick_x_gen, pad.stick_x_key),
                Axis::Y => (pad.stick_y_gen, pad.stick_y_key),
            };
            if generation == live_gen && live_key == Some(key) {
                sh.emit_key(key, 0);
                sh.emit_key(key, 1);
            }
        }
        Internal::MouseTick { fd } => {
            if let Some(pad) = fleet.pads.get_mut(&fd) {
                pad.mouse_tick(sh);
            }
        }
        Internal::HomeHoldFired { fd, generation } => {
            let Some(pad) = fleet.pads.get_mut(&fd) else {
                return; // pad left
            };
            if generation != pad.home_hold_gen {
                return; // stale: the button was already released/re-pressed
            }
            pad.home_hold_task = None;
            info!("Home hold detected (slot {})", pad.player_slot);
            // Fleet-level dedup: publish the escape intent only on the 0->1 edge
            // of the latch, so two pads holding Home at once fire it once (a
            // double home-tap would toggle the drawer open then shut). The latch
            // clears (loop event arm) when no pad holds Home. Single-pad: the latch
            // toggles with the one pad, identical to the old behavior.
            //
            // Which intent depends on the presenter (see `hold_fire_intent`): the
            // Shell home publishes `home-hold` (idle reset), an APP presenter
            // publishes `home-tap` (the controllable overlay drawer over the app).
            if !sh.home_hold_active {
                sh.home_hold_active = true;
                let routed = route_presenter(sh.overlay_focus, sh.presenter);
                sh.publish(Event::Intent(hold_fire_intent(routed).into()));
            }
        }
        Internal::ComboGuardFired { fd, generation } => {
            let Some(pad) = fleet.pads.get_mut(&fd) else {
                return; // pad left
            };
            // Stale (disarmed/rearmed since) or no longer buffering: ignore.
            if generation != pad.combo_guard_gen || !pad.combo_armed {
                return;
            }
            pad.combo_guard_task = None;
            // The settle window elapsed with no combo: the buffered participants
            // were not a combo chord. Replay them to the app (in order) via the
            // current routed presenter and disarm. If the presenter changed since
            // arming, a transition already reset the buffer, so this is a no-op.
            let routed = route_presenter(sh.overlay_focus, sh.presenter);
            pad.replay_combo_buffer(sh, routed);
            pad.disarm_combo_guard(sh);
        }
        Internal::ComboEndSessionFired { fd, generation } => {
            let Some(pad) = fleet.pads.get_mut(&fd) else {
                return; // pad left
            };
            if generation != pad.combo_gen {
                return; // stale: a prior combo timer that was cancelled
            }
            pad.combo_task = None;
            // Per-pad-complete: this pad must still hold the whole combo.
            if state::subset_held(&config::COMBO_KEYS, &pad.held_keys) {
                info!("End-session combo detected (slot {})", pad.player_slot);
                sh.publish(Event::ComboEndSession);
            }
        }
        Internal::CaptureTimeout(generation) => {
            if generation != sh.capture_gen {
                return; // stale: the capture was already resolved/cancelled
            }
            sh.capture_timeout_task = None;
            if let Some(r) = sh.pending_capture.take() {
                let _ = r.send(resp_timeout());
            }
        }
        Internal::FocusSettle { generation } => {
            // The follow-focus settle debounce elapsed. Ignore a superseded tick
            // (focus moved again before this timer fired) — only the newest
            // pending focus is applied, so a launch/close flap burst yields a
            // single net presenter transition.
            if generation != sh.focus_settle_gen {
                return;
            }
            // This is the live timer firing (not a superseded one): it has
            // completed by sending this message, so drop its now-finished handle.
            // A superseded timer never reaches here (gen mismatch above), so this
            // can't clear the handle of a newer armed timer.
            sh.focus_settle_task = None;
            if let Some(class) = sh.pending_focus_class.take() {
                apply_focus_change(sh, fleet, &class);
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Axis {
    X,
    Y,
}
