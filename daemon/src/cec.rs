//! HDMI-CEC subsystem (#94): a single-owner async actor owning one persistent
//! libcec connection via `cec-rs`. It answers request/response queries over an
//! `mpsc` of [`CecReq`] and pushes `cec:*` [`Event`]s onto the shared broadcast
//! bus.
//!
//! Handles: READ via `cec-scan` / `cec-device`, ACTIONS via `cec-power-on` /
//! `cec-power-off` / `cec-active-source`. Pushes `cec:device:<json>` events
//! when devices are discovered/updated and `cec:power:<json>` when power status
//! changes.
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
    /// Lifecycle wake (start / resume-from-suspend): power on AVR (addr 5) then
    /// TV (addr 0), then claim active source. Replies `ok` / `error:*`. A no-op
    /// (still replying `ok`) when the lifecycle flag is off, so callers never
    /// drive the bus on dev/CI hosts. Not exposed as a manual IPC command.
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
/// suspend/SIGTERM) is enabled. Read from `GAME_SHELL_CEC_LIFECYCLE`: enabled
/// only when the value is exactly `"1"` or `"true"`. **Default OFF** so a plain
/// `cargo run`, CI, or a dev box never drives a real CEC bus — it's opted into
/// on the deploy host via `daemon.env`. The manual `cec-*` IPC commands are
/// unaffected by this flag.
pub fn lifecycle_enabled() -> bool {
    matches!(
        std::env::var("GAME_SHELL_CEC_LIFECYCLE").as_deref(),
        Ok("1") | Ok("true")
    )
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

/// Reply `error:libcec unavailable` to every pending request, then return.
/// Used when libcec can't be initialised so a missing/asleep adapter never
/// wedges a client (mirrors `power.rs`'s drain-on-unavailable).
fn drain_unavailable(rx: &std_mpsc::Receiver<WorkerReq>) {
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
        ));

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
            tracing::warn!(
                "cec: failed to build CecConnectionCfg ({e:?}); replying error to all requests"
            );
            drain_unavailable(&rx);
            return;
        }
    };
    let conn = match cfg.open() {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(
                "cec: failed to open libcec connection ({e:?}); replying error to all requests"
            );
            drain_unavailable(&rx);
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

    // Wake-on-open: when daemon-owned CEC lifecycle is enabled, run the wake
    // sequence once at startup (power on AVR + TV, claim active source) so the
    // shell comes up on an awake display. Off by default — see
    // `lifecycle_enabled` — so dev/CI never drives the bus from a plain start.
    if lifecycle_enabled() {
        tracing::info!("cec: lifecycle enabled — running wake sequence on open");
        let resp = wake_sequence(&conn, &events_tx);
        if resp.starts_with("error:") {
            tracing::warn!("cec: wake-on-open failed: {resp}");
        }
    }

    while let Ok(req) = rx.recv() {
        match req {
            WorkerReq::Scan(tx) => {
                // Single bus sweep, reused for the response and the push events.
                let devices = scan_devices(&conn);
                for obj in &devices {
                    let _ = events_tx.send(Event::CecDevice(obj.clone()));
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
                    Some(logical) => match conn.send_power_on_devices(logical) {
                        Ok(()) => {
                            let ps = conn.get_device_power_status(logical);
                            let payload = protocol::cec_power_json(&addr, power_status_word(ps));
                            let _ = events_tx.send(Event::CecPower(payload));
                            protocol::resp_ok()
                        }
                        Err(e) => protocol::resp_error(&format!("power-on failed: {e:?}")),
                    },
                    None => protocol::resp_error(&format!("invalid address {addr}")),
                };
                let _ = tx.send(resp);
            }
            WorkerReq::PowerOff { addr, tx } => {
                let resp = match addr.parse::<i32>().ok().and_then(logical_from_i32) {
                    Some(logical) => match conn.send_standby_devices(logical) {
                        Ok(()) => {
                            let ps = conn.get_device_power_status(logical);
                            let payload = protocol::cec_power_json(&addr, power_status_word(ps));
                            let _ = events_tx.send(Event::CecPower(payload));
                            protocol::resp_ok()
                        }
                        Err(e) => protocol::resp_error(&format!("power-off failed: {e:?}")),
                    },
                    None => protocol::resp_error(&format!("invalid address {addr}")),
                };
                let _ = tx.send(resp);
            }
            WorkerReq::ActiveSource(tx) => {
                let resp = match conn.set_active_source(cec_rs::CecDeviceType::PlaybackDevice) {
                    Ok(()) => protocol::resp_ok(),
                    Err(e) => protocol::resp_error(&format!("active-source failed: {e:?}")),
                };
                let _ = tx.send(resp);
            }
            WorkerReq::WakeSequence(tx) => {
                let _ = tx.send(wake_sequence(&conn, &events_tx));
            }
            WorkerReq::StandbyAll(tx) => {
                let _ = tx.send(standby_all(&conn, &events_tx));
            }
            WorkerReq::Shutdown => break,
        }
    }

    tracing::info!("cec: worker stopped");
}

// ---------------------------------------------------------------------------
// Async actor entry point.
// ---------------------------------------------------------------------------

/// Forward one request to the blocking worker and await its reply off-thread,
/// so the libcec round-trip never blocks the reactor and no libcec handle is
/// held across the `.await`. `make` builds the `WorkerReq` from the per-request
/// reply sender.
async fn forward(
    work_tx: &std_mpsc::SyncSender<WorkerReq>,
    make: impl FnOnce(std_mpsc::SyncSender<String>) -> WorkerReq,
) -> String {
    let (tx, rx) = std_mpsc::sync_channel::<String>(1);
    if work_tx.send(make(tx)).is_err() {
        return protocol::resp_error("cec worker unavailable");
    }
    tokio::task::spawn_blocking(move || {
        rx.recv()
            .unwrap_or_else(|_| protocol::resp_error("cec worker dropped reply"))
    })
    .await
    .unwrap_or_else(|_| protocol::resp_error("cec worker task failed"))
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
            // Lifecycle reqs are no-ops (reply `ok` without touching the bus)
            // when the lifecycle flag is off, so suspend/resume/SIGTERM wiring
            // never drives a CEC bus on dev/CI hosts. When on, forward to the
            // blocking worker which owns libcec.
            CecReq::WakeSequence(reply) => {
                let resp = if lifecycle_enabled() {
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
