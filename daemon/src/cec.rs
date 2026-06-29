//! HDMI-CEC subsystem (#94): a single-owner async actor owning one persistent
//! libcec connection via `cec-rs`. It answers request/response queries over an
//! `mpsc` of [`CecReq`] and pushes `cec:*` [`Event`]s onto the shared broadcast
//! bus.
//!
//! Handles: READ via `cec-scan` / `cec-device` / `cec-health`, ACTIONS via
//! `cec-power-on` / `cec-power-off` / `cec-active-source`, plus an on-demand
//! `cec-test` poll probe. Pushes `cec:device:<json>` events when devices are
//! discovered/updated, `cec:power:<json>` when power status changes, and
//! `cec:health:<json>` when the transmit-wedge health state changes.
//!
//! **Transmit-wedge health (#19).** The Pulse-Eight adapter periodically enters
//! a state where libcec opens + RECEIVES fine but every TRANSMIT returns
//! `TransmitFailed`. A pure [`protocol::CecHealthState`] in the blocking worker
//! tracks the last transmit outcome (Unknown/Ok/Failing); every transmit site
//! (power on/off, active-source, wake/standby) and a side-effect-free poll probe
//! (folded into `cec-scan` and run explicitly by `cec-test`) update it, and a
//! `cec:health` event fires on each real transition.
//!
//! **Remote input -> navigation.** When the `GAME_SHELL_CEC_LIFECYCLE` flag is
//! on, the worker registers a libcec key-press callback. Each TV/AVR remote
//! button (a `CecUserControlCode`) arriving on the CEC bus is forwarded over a
//! std channel to a dedicated forwarder thread, debounced (initial press only),
//! mapped to a nav action, and injected onto the input runtime's control
//! channel as a `Control::Key` — the SAME synthesized keyboard event the
//! gamepad d-pad produces. This replaces the retired kernel `pulse8-cec` evdev
//! input device. Off by default, so dev/CI never inject keys.
//!
//! Linux-only AND feature-gated (`cec`): libcec is a Linux/udev C library, and
//! `libcec-sys` links it at build time (needs libcec-dev + libclang-dev), so
//! `lib.rs` declares this module under
//! `#[cfg(all(target_os = "linux", feature = "cec"))]`. Default builds — the
//! Linux CI default leg and macOS dev boxes — exclude it entirely, preserving
//! the no-system-C-deps invariant shared by evdev/zbus/bluer.
//!
//! **Sync-libcec-in-async bridge.** libcec's `CecConnection` is `!Send` and its
//! calls are blocking/callback-driven, so a DEDICATED BLOCKING WORKER owns the
//! connection for the actor's lifetime (one persistent handle — re-opening per
//! call is exactly the subprocess churn that caused #16's flaky detection).
//! The async [`run`] loop drains the tokio `mpsc<CecReq>`, forwards each as a
//! [`WorkerReq`] over a `std::sync::mpsc::sync_channel`, and awaits the reply on
//! a per-request `sync_channel::<String>(1)` — the wait itself wrapped in
//! `spawn_blocking` so the reactor is never blocked. Nothing libcec is held
//! across an `.await`; the async side only touches channels.
//!
//! **cec-rs 12.0.1 API notes** (this is the un-revert of the 8.0.1 build, #94).
//! The safe wrapper still exposes only `get_device_power_status`,
//! `send_power_on_devices`, `send_standby_devices`, `set_active_source`, and
//! `get_active_source`. The per-device metadata calls (OSD name, physical
//! address, vendor id, active-device enumeration) exist in libcec but are NOT
//! wrapped by cec-rs 12.0.1 (they're commented-out TODOs in the crate source),
//! so the scan result still carries only `logicalAddress` + `powerStatus`. The
//! bus is enumerated by probing the 16 logical addresses via
//! `get_device_power_status` (Unknown ≡ absent). `CecConnectionCfg::open(self)`
//! consumes the config. The 8.0.1 `CecLogicalAddress::try_from(i32)` call shape
//! is gone in 12.x — we enumerate the known variants directly instead.

use crate::protocol::{self, Event};
use crate::state::{Control, Reply};
use anyhow::Result;
use std::sync::mpsc as std_mpsc;
use tokio::sync::{broadcast, mpsc, oneshot};

// ---------------------------------------------------------------------------
// Request type.
// ---------------------------------------------------------------------------

/// Requests from the IPC server to the CEC actor.  Each carries a `oneshot`
/// reply with a fully-formatted wire string.
#[derive(Debug)]
pub enum CecReq {
    /// `cec-scan` -> compact JSON array of device objects.
    Scan(Reply),
    /// `cec-device <addr>` -> compact JSON object for the given logical
    /// address, or `error:*` if absent.
    Device { addr: String, reply: Reply },
    /// `cec-power-on <addr>` -> `ok` / `error:*`.
    PowerOn { addr: String, reply: Reply },
    /// `cec-power-off <addr>` -> `ok` / `error:*`.
    PowerOff { addr: String, reply: Reply },
    /// `cec-active-source` -> `ok` / `error:*`.
    ActiveSource(Reply),
    /// `cec-health` -> the current transmit-wedge health as a compact JSON object
    /// `{transmit,since,lastError}` (#19). READ-ONLY: it returns the last-known
    /// transmit state and never touches the bus, so it's safe to poll cheaply.
    Health(Reply),
    /// `cec-test` -> run an explicit side-effect-free CEC poll probe, update the
    /// transmit-health, emit `cec:health` on a change, and reply with the same
    /// JSON object as `cec-health` (#19).
    Test(Reply),
    /// Lifecycle wake (start / resume-from-suspend): power on AVR (addr 5) then
    /// TV (addr 0), then claim active source. Replies `ok` / `error:*`. A no-op
    /// (still replying `ok`) when the lifecycle flag is off OR when the
    /// `cecFocusOnWake` setting is false, so callers never drive the bus on
    /// dev/CI hosts or when the user has opted out of focus-on-wake. Not
    /// exposed as a manual IPC command.
    WakeSequence(Reply),
    /// Lifecycle standby (suspend / session-end SIGTERM): send CEC standby to TV
    /// (addr 0) then AVR (addr 5). Replies `ok` / `error:*`. A no-op (`ok`) when
    /// the lifecycle flag is off. Not exposed as a manual IPC command.
    StandbyAll(Reply),
}

/// CEC logical address of the AV receiver (Audiosystem).
const AVR_ADDR: i32 = 5;
/// CEC logical address of the TV.
const TV_ADDR: i32 = 0;
/// Delay between waking the TV and claiming active source, giving the display
/// time to come out of standby before it's switched to our input. Mirrors the
/// `cec_wake_delay` concept from the prior `living-room-cec` shell flow.
const WAKE_ACTIVE_SOURCE_DELAY: std::time::Duration = std::time::Duration::from_millis(1500);

/// Whether daemon-owned CEC lifecycle (wake-on-start/resume + standby-on-
/// suspend/SIGTERM) is enabled. Read from `[cec].lifecycle` in `config.toml`.
/// **Default OFF** so a plain `cargo run`, CI, or a dev box never drives a real
/// CEC bus — it's opted into on the deploy host. The manual `cec-*` IPC commands
/// are unaffected by this flag.
pub fn lifecycle_enabled() -> bool {
    crate::daemon_config::global().cec.lifecycle
}

// ---------------------------------------------------------------------------
// CEC remote input -> navigation key mapping.
// ---------------------------------------------------------------------------

/// Map a CEC user-control code (a TV/AVR remote button arriving over the CEC
/// bus) to the `config::key_for_action` name the input runtime already
/// understands. Returns `None` for codes we deliberately ignore (media
/// transport, menus, numbers, colour buttons, etc. — follow-up scope). The six
/// mapped codes reuse the EXACT same nav vocabulary the gamepad emits, so a
/// remote keypress lands as the same synthesized keyboard event:
///
/// | CEC code | action | key |
/// |----------|--------|-----|
/// | `Up`     | `up`    | KEY_UP    |
/// | `Down`   | `down`  | KEY_DOWN  |
/// | `Left`   | `left`  | KEY_LEFT  |
/// | `Right`  | `right` | KEY_RIGHT |
/// | `Select` | `select`| KEY_ENTER |
/// | `Exit`   | `back`  | KEY_ESC   |
///
/// Pure (no libcec calls), so it is unit-tested in the `cec` feature leg. It
/// references `CecUserControlCode` (a cec-rs type) so it must live here in the
/// feature-gated module, not in the cross-platform `protocol.rs`.
fn cec_key_action(code: cec_rs::CecUserControlCode) -> Option<&'static str> {
    use cec_rs::CecUserControlCode as K;
    match code {
        K::Up => Some("up"),
        K::Down => Some("down"),
        K::Left => Some("left"),
        K::Right => Some("right"),
        K::Select => Some("select"),
        // NOTE: on-device testing (2026-06) confirmed Up/Down/Left/Right + Select
        // via the TV remote. `Exit` is mapped but UNTESTED (the test TV remote had
        // no Exit key), and an AVR remote produced no CEC user-control codes at all
        // — so AVR-remote nav is unverified. Revisit if a remote with Exit/known
        // codes is available.
        K::Exit => Some("back"),
        // RootMenu / media transport / numbers / colour keys: ignored for now.
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Internal worker message (async → blocking boundary).
// ---------------------------------------------------------------------------

enum WorkerReq {
    Scan(std_mpsc::SyncSender<String>),
    Device {
        addr: String,
        tx: std_mpsc::SyncSender<String>,
    },
    PowerOn {
        addr: String,
        tx: std_mpsc::SyncSender<String>,
    },
    PowerOff {
        addr: String,
        tx: std_mpsc::SyncSender<String>,
    },
    ActiveSource(std_mpsc::SyncSender<String>),
    WakeSequence(std_mpsc::SyncSender<String>),
    StandbyAll(std_mpsc::SyncSender<String>),
    Health(std_mpsc::SyncSender<String>),
    Test(std_mpsc::SyncSender<String>),
    Shutdown,
}

// ---------------------------------------------------------------------------
// CEC libcec helpers (cec-rs 12.0.1 API).
// ---------------------------------------------------------------------------

/// The 16 CEC logical addresses in numeric order (TV=0 … Unregistered=15).
/// We enumerate these directly rather than converting raw `i32`s into the
/// `cec-rs` enum: 12.x dropped the `TryFrom<i32>` impl the 8.0.1 code used, and
/// `from_repr` takes the FFI integer typedef. A fixed table is both portable
/// and unambiguous.
const LOGICAL_ADDRESSES: [cec_rs::CecLogicalAddress; 16] = [
    cec_rs::CecLogicalAddress::Tv,
    cec_rs::CecLogicalAddress::Recordingdevice1,
    cec_rs::CecLogicalAddress::Recordingdevice2,
    cec_rs::CecLogicalAddress::Tuner1,
    cec_rs::CecLogicalAddress::Playbackdevice1,
    cec_rs::CecLogicalAddress::Audiosystem,
    cec_rs::CecLogicalAddress::Tuner2,
    cec_rs::CecLogicalAddress::Tuner3,
    cec_rs::CecLogicalAddress::Playbackdevice2,
    cec_rs::CecLogicalAddress::Recordingdevice3,
    cec_rs::CecLogicalAddress::Tuner4,
    cec_rs::CecLogicalAddress::Playbackdevice3,
    cec_rs::CecLogicalAddress::Reserved1,
    cec_rs::CecLogicalAddress::Reserved2,
    cec_rs::CecLogicalAddress::Freeuse,
    cec_rs::CecLogicalAddress::Unregistered,
];

/// Map a numeric logical address (as received on the wire) to the `cec-rs`
/// enum.  Returns `None` for anything outside 0..=15.
fn logical_from_i32(n: i32) -> Option<cec_rs::CecLogicalAddress> {
    if (0..=15).contains(&n) {
        Some(LOGICAL_ADDRESSES[n as usize])
    } else {
        None
    }
}

/// Map a cec-rs `CecPowerStatus` to its wire word.
fn power_status_word(ps: cec_rs::CecPowerStatus) -> &'static str {
    match ps {
        cec_rs::CecPowerStatus::On => "on",
        cec_rs::CecPowerStatus::Standby => "standby",
        cec_rs::CecPowerStatus::InTransitionStandbyToOn => "waking",
        cec_rs::CecPowerStatus::InTransitionOnToStandby => "sleeping",
        cec_rs::CecPowerStatus::Unknown => "unknown",
    }
}

/// Build a compact-JSON device object for a single logical address, or `None`
/// if the device is not present on the bus (power status Unknown means the
/// device did not respond). cec-rs 12.0.1 wraps no per-device metadata query
/// beyond power status, so the object carries only `logicalAddress` +
/// `powerStatus`. The wire-format builder lives in `protocol.rs` (pure,
/// unit-tested in the default leg); we pass it the already-mapped word.
fn device_json(
    conn: &cec_rs::CecConnection,
    idx: i32,
    addr: cec_rs::CecLogicalAddress,
) -> Option<String> {
    let ps = conn.get_device_power_status(addr);
    if matches!(ps, cec_rs::CecPowerStatus::Unknown) {
        return None;
    }
    Some(protocol::cec_device_json(idx, power_status_word(ps)))
}

/// Build a compact-JSON array of all active CEC devices by probing each of the
/// 16 CEC logical addresses (`get_device_power_status`; Unknown ≡ absent).
/// Probe every logical address once, returning the JSON object for each device
/// that responds. Callers reuse this single sweep for both the `cec-scan`
/// response body and the `cec:device` push events (avoids double-probing the
/// blocking libcec bus on every scan).
fn scan_devices(conn: &cec_rs::CecConnection) -> Vec<String> {
    let mut entries = Vec::new();
    for (idx, addr) in LOGICAL_ADDRESSES.iter().enumerate() {
        if let Some(obj) = device_json(conn, idx as i32, *addr) {
            entries.push(obj);
        }
    }
    entries
}

// ---------------------------------------------------------------------------
// Stale-bus auto-recovery (cec active-source "TransmitFailed" loop).
// ---------------------------------------------------------------------------

/// Rebuild and reopen a BASIC libcec connection for recovery, returning the
/// fresh handle (or `None` if the rebuild/open fails).
///
/// When the CEC bus/adapter goes stale, a held connection's transmit ops
/// (`set_active_source`, power on/off) start returning `Err` ("TransmitFailed")
/// indefinitely — previously only a daemon restart cleared it. Dropping and
/// reopening the connection mirrors that restart in-process.
///
/// The recovery config is deliberately MINIMAL: same device name / playback
/// device / `activate_source(false)` as the initial open, but WITHOUT the
/// key-press callback. The CEC remote-input forwarder (a secondary feature) is
/// only attached on the initial open in `blocking_worker`; a recovery reopen
/// does NOT re-attach remote-key input — that resumes on the next daemon
/// restart. Re-arming the callback would mean threading the (already-consumed)
/// `control_tx` and a fresh forwarder thread through here, which is out of scope
/// for a transmit-failure retry.
fn reopen_connection() -> Option<cec_rs::CecConnection> {
    tracing::warn!("cec: reopening libcec connection (recovery after transmit failure)");
    cec_rs::CecConnectionCfgBuilder::default()
        .device_name("game-shell".to_string())
        .device_types(cec_rs::CecDeviceTypeVec::new(
            cec_rs::CecDeviceType::PlaybackDevice,
        ))
        .activate_source(false)
        .build()
        .ok()?
        .open()
        .ok()
}

/// Minimum spacing between recovery reopens. A stale/wedged adapter can make
/// EVERY transmit fail; without a cooldown, back-to-back failures would
/// rapid-fire `open()`/close on the Pulse-Eight adapter and churn it into a
/// hardware-stuck state (observed live). Once per 30s is enough to recover a
/// genuinely transient stale bus while never hammering the hardware.
const REOPEN_COOLDOWN: std::time::Duration = std::time::Duration::from_secs(30);

/// Whether a recovery reopen is allowed now, given the last reopen instant.
/// True if we've never reopened or the cooldown has elapsed; updates
/// `last_reopen` to now when it returns true (the caller is about to reopen).
fn reopen_allowed(last_reopen: &mut Option<std::time::Instant>) -> bool {
    let ready = match last_reopen {
        None => true,
        Some(t) => t.elapsed() >= REOPEN_COOLDOWN,
    };
    if ready {
        *last_reopen = Some(std::time::Instant::now());
    }
    ready
}

/// Run a transmit op against `conn`; on failure, reopen the connection ONCE and
/// retry the op a single time — but ONLY if the reopen cooldown has elapsed
/// (`last_reopen`). Bounded two ways: at most one reopen+retry per call (no
/// infinite loop), AND at most one reopen per [`REOPEN_COOLDOWN`] (no adapter
/// churn). Within cooldown, the reopen is skipped and the ORIGINAL error is
/// returned. Reads (`scan`, `device`) deliberately do NOT route through this;
/// recovery is driven by the transmit ops (active-source / power) that exhibit
/// the stale-bus "TransmitFailed" behavior.
fn with_cec_reconnect<T, E: std::fmt::Debug>(
    conn: &mut cec_rs::CecConnection,
    last_reopen: &mut Option<std::time::Instant>,
    label: &str,
    mut op: impl FnMut(&cec_rs::CecConnection) -> Result<T, E>,
) -> Result<T, E> {
    match op(conn) {
        Ok(v) => Ok(v),
        Err(e) => {
            if !reopen_allowed(last_reopen) {
                tracing::warn!(
                    "cec: {label} failed ({e:?}); reopen on cooldown — skipping recovery"
                );
                return Err(e);
            }
            tracing::warn!("cec: {label} failed ({e:?}); reopening libcec connection and retrying");
            match reopen_connection() {
                Some(fresh) => {
                    *conn = fresh;
                    op(conn)
                }
                None => Err(e),
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Transmit-wedge health tracking (#19).
// ---------------------------------------------------------------------------

/// Current wall-clock as epoch milliseconds (UTC). Used to stamp the `since`
/// field of a CEC health transition. A pre-epoch clock (impossible in practice)
/// degrades to `0` rather than panicking — this runs on a long-lived daemon.
fn now_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Fold a transmit outcome into `health` and broadcast `cec:health` IFF the
/// variant CHANGED (so subscribers see only real transitions, not every probe).
/// `ok` = the transmit round-tripped; `err` is the failure message recorded when
/// `ok` is false. Centralizes the record-then-maybe-emit step shared by every
/// transmit site (power on/off, active-source, wake/standby, the poll probe).
fn note_health(
    health: &mut protocol::CecHealthState,
    events_tx: &broadcast::Sender<Event>,
    ok: bool,
    err: &str,
) {
    let changed = if ok {
        health.record_success(now_millis())
    } else {
        health.record_failure(err, now_millis())
    };
    if changed {
        let _ = events_tx.send(Event::CecHealth(health.to_json()));
    }
}

/// A side-effect-free CEC transmit probe used to refresh the transmit-health.
///
/// **cec-rs 12.0.1 API finding.** There is NO dedicated `poll_device` / ping
/// wrapper in cec-rs 12.0.1. The only primitives that BOTH transmit AND return a
/// `Result` are `send_power_on_devices` / `send_standby_devices` /
/// `set_active_source` (all have side effects — they power/standby a device or
/// steal the active source) and the generic `transmit(CecCommand)`. A CEC
/// **POLL** message — `opcode_set = false`, empty parameters, the `<Polling
/// Message>` / "ping" — has NO side effect: it's just the header byte,
/// transmitted and ACK-checked, exactly what libcec uses internally for device
/// detection. So we hand-build a POLL `CecCommand` addressed to the TV (logical
/// addr 0) and send it via `transmit`, which returns `Err(TransmitFailed)` on a
/// wedged adapter — the clean failing-signal that `get_device_power_status`
/// cannot give (it swallows transmit failures into `Unknown`).
///
/// The initiator is our own primary logical address (queried from libcec; falls
/// back to `Unregistered` if unavailable) so the poll is well-formed. This
/// deliberately does NOT route through `with_cec_reconnect`: the probe is a
/// health READ, not a recovery driver, so a failing poll must not trigger a
/// reopen storm on the Pulse-Eight adapter.
///
/// Caveat: a poll to addr 0 ACKs whenever the TV is plugged in (CEC devices ACK
/// polls even in standby), so on this AV setup it's a faithful bus-alive check.
/// If the TV were fully unplugged a healthy adapter would still report failing —
/// acceptable here (htpc-1 always has a TV on the bus), and the scan path
/// only falls back to the poll when the bus shows zero devices.
fn poll_probe(conn: &cec_rs::CecConnection) -> Result<(), cec_rs::CecConnectionResultError> {
    let initiator = conn
        .get_logical_addresses()
        .ok()
        .map(|la| cec_rs::CecLogicalAddress::from(la.primary))
        .unwrap_or(cec_rs::CecLogicalAddress::Unregistered);
    let poll = cec_rs::CecCommand {
        initiator,
        destination: cec_rs::CecLogicalAddress::Tv,
        ack: false,
        eom: false,
        // `opcode_set = false` makes this a POLL message; the opcode value itself
        // is ignored on the wire, but the field must hold a valid variant.
        opcode: cec_rs::CecOpcode::None,
        parameters: cec_rs::CecDatapacket(arrayvec::ArrayVec::new()),
        opcode_set: false,
        transmit_timeout: std::time::Duration::from_millis(1000),
    };
    conn.transmit(poll)
}

/// Run the poll probe and fold its outcome into `health`, emitting `cec:health`
/// on a change. Returns the current health JSON (the `cec-test` reply body).
fn run_poll_probe(
    conn: &cec_rs::CecConnection,
    health: &mut protocol::CecHealthState,
    events_tx: &broadcast::Sender<Event>,
) -> String {
    match poll_probe(conn) {
        Ok(()) => note_health(health, events_tx, true, ""),
        Err(e) => note_health(
            health,
            events_tx,
            false,
            &format!("cec-test poll failed: {e:?}"),
        ),
    }
    health.to_json()
}

// ---------------------------------------------------------------------------
// Lifecycle sequences (blocking-worker context only — they touch libcec).
// ---------------------------------------------------------------------------

/// Power on the AVR (addr 5) then the TV (addr 0), wait for the display to come
/// out of standby, then announce ourselves as the active source. Returns a wire
/// response: `ok` if every step succeeded, otherwise the first `error:*`. Runs
/// only inside the blocking worker (it issues blocking libcec calls). Emits a
/// `cec:power` event per successful power-on so subscribers see the change.
fn wake_sequence(conn: &cec_rs::CecConnection, events_tx: &broadcast::Sender<Event>) -> String {
    for addr in [AVR_ADDR, TV_ADDR] {
        let Some(logical) = logical_from_i32(addr) else {
            return protocol::resp_error(&format!("invalid lifecycle address {addr}"));
        };
        if let Err(e) = conn.send_power_on_devices(logical) {
            return protocol::resp_error(&format!("wake power-on {addr} failed: {e:?}"));
        }
        let ps = conn.get_device_power_status(logical);
        let _ = events_tx.send(Event::CecPower(protocol::cec_power_json(
            &addr.to_string(),
            power_status_word(ps),
        )));
    }
    // Give the TV time to leave standby before switching it to our input.
    std::thread::sleep(WAKE_ACTIVE_SOURCE_DELAY);
    match conn.set_active_source(cec_rs::CecDeviceType::PlaybackDevice) {
        Ok(()) => protocol::resp_ok(),
        Err(e) => protocol::resp_error(&format!("wake active-source failed: {e:?}")),
    }
}

/// Send CEC standby to the TV (addr 0) then the AVR (addr 5). Returns a wire
/// response: `ok` if both standby commands succeeded, otherwise the first
/// `error:*`. Runs only inside the blocking worker. Emits a `cec:power` event
/// per successful standby.
fn standby_all(conn: &cec_rs::CecConnection, events_tx: &broadcast::Sender<Event>) -> String {
    for addr in [TV_ADDR, AVR_ADDR] {
        let Some(logical) = logical_from_i32(addr) else {
            return protocol::resp_error(&format!("invalid lifecycle address {addr}"));
        };
        if let Err(e) = conn.send_standby_devices(logical) {
            return protocol::resp_error(&format!("standby {addr} failed: {e:?}"));
        }
        let ps = conn.get_device_power_status(logical);
        let _ = events_tx.send(Event::CecPower(protocol::cec_power_json(
            &addr.to_string(),
            power_status_word(ps),
        )));
    }
    protocol::resp_ok()
}

// ---------------------------------------------------------------------------
// Blocking worker: owns the CecConnection for its lifetime.
// ---------------------------------------------------------------------------

/// Map a libcec open-handshake failure to the wire `reason` word surfaced on the
/// AV Control page (#19 follow-up). `open()` runs `libcec_detect_adapters`
/// internally and returns `NoAdapterFound` when ZERO adapters are present (no
/// hardware) versus `AdapterOpenFailed` when an adapter IS found but
/// `libcec_open` fails — the hardware "wedge" where the actionable truth is
/// "adapter detected but not responding — re-seat it", NOT "no adapter".
///
/// `LibInitFailed` / `CallbackRegistrationFailed` (and the earlier
/// `builder.build()` failure, handled by passing the constant directly) all mean
/// libcec is present but the connection couldn't be brought up, so they map to
/// `adapter_open_failed` too. `TransmitFailed` can't occur on open (it's a
/// transmit-time error) but falls into the same bucket defensively.
fn open_failure_reason(e: &cec_rs::CecConnectionResultError) -> &'static str {
    match e {
        cec_rs::CecConnectionResultError::NoAdapterFound => "no_adapter",
        _ => "adapter_open_failed",
    }
}

/// Reply to every pending request when libcec can't be brought up so a
/// missing/wedged adapter never wedges a client (mirrors `power.rs`'s
/// drain-on-unavailable). `reason` is the structured open-failure reason word
/// (`no_adapter` / `adapter_open_failed`): `Health` and `Test` requests get the
/// STRUCTURED unavailable JSON (`{transmit:"unavailable",reason,…}`) so the AV
/// Control page can show an accurate per-case message; every OTHER request kind
/// keeps the bare `error:libcec unavailable` line.
fn drain_unavailable(rx: &std_mpsc::Receiver<WorkerReq>, reason: &str) {
    while let Ok(req) = rx.recv() {
        let err = protocol::resp_error("libcec unavailable");
        match req {
            WorkerReq::Scan(tx) => {
                let _ = tx.send(err);
            }
            WorkerReq::Device { tx, .. } => {
                let _ = tx.send(err);
            }
            WorkerReq::PowerOn { tx, .. } => {
                let _ = tx.send(err);
            }
            WorkerReq::PowerOff { tx, .. } => {
                let _ = tx.send(err);
            }
            WorkerReq::ActiveSource(tx) => {
                let _ = tx.send(err);
            }
            WorkerReq::WakeSequence(tx) => {
                let _ = tx.send(err);
            }
            WorkerReq::StandbyAll(tx) => {
                let _ = tx.send(err);
            }
            // Health/Test get the structured unavailable reply (with the reason),
            // not the bare error, so the page distinguishes no_adapter from
            // adapter_open_failed.
            WorkerReq::Health(tx) => {
                let _ = tx.send(protocol::cec_unavailable_json(reason, now_millis()));
            }
            WorkerReq::Test(tx) => {
                let _ = tx.send(protocol::cec_unavailable_json(reason, now_millis()));
            }
            WorkerReq::Shutdown => break,
        }
    }
}

/// Run the blocking libcec worker loop.  Owns the `CecConnection`; called from
/// `tokio::task::spawn_blocking`.  Returns once a `Shutdown` message is received
/// or `rx` is closed.  Broadcasts `cec:device` / `cec:power` push events on
/// `events_tx` after each scan or power command.
fn blocking_worker(
    rx: std_mpsc::Receiver<WorkerReq>,
    events_tx: broadcast::Sender<Event>,
    control_tx: mpsc::Sender<Control>,
) {
    // cec-rs 12.0.1: CecConnectionCfg is built via the derive_builder
    // CecConnectionCfgBuilder (owned pattern, so each setter consumes+returns
    // the builder); `open(self)` consumes the config.
    let mut builder = cec_rs::CecConnectionCfgBuilder::default()
        .device_name("game-shell".to_string())
        .device_types(cec_rs::CecDeviceTypeVec::new(
            cec_rs::CecDeviceType::PlaybackDevice,
        ))
        // Never auto-claim active source on open — claims are explicit only
        // (wake_sequence + the cec-active-source IPC). Prevents a daemon
        // start/restart/deploy from stealing the TV input. cec-rs strips the
        // Option, so the setter takes a bare bool.
        .activate_source(false);

    // Remote-input bridge (gated by the SAME lifecycle flag as wake/standby):
    // register a libcec key-press callback that forwards each CecKeypress over a
    // std channel to a dedicated forwarder thread (below). The callback fires on
    // libcec's OWN thread, so it must not block or re-enter libcec — it only does
    // a non-blocking `send` on a std mpsc (which is `Send`, satisfying the
    // `FnMut(CecKeypress) + Send` bound). When the flag is off, no callback is
    // registered at all, so dev/CI never inject keys.
    let key_rx = if lifecycle_enabled() {
        let (key_tx, key_rx) = std_mpsc::channel::<cec_rs::CecKeypress>();
        builder = builder.key_press_callback(Box::new(move |kp| {
            // Best-effort: a closed receiver (forwarder gone) just drops the key.
            let _ = key_tx.send(kp);
        }));
        Some(key_rx)
    } else {
        None
    };

    let cfg = match builder.build() {
        Ok(c) => c,
        Err(e) => {
            // A build failure means libcec is present but the connection couldn't
            // be brought up, so it's the `adapter_open_failed` ("re-seat it")
            // class — same actionable message as a failed open.
            let reason = "adapter_open_failed";
            tracing::warn!(
                "cec: failed to build CecConnectionCfg ({e:?}); replying unavailable ({reason}) to all requests"
            );
            // Broadcast once so subscribers (the AV Control page) update promptly
            // without waiting for a Health/Test request.
            let _ = events_tx.send(Event::CecHealth(protocol::cec_unavailable_json(
                reason,
                now_millis(),
            )));
            drain_unavailable(&rx, reason);
            return;
        }
    };
    // `mut` so a failing transmit op can drop + reopen the handle for recovery
    // (`with_cec_reconnect`); the initial open here keeps the key-press callback.
    let mut conn = match cfg.open() {
        Ok(c) => c,
        Err(e) => {
            // Distinguish "no hardware" (NoAdapterFound) from "adapter present but
            // won't open" (AdapterOpenFailed, the Pulse-Eight wedge) so the page
            // shows the right message. open() runs libcec_detect_adapters first.
            let reason = open_failure_reason(&e);
            tracing::warn!(
                "cec: failed to open libcec connection ({e:?}); replying unavailable ({reason}) to all requests"
            );
            // Broadcast once so subscribers update promptly (no Health/Test wait).
            let _ = events_tx.send(Event::CecHealth(protocol::cec_unavailable_json(
                reason,
                now_millis(),
            )));
            drain_unavailable(&rx, reason);
            return;
        }
    };

    tracing::info!("cec: libcec connection opened");

    // Spawn the remote-input forwarder thread. The blocking worker thread itself
    // is busy in its `rx.recv()` request loop, so a DEDICATED std thread drains
    // the key-press channel: it debounces, maps the CEC code to a nav action,
    // and pushes it onto the input runtime's control channel via
    // `blocking_send` (the tokio `mpsc::Sender` supports this from a non-async
    // thread). Only present when `key_rx` is `Some`, i.e. the lifecycle flag is
    // on. The thread exits when the callback's `key_tx` is dropped (connection
    // torn down) or the control channel closes.
    if let Some(key_rx) = key_rx {
        let control_tx = control_tx.clone();
        std::thread::Builder::new()
            .name("cec-input".into())
            .spawn(move || {
                while let Ok(kp) = key_rx.recv() {
                    // DEBOUNCE: libcec fires the callback with `duration == 0` on
                    // the INITIAL key-down, then again on release with the actual
                    // held duration (> 0). We act on the initial press only, so a
                    // single button push emits exactly one nav event (no repeat /
                    // double-fire on release).
                    if !kp.duration.is_zero() {
                        continue;
                    }
                    let Some(action) = cec_key_action(kp.keycode) else {
                        continue; // unmapped code (menu/media/etc.) — ignore.
                    };
                    // The input runtime's `Control::Key` carries a oneshot reply;
                    // we don't need the response (fire-and-forget nav injection),
                    // so we drop the receiver and ignore send errors.
                    let (reply_tx, _reply_rx) = oneshot::channel();
                    if control_tx
                        .blocking_send(Control::Key {
                            name: action.to_string(),
                            reply: reply_tx,
                        })
                        .is_err()
                    {
                        // Control channel closed (daemon shutting down): stop.
                        break;
                    }
                }
                tracing::debug!("cec: remote-input forwarder stopped");
            })
            .map_err(|e| tracing::warn!("cec: failed to spawn remote-input forwarder: {e}"))
            .ok();
    }

    // Wake-on-open: when daemon-owned CEC lifecycle is enabled AND
    // `cecFocusOnStartup` is true, run the wake sequence once at startup
    // (power on AVR + TV, claim active source). Off by default on both counts
    // — `lifecycle_enabled` is the master gate, `cecFocusOnStartup` defaults
    // false — so dev/CI and normal restarts never steal the TV input.
    let focus_on_startup = crate::config::cec_focus_on_startup(&crate::config::settings_path());
    if crate::config::should_focus(lifecycle_enabled(), focus_on_startup) {
        tracing::info!("cec: lifecycle + focus-on-startup enabled — running wake sequence on open");
        let resp = wake_sequence(&conn, &events_tx);
        if resp.starts_with("error:") {
            tracing::warn!("cec: wake-on-open failed: {resp}");
        }
    }

    // Cooldown clock for recovery reopens — gates `with_cec_reconnect` and the
    // wake/standby retry below so a persistently-failing bus can't churn the
    // adapter (see `REOPEN_COOLDOWN`). `None` until the first reopen.
    let mut last_reopen: Option<std::time::Instant> = None;

    // Transmit-wedge health (#19). Starts `Unknown` (no transmit attempted yet);
    // every transmit site below folds its outcome in via `note_health`, which
    // broadcasts `cec:health` only on a real variant transition.
    let mut health = protocol::CecHealthState::new(now_millis());

    while let Ok(req) = rx.recv() {
        match req {
            WorkerReq::Scan(tx) => {
                // Single bus sweep, reused for the response and the push events.
                let devices = scan_devices(&conn);
                for obj in &devices {
                    let _ = events_tx.send(Event::CecDevice(obj.clone()));
                }
                // Health refresh side effect (#19): so the QML 30s `cec-scan`
                // poll keeps the status line fresh with no user action. A
                // non-empty sweep means polls round-tripped (the adapter
                // transmits) -> success. An EMPTY sweep is ambiguous (a wedged
                // adapter OR genuinely no devices), so disambiguate with ONE
                // side-effect-free poll probe — never `with_cec_reconnect`, so a
                // failing probe can't churn the adapter.
                if !devices.is_empty() {
                    note_health(&mut health, &events_tx, true, "");
                } else {
                    run_poll_probe(&conn, &mut health, &events_tx);
                }
                let _ = tx.send(format!("[{}]", devices.join(",")));
            }
            WorkerReq::Device { addr, tx } => {
                let resp = match addr.parse::<i32>().ok().and_then(logical_from_i32) {
                    Some(logical) => {
                        let idx = addr.parse::<i32>().unwrap_or(-1);
                        match device_json(&conn, idx, logical) {
                            Some(obj) => {
                                let _ = events_tx.send(Event::CecDevice(obj.clone()));
                                obj
                            }
                            None => protocol::resp_error(&format!("no device at address {addr}")),
                        }
                    }
                    None => protocol::resp_error(&format!("invalid address {addr}")),
                };
                let _ = tx.send(resp);
            }
            WorkerReq::PowerOn { addr, tx } => {
                let resp = match addr.parse::<i32>().ok().and_then(logical_from_i32) {
                    // Transmit op: reopen + retry once on a stale-bus failure.
                    Some(logical) => {
                        match with_cec_reconnect(&mut conn, &mut last_reopen, "power-on", |c| {
                            c.send_power_on_devices(logical)
                        }) {
                            Ok(()) => {
                                note_health(&mut health, &events_tx, true, "");
                                let ps = conn.get_device_power_status(logical);
                                let payload =
                                    protocol::cec_power_json(&addr, power_status_word(ps));
                                let _ = events_tx.send(Event::CecPower(payload));
                                protocol::resp_ok()
                            }
                            Err(e) => {
                                let msg = format!("power-on failed: {e:?}");
                                note_health(&mut health, &events_tx, false, &msg);
                                protocol::resp_error(&msg)
                            }
                        }
                    }
                    None => protocol::resp_error(&format!("invalid address {addr}")),
                };
                let _ = tx.send(resp);
            }
            WorkerReq::PowerOff { addr, tx } => {
                let resp = match addr.parse::<i32>().ok().and_then(logical_from_i32) {
                    // Transmit op: reopen + retry once on a stale-bus failure.
                    Some(logical) => {
                        match with_cec_reconnect(&mut conn, &mut last_reopen, "power-off", |c| {
                            c.send_standby_devices(logical)
                        }) {
                            Ok(()) => {
                                note_health(&mut health, &events_tx, true, "");
                                let ps = conn.get_device_power_status(logical);
                                let payload =
                                    protocol::cec_power_json(&addr, power_status_word(ps));
                                let _ = events_tx.send(Event::CecPower(payload));
                                protocol::resp_ok()
                            }
                            Err(e) => {
                                let msg = format!("power-off failed: {e:?}");
                                note_health(&mut health, &events_tx, false, &msg);
                                protocol::resp_error(&msg)
                            }
                        }
                    }
                    None => protocol::resp_error(&format!("invalid address {addr}")),
                };
                let _ = tx.send(resp);
            }
            WorkerReq::ActiveSource(tx) => {
                // Transmit op: reopen + retry once on a stale-bus failure (this is
                // the exact op that loops "TransmitFailed" forever on a stale bus).
                let resp =
                    match with_cec_reconnect(&mut conn, &mut last_reopen, "active-source", |c| {
                        c.set_active_source(cec_rs::CecDeviceType::PlaybackDevice)
                    }) {
                        Ok(()) => {
                            note_health(&mut health, &events_tx, true, "");
                            protocol::resp_ok()
                        }
                        Err(e) => {
                            let msg = format!("active-source failed: {e:?}");
                            note_health(&mut health, &events_tx, false, &msg);
                            protocol::resp_error(&msg)
                        }
                    };
                let _ = tx.send(resp);
            }
            WorkerReq::WakeSequence(tx) => {
                // The sequence is several transmit ops; it returns a wire string
                // (`ok` / `error:*`) rather than a Result. On a leading `error:`,
                // reopen the connection once and retry the whole sequence a single
                // time — but only if the reopen cooldown has elapsed (bounded: no
                // loop AND no adapter churn).
                let mut resp = wake_sequence(&conn, &events_tx);
                if resp.starts_with("error:") && reopen_allowed(&mut last_reopen) {
                    if let Some(fresh) = reopen_connection() {
                        conn = fresh;
                        resp = wake_sequence(&conn, &events_tx);
                    }
                }
                // Fold the wake transmits into health: success on the bare `ok`
                // wire string, failure on a leading `error:` (record the message).
                let ok = resp == protocol::resp_ok();
                note_health(
                    &mut health,
                    &events_tx,
                    ok,
                    resp.strip_prefix("error:").unwrap_or(&resp),
                );
                let _ = tx.send(resp);
            }
            WorkerReq::StandbyAll(tx) => {
                // Same bounded, cooldown-gated reopen+retry as WakeSequence.
                let mut resp = standby_all(&conn, &events_tx);
                if resp.starts_with("error:") && reopen_allowed(&mut last_reopen) {
                    if let Some(fresh) = reopen_connection() {
                        conn = fresh;
                        resp = standby_all(&conn, &events_tx);
                    }
                }
                let ok = resp == protocol::resp_ok();
                note_health(
                    &mut health,
                    &events_tx,
                    ok,
                    resp.strip_prefix("error:").unwrap_or(&resp),
                );
                let _ = tx.send(resp);
            }
            WorkerReq::Health(tx) => {
                // Read-only: report the last-known transmit-health without
                // touching the bus.
                let _ = tx.send(health.to_json());
            }
            WorkerReq::Test(tx) => {
                // Explicit on-demand poll probe: refresh health (emits `cec:health`
                // on a change) and reply with the current health JSON.
                let resp = run_poll_probe(&conn, &mut health, &events_tx);
                let _ = tx.send(resp);
            }
            WorkerReq::Shutdown => break,
        }
    }

    tracing::info!("cec: worker stopped");
}

// ---------------------------------------------------------------------------
// Async actor entry point.
// ---------------------------------------------------------------------------

/// Hard upper bound on a single worker round-trip. Generous enough for a legit
/// power-on / active-source plus ONE reopen+retry (libcec opens can take a few
/// seconds), but finite — so a wedged blocking libcec call (an `open()` that
/// never returns, observed on a hardware-stuck Pulse-Eight adapter) can NEVER
/// silence the actor. Past this bound the request returns a timeout error and
/// the actor's loop processes the next request. Paired with the 30s reopen
/// cooldown in `blocking_worker`, the worst case is "CEC returns timeouts until
/// a daemon restart" — the actor never wedges and the adapter isn't churned.
const WORKER_REPLY_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(15);

/// Forward one request to the blocking worker and await its reply off-thread,
/// so the libcec round-trip never blocks the reactor and no libcec handle is
/// held across the `.await`. `make` builds the `WorkerReq` from the per-request
/// reply sender.
///
/// The reply await is wrapped in [`WORKER_REPLY_TIMEOUT`] so a hung worker can
/// never wedge the async actor — on elapse the caller gets a prompt
/// `error:cec timeout (adapter busy)` and the request loop moves on. The send is
/// `try_send` so a full worker queue (64) also fails fast (`error:cec busy`)
/// instead of blocking the reactor on a `sync_channel` send.
async fn forward(
    work_tx: &std_mpsc::SyncSender<WorkerReq>,
    make: impl FnOnce(std_mpsc::SyncSender<String>) -> WorkerReq,
) -> String {
    let (tx, rx) = std_mpsc::sync_channel::<String>(1);
    match work_tx.try_send(make(tx)) {
        Ok(()) => {}
        Err(std_mpsc::TrySendError::Full(_)) => return protocol::resp_error("cec busy"),
        Err(std_mpsc::TrySendError::Disconnected(_)) => {
            return protocol::resp_error("cec worker unavailable")
        }
    }
    let reply = async move {
        tokio::task::spawn_blocking(move || {
            rx.recv()
                .unwrap_or_else(|_| protocol::resp_error("cec worker dropped reply"))
        })
        .await
        .unwrap_or_else(|_| protocol::resp_error("cec worker task failed"))
    };
    protocol::reply_with_timeout(WORKER_REPLY_TIMEOUT, "cec timeout (adapter busy)", reply).await
}

/// Run the CEC actor until `rx` is closed.
///
/// Owns a blocking libcec worker (via `tokio::task::spawn_blocking`) and
/// forwards [`CecReq`]s to it over a `std::sync::mpsc` channel, bridging the
/// async/blocking boundary. Never panics: if libcec is absent the worker drains
/// its channel replying `error:*` to each request. Push events
/// (`cec:device:<json>` / `cec:power:<json>`) are broadcast on `events_tx` for
/// each scan and power command.
///
/// `control_tx` is a clone of the input runtime's control channel: when the
/// lifecycle flag is on, the worker uses it to inject CEC remote keypresses as
/// `Control::Key` nav events (see the module docs).
pub async fn run(
    mut rx: mpsc::Receiver<CecReq>,
    events_tx: broadcast::Sender<Event>,
    control_tx: mpsc::Sender<Control>,
) -> Result<()> {
    // Bounded sync channel from the async loop to the blocking worker.
    let (work_tx, work_rx) = std_mpsc::sync_channel::<WorkerReq>(64);

    // Spawn the blocking worker on tokio's blocking pool. It owns the libcec
    // connection and (when the lifecycle flag is on) the remote-input forwarder
    // thread that injects CEC keypresses onto `control_tx`.
    let worker_handle =
        tokio::task::spawn_blocking(move || blocking_worker(work_rx, events_tx, control_tx));

    tracing::info!("cec actor started");

    while let Some(req) = rx.recv().await {
        match req {
            CecReq::Scan(reply) => {
                let _ = reply.send(forward(&work_tx, WorkerReq::Scan).await);
            }
            CecReq::Device { addr, reply } => {
                let _ =
                    reply.send(forward(&work_tx, move |tx| WorkerReq::Device { addr, tx }).await);
            }
            CecReq::PowerOn { addr, reply } => {
                let _ =
                    reply.send(forward(&work_tx, move |tx| WorkerReq::PowerOn { addr, tx }).await);
            }
            CecReq::PowerOff { addr, reply } => {
                let _ =
                    reply.send(forward(&work_tx, move |tx| WorkerReq::PowerOff { addr, tx }).await);
            }
            CecReq::ActiveSource(reply) => {
                let _ = reply.send(forward(&work_tx, WorkerReq::ActiveSource).await);
            }
            // Read-only health query and the on-demand poll probe — both forward
            // to the blocking worker (which owns the health state + libcec). Not
            // gated by the lifecycle flag: the AV Control page must read health on
            // dev/CI hosts too, and the poll probe is side-effect-free.
            CecReq::Health(reply) => {
                let _ = reply.send(forward(&work_tx, WorkerReq::Health).await);
            }
            CecReq::Test(reply) => {
                let _ = reply.send(forward(&work_tx, WorkerReq::Test).await);
            }
            // Lifecycle reqs are no-ops (reply `ok` without touching the bus)
            // when the lifecycle flag is off, so suspend/resume/SIGTERM wiring
            // never drives a CEC bus on dev/CI hosts. When on, forward to the
            // blocking worker which owns libcec.
            CecReq::WakeSequence(reply) => {
                // Read the setting OFF the reactor: it does sync file I/O and must
                // not block the async CEC actor (the startup read at ~462 runs in
                // the blocking worker; this mirrors that).
                let focus_on_wake = tokio::task::spawn_blocking(|| {
                    crate::config::cec_focus_on_wake(&crate::config::settings_path())
                })
                .await
                .unwrap_or(crate::config::CEC_FOCUS_ON_WAKE_DEFAULT);
                let resp = if crate::config::should_focus(lifecycle_enabled(), focus_on_wake) {
                    forward(&work_tx, WorkerReq::WakeSequence).await
                } else {
                    protocol::resp_ok()
                };
                let _ = reply.send(resp);
            }
            CecReq::StandbyAll(reply) => {
                let resp = if lifecycle_enabled() {
                    forward(&work_tx, WorkerReq::StandbyAll).await
                } else {
                    protocol::resp_ok()
                };
                let _ = reply.send(resp);
            }
        }
    }

    // Signal the worker to stop.
    let _ = work_tx.send(WorkerReq::Shutdown);
    let _ = worker_handle.await;
    tracing::info!("cec actor stopped");
    Ok(())
}

// ---------------------------------------------------------------------------
// Unit tests (pure helpers only — no libcec calls).
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn power_status_word_mapping() {
        assert_eq!(power_status_word(cec_rs::CecPowerStatus::On), "on");
        assert_eq!(
            power_status_word(cec_rs::CecPowerStatus::Standby),
            "standby"
        );
        assert_eq!(
            power_status_word(cec_rs::CecPowerStatus::InTransitionStandbyToOn),
            "waking"
        );
        assert_eq!(
            power_status_word(cec_rs::CecPowerStatus::InTransitionOnToStandby),
            "sleeping"
        );
        assert_eq!(
            power_status_word(cec_rs::CecPowerStatus::Unknown),
            "unknown"
        );
    }

    #[test]
    fn lifecycle_addresses_map_to_expected_logical() {
        // AVR=5 (Audiosystem), TV=0 (Tv) — the lifecycle wake/standby targets.
        assert_eq!(
            logical_from_i32(AVR_ADDR),
            Some(cec_rs::CecLogicalAddress::Audiosystem)
        );
        assert_eq!(
            logical_from_i32(TV_ADDR),
            Some(cec_rs::CecLogicalAddress::Tv)
        );
    }

    #[test]
    fn cec_key_action_maps_nav_codes() {
        use cec_rs::CecUserControlCode as K;
        // The six mapped nav codes reuse the gamepad's key_for_action names.
        assert_eq!(cec_key_action(K::Up), Some("up"));
        assert_eq!(cec_key_action(K::Down), Some("down"));
        assert_eq!(cec_key_action(K::Left), Some("left"));
        assert_eq!(cec_key_action(K::Right), Some("right"));
        assert_eq!(cec_key_action(K::Select), Some("select"));
        assert_eq!(cec_key_action(K::Exit), Some("back"));
        // Every mapped action must be a name the input runtime accepts.
        for code in [K::Up, K::Down, K::Left, K::Right, K::Select, K::Exit] {
            let action = cec_key_action(code).expect("mapped");
            assert!(
                crate::config::key_for_action(action).is_some(),
                "action {action:?} must be a valid key_for_action name"
            );
        }
        // Unmapped codes are ignored (no new vocabulary).
        assert_eq!(cec_key_action(K::RootMenu), None);
        assert_eq!(cec_key_action(K::Play), None);
        assert_eq!(cec_key_action(K::Number0), None);
    }

    #[test]
    fn logical_from_i32_bounds() {
        assert_eq!(logical_from_i32(0), Some(cec_rs::CecLogicalAddress::Tv));
        assert_eq!(
            logical_from_i32(5),
            Some(cec_rs::CecLogicalAddress::Audiosystem)
        );
        assert_eq!(
            logical_from_i32(15),
            Some(cec_rs::CecLogicalAddress::Unregistered)
        );
        assert_eq!(logical_from_i32(-1), None);
        assert_eq!(logical_from_i32(16), None);
    }
}
