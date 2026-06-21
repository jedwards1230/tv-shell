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
use std::collections::{HashMap, HashSet};
use std::os::fd::{AsRawFd, RawFd};
use std::path::PathBuf;
use std::time::Duration;
use tokio::sync::{broadcast, mpsc};
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
    /// A pending `capture-next` timed out (fleet-level).
    CaptureTimeout(u64),
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

    /// The active presenter (Phase 5). Starts in [`Presenter::Shell`] (the shell
    /// boots focused). `release`/`grab` flip it; per-pad event routing
    /// (`handle_event`) branches on it.
    presenter: Presenter,

    /// Whether our logind session is the foreground (active) one. Maintained by
    /// `Control::SetSessionActive` (the `session` actor). While `false` the
    /// physical `EVIOCGRAB` is dropped on every pad and their events are ignored,
    /// so a VT-switched-to session (Plasma/Bigscreen) owns the controller.
    /// Orthogonal to `presenter`. Defaults `true`.
    session_active: bool,

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
        // Route by the active presenter, not the physical grab: the pad stays
        // grabbed in both modes (Phase 5), so `grabbed` no longer discriminates.
        match sh.presenter {
            Presenter::Shell => self.handle_shell(sh, ev),
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

            // Home button hold detection -> neutral `intent:home-*` (Phase 5).
            if code == cfg::BTN_MODE {
                if value == 1 {
                    self.start_home_hold(sh);
                } else if value == 0 {
                    if let Some(t) = self.home_hold_task.take() {
                        t.abort();
                        // Invalidate a possibly-queued HomeHoldFired so the tap
                        // doesn't also produce `intent:home-hold`.
                        self.home_hold_gen = sh.next_generation();
                        sh.publish(Event::Intent("home-tap".into()));
                    }
                }
            }

            // View/Select button (single press) opens the right-edge Session
            // QAM (#218): emit the `overlay:session` deep-link intent the shell
            // routes to SessionQAM. Shell-only — deliberately NOT mirrored in
            // `handle_game`, so it never interferes with the in-game force-quit
            // combo (Back+Home+LB+RB, which also uses BTN_SELECT) or game input.
            // The shell opens the QAM both on the home screen and over a running
            // local app (`appRunning` keeps the Shell presenter, so this handler
            // still runs). Over a Moonlight *stream* the presenter is Game and
            // `handle_game` runs instead — which does NOT mirror this intercept,
            // so streams are left untouched. BTN_SELECT has no default key
            // binding, so nothing else consumes this press.
            if code == cfg::BTN_SELECT && value == 1 {
                sh.publish(Event::Intent("overlay:session".into()));
            }

            // LB/RB -> mouse left/right click.
            if code == cfg::BTN_TL {
                sh.emit_mouse_button(cfg::BTN_LEFT, value);
            } else if code == cfg::BTN_TR {
                sh.emit_mouse_button(cfg::BTN_RIGHT, value);
            }

            // Map to keyboard (layered: game > player > global, #104).
            if let Some(mapped) = sh.resolved_key(self.player_slot, code) {
                sh.emit_key(mapped, value);
            }
        } else if et == EventType::ABSOLUTE {
            self.handle_abs(sh, code, value);
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

            // Home is always intercepted -> `intent:home-*`, never forwarded to
            // the game's virtual pad, so the shell overlay can come up over a
            // running game (#75 substrate).
            if code == cfg::BTN_MODE {
                if value == 1 {
                    self.start_home_hold(sh);
                } else if value == 0 {
                    if let Some(t) = self.home_hold_task.take() {
                        t.abort();
                        // Invalidate a possibly-queued HomeHoldFired so the tap
                        // doesn't also produce `intent:home-hold`.
                        self.home_hold_gen = sh.next_generation();
                        sh.publish(Event::Intent("home-tap".into()));
                    }
                }
                return; // never forward Home to the game
            }

            // Forward the raw button to the clean virtual pad (the game's input).
            self.forward_to_virtual_pad(ev);
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
            // In the game presenter a stream/app owns the screen, so also send
            // the Moonlight force-quit chord on the shared virtual keyboard. The
            // shell presenter has no app to quit. (Pre-Phase-5 this keyed off the
            // grab; the pad now stays grabbed in both modes, so it keys off the
            // presenter instead.) Handoff (#221) is also a streaming presenter, so
            // it fires the chord too.
            if matches!(sh.presenter, Presenter::Game | Presenter::Handoff) {
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

    fn start_home_hold(&mut self, sh: &mut Shared) {
        if let Some(t) = self.home_hold_task.take() {
            t.abort();
        }
        let generation = sh.next_generation();
        self.home_hold_gen = generation;
        let tx = sh.internal_tx.clone();
        let fd = self.fd;
        self.home_hold_task = Some(tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs_f64(config::HOME_HOLD_SECS)).await;
            let _ = tx.send(Internal::HomeHoldFired { fd, generation }).await;
        }));
    }

    // --- presenter entry/exit (Phase 5) ----------------------------------

    /// Enter the game presenter: create this pad's clean virtual gamepad (if it
    /// doesn't already have one) and register its fd so discovery never grabs it.
    /// The physical pad stays grabbed; subsequent events route through
    /// [`PadDevice::handle_game`] onto the virtual pad. Idempotent.
    fn enter_game(&mut self, sh: &mut Shared) {
        if self.virtual_pad.is_some() {
            return;
        }
        match build_virtual_pad(self.event_stream.device(), self.player_slot) {
            Ok(mut vpad) => {
                register_vpad_devnodes(&mut sh.reg, &mut vpad);
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
    fn status_string(&self, presenter: Presenter) -> String {
        let connected = !self.pads.is_empty();
        let grabbed = presenter == Presenter::Shell;
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

/// Entry point: build the daemon and run the event loop until `Shutdown`.
///
/// Receives external `config:changed` signals from the file-watch actor via a
/// dedicated [`tokio::sync::Notify`] (not the global broadcast bus) so that
/// the input runtime does not hold a permanent broadcast receiver — which
/// would prevent `receiver_count()==0` fast-paths like `notify_held_buttons()`.
/// Both paths (IPC `set-config` → `Control::ConfigChanged` and file-watch →
/// `config_changed` notification) converge on the same `apply_config_changed`
/// helper, so cache state is always consistent (#163).
pub async fn run(
    mut control_rx: mpsc::Receiver<Control>,
    events: broadcast::Sender<Event>,
    config_changed: std::sync::Arc<tokio::sync::Notify>,
    metrics: std::sync::Arc<crate::metrics::Metrics>,
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
        session_active: true,
        // rumble_enabled is derived from the single offloaded startup settings
        // read (M3), superseding origin/main's inline config::rumble_enabled call.
        rumble_enabled,
        metrics,
    };
    let mut fleet = Fleet::new();

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
                    }
                }
            }
        }

        Control::Shutdown => return false,
    }
    true
}

/// Switch the fleet to the **shell presenter** (the `grab` IPC). Per-fleet mode
/// toggle (Phase 5): set the mode, ensure every pad is physically grabbed, and
/// tear down any per-player virtual gamepads. The physical grab is *kept* — the
/// shell presenter routes pad input to nav keys + `intent:*` on the shared
/// virtual keyboard/mouse.
fn grab_all(sh: &mut Shared, fleet: &mut Fleet) {
    info!(pads = fleet.pads.len(), "presenter -> Shell (grab)");
    sh.metrics.inc_transitions();
    sh.presenter = Presenter::Shell;
    for pad in fleet.pads.values_mut() {
        pad.grab(sh); // no-op if already grabbed; re-grabs if somehow released
        pad.enter_shell(sh);
    }
}

/// Switch the fleet to the **game presenter** (the `release` IPC). Per-fleet
/// mode toggle (Phase 5): set the mode, **keep** the physical grab (so nothing
/// leaks to the compositor), and create one clean virtual gamepad per pad. The
/// game reads the virtual pads; Home is intercepted into `intent:home-*`.
fn release_all(sh: &mut Shared, fleet: &mut Fleet) {
    info!(pads = fleet.pads.len(), "presenter -> Game (release)");
    sh.metrics.inc_transitions();
    sh.presenter = Presenter::Game;
    for pad in fleet.pads.values_mut() {
        // Keep the physical grab; only ensure it's grabbed (it is, post-join).
        pad.grab(sh);
        pad.enter_game(sh);
    }
}

/// Switch the fleet to the **handoff presenter** (the `handoff` IPC, #221). Hand
/// the physical pads directly to a Moonlight stream: drop any virtual twin and
/// **release** the physical `EVIOCGRAB` so SDL/Moonlight reads the real evdev
/// node (true handoff, no virtual pad). The daemon keeps reading events so the
/// session stays active and the safety combos still arm. Both `enter_shell`
/// (drop vpad) and `ungrab` are idempotent.
fn handoff_all(sh: &mut Shared, fleet: &mut Fleet) {
    info!(pads = fleet.pads.len(), "presenter -> Handoff (handoff)");
    sh.metrics.inc_transitions();
    sh.presenter = Presenter::Handoff;
    for pad in fleet.pads.values_mut() {
        pad.enter_shell(sh); // drop any virtual pad
        pad.ungrab(sh); // release the physical grab so SDL reads the real node
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
        //   * Shell — grab so its input drives nav (default).
        //   * Game  — grab + clean virtual gamepad (a 2nd player joining a stream
        //     that runs through the virtual-pad path).
        //   * Handoff (#221) — leave it UNGRABBED so SDL/Moonlight reads the real
        //     evdev node directly, exactly like the pads already handed off.
        match sh.presenter {
            Presenter::Shell => pad.grab(sh),
            Presenter::Game => {
                pad.grab(sh);
                pad.enter_game(sh);
            }
            Presenter::Handoff => { /* leave ungrabbed — SDL reads it directly */ }
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
            // Fleet-level dedup: publish `intent:home-hold` only on the 0->1 edge
            // of the latch, so two pads holding Home at once fire it once. The
            // latch clears (loop event arm) when no pad holds Home. Single-pad:
            // the latch toggles with the one pad, identical to the old behavior.
            if !sh.home_hold_active {
                sh.home_hold_active = true;
                sh.publish(Event::Intent("home-hold".into()));
            }
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
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Axis {
    X,
    Y,
}
