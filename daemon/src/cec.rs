//! HDMI-CEC subsystem (Phase 4): a single-owner async actor owning one
//! persistent libcec connection via `cec-rs`. It answers request/response
//! queries over an `mpsc` of [`CecReq`] and pushes `cec:*` [`Event`]s onto
//! the shared broadcast bus.
//!
//! Handles: READ via `cec-scan` / `cec-device`, ACTIONS via `cec-power-on` /
//! `cec-power-off` / `cec-active-source`. Pushes `cec:device:<json>` events
//! when devices are discovered/updated and `cec:power:<json>` when power
//! status changes.
//!
//! Linux-only (libcec is a Linux/udev-based C library); `lib.rs` declares
//! this module under `#[cfg(target_os = "linux")]`. Single-owner discipline:
//! the `run` loop owns the `CecConnection` for the actor's entire lifetime;
//! nothing is held across an `.await` behind a lock. Because libcec's
//! connection type is not `Send`, the blocking libcec calls are driven inside
//! `tokio::task::spawn_blocking` closures that own the connection for the
//! duration of the call, OR (simpler for a persistent handle) the entire
//! actor runs on a dedicated blocking thread via `tokio::task::spawn_blocking`
//! with an inner loop. We use the spawn_blocking pattern: the `run` entry
//! point spawns a blocking worker thread that owns the connection; the async
//! wrapper drains `rx` and sends work to the worker via a std::sync channel.
//!
//! **cec-rs 8.0.1 API notes**: the 8.0.1 crate does not expose per-device
//! metadata queries (OSD name, physical address, vendor, device type). The
//! scan result therefore carries only `logicalAddress` and `powerStatus`.
//! `CecConnectionCfg::open(self)` consumes the config (not
//! `CecConnection::open(&cfg)`). Power commands are `send_power_on_devices` /
//! `send_standby_devices`. Active-bus enumeration is performed by probing
//! logical addresses 0–15 via `get_device_power_status` (Unknown ≡ absent).

use crate::protocol::{self, Event};
use crate::state::Reply;
use anyhow::Result;
use serde_json::json;
use std::sync::mpsc as std_mpsc;
use tokio::sync::{broadcast, mpsc};

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
    Shutdown,
}

// ---------------------------------------------------------------------------
// CEC libcec helpers (cec-rs 8.0.1 API).
// ---------------------------------------------------------------------------

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

/// Build a compact-JSON device object for a single logical address.
/// Returns `None` if the device is not present on the bus (power status
/// Unknown means the device did not respond). In cec-rs 8.0.1 only
/// `get_device_power_status` is available for per-device queries.
fn device_json(conn: &cec_rs::CecConnection, addr: cec_rs::CecLogicalAddress) -> Option<String> {
    let ps = conn.get_device_power_status(addr);
    if matches!(ps, cec_rs::CecPowerStatus::Unknown) {
        return None;
    }
    let logical_addr = addr as i32;
    Some(
        json!({
            "logicalAddress": logical_addr,
            "powerStatus": power_status_word(ps),
        })
        .to_string(),
    )
}

/// Build a compact-JSON array of all active CEC devices by probing each of
/// the 16 CEC logical addresses. cec-rs 8.0.1 does not expose a bus-active
/// enumeration call; probing via `get_device_power_status` (Unknown ≡ absent)
/// is the supported substitute.
fn scan_json(conn: &cec_rs::CecConnection) -> String {
    let mut entries = Vec::new();
    // CEC logical addresses 0..=15 (TV=0, PlaybackDevice1=4, AudioSystem=5, …)
    for raw in 0i32..=15 {
        if let Ok(addr) = cec_rs::CecLogicalAddress::try_from(raw) {
            if let Some(obj) = device_json(conn, addr) {
                entries.push(obj);
            }
        }
    }
    format!("[{}]", entries.join(","))
}

// ---------------------------------------------------------------------------
// Blocking worker: owns the CecConnection for its lifetime.
// ---------------------------------------------------------------------------

/// Run the blocking libcec worker loop.  Owns the `CecConnection`; called
/// from `tokio::task::spawn_blocking`.  Returns once a `Shutdown` message is
/// received or `rx` is closed.
///
/// The `events_tx` sender is used to broadcast `cec:device` and `cec:power`
/// push events after each scan or power-status query.
fn blocking_worker(rx: std_mpsc::Receiver<WorkerReq>, events_tx: broadcast::Sender<Event>) {
    // cec-rs 8.0.1: open() is on CecConnectionCfg (consuming self).
    let cfg = cec_rs::CecConnectionCfg {
        device_name: "game-shell".to_string(),
        device_types: cec_rs::CecDeviceTypeVec::new(vec![cec_rs::CecDeviceType::PlaybackDevice]),
        ..Default::default()
    };
    let conn = match cfg.open() {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(
                "cec: failed to open libcec connection ({e:?}); replying error to all requests"
            );
            // Drain the channel, replying error to every pending request.
            while let Ok(req) = rx.recv() {
                match req {
                    WorkerReq::Scan(tx) => {
                        let _ = tx.send(protocol::resp_error("libcec unavailable"));
                    }
                    WorkerReq::Device { tx, .. } => {
                        let _ = tx.send(protocol::resp_error("libcec unavailable"));
                    }
                    WorkerReq::PowerOn { tx, .. } => {
                        let _ = tx.send(protocol::resp_error("libcec unavailable"));
                    }
                    WorkerReq::PowerOff { tx, .. } => {
                        let _ = tx.send(protocol::resp_error("libcec unavailable"));
                    }
                    WorkerReq::ActiveSource(tx) => {
                        let _ = tx.send(protocol::resp_error("libcec unavailable"));
                    }
                    WorkerReq::Shutdown => break,
                }
            }
            return;
        }
    };

    tracing::info!("cec: libcec connection opened");

    while let Ok(req) = rx.recv() {
        match req {
            WorkerReq::Scan(tx) => {
                let result = scan_json(&conn);
                // Broadcast cec:device push events for each discovered device.
                for raw in 0i32..=15 {
                    if let Ok(addr) = cec_rs::CecLogicalAddress::try_from(raw) {
                        if let Some(obj) = device_json(&conn, addr) {
                            let _ = events_tx.send(Event::CecDevice(obj));
                        }
                    }
                }
                let _ = tx.send(result);
            }
            WorkerReq::Device { addr, tx } => {
                let logical = match addr.parse::<i32>() {
                    Ok(n) => cec_rs::CecLogicalAddress::try_from(n)
                        .unwrap_or(cec_rs::CecLogicalAddress::Unknown),
                    Err(_) => cec_rs::CecLogicalAddress::Unknown,
                };
                let resp = match device_json(&conn, logical) {
                    Some(obj) => {
                        // Emit cec:device push event for this address.
                        let _ = events_tx.send(Event::CecDevice(obj.clone()));
                        obj
                    }
                    None => protocol::resp_error(&format!("no device at address {addr}")),
                };
                let _ = tx.send(resp);
            }
            WorkerReq::PowerOn { addr, tx } => {
                let logical = match addr.parse::<i32>() {
                    Ok(n) => cec_rs::CecLogicalAddress::try_from(n)
                        .unwrap_or(cec_rs::CecLogicalAddress::Unknown),
                    Err(_) => cec_rs::CecLogicalAddress::Unknown,
                };
                // cec-rs 8.0.1: send_power_on_devices (not power_on_devices).
                let resp = match conn.send_power_on_devices(logical) {
                    Ok(()) => {
                        // Emit a cec:power push event for the targeted device.
                        let ps = conn.get_device_power_status(logical);
                        let payload = json!({
                            "addr": addr,
                            "power": power_status_word(ps),
                        })
                        .to_string();
                        let _ = events_tx.send(Event::CecPower(payload));
                        protocol::resp_ok()
                    }
                    Err(e) => protocol::resp_error(&format!("power-on failed: {e:?}")),
                };
                let _ = tx.send(resp);
            }
            WorkerReq::PowerOff { addr, tx } => {
                let logical = match addr.parse::<i32>() {
                    Ok(n) => cec_rs::CecLogicalAddress::try_from(n)
                        .unwrap_or(cec_rs::CecLogicalAddress::Unknown),
                    Err(_) => cec_rs::CecLogicalAddress::Unknown,
                };
                // cec-rs 8.0.1: send_standby_devices (not standby_devices).
                let resp = match conn.send_standby_devices(logical) {
                    Ok(()) => {
                        // Emit a cec:power push event for the targeted device.
                        let ps = conn.get_device_power_status(logical);
                        let payload = json!({
                            "addr": addr,
                            "power": power_status_word(ps),
                        })
                        .to_string();
                        let _ = events_tx.send(Event::CecPower(payload));
                        protocol::resp_ok()
                    }
                    Err(e) => protocol::resp_error(&format!("power-off failed: {e:?}")),
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
            WorkerReq::Shutdown => break,
        }
    }

    tracing::info!("cec: worker stopped");
}

// ---------------------------------------------------------------------------
// Async actor entry point.
// ---------------------------------------------------------------------------

/// Run the CEC actor until `rx` is closed.
///
/// Owns a blocking libcec worker (via `tokio::task::spawn_blocking`) and
/// forwards [`CecReq`]s to it over a `std::sync::mpsc` channel, bridging the
/// async/blocking boundary. Never panics: if libcec is absent the worker
/// drains its channel replying `error:*` to each request. Push events
/// (`cec:device:<json>` and `cec:power:<json>`) are broadcast on `events_tx`
/// for each scan and power-status query.
pub async fn run(
    mut rx: mpsc::Receiver<CecReq>,
    events_tx: broadcast::Sender<Event>,
) -> Result<()> {
    // Bounded sync channel from the async loop to the blocking worker.
    let (work_tx, work_rx) = std_mpsc::sync_channel::<WorkerReq>(64);

    // Spawn the blocking worker on tokio's blocking pool.
    let worker_handle = tokio::task::spawn_blocking(move || blocking_worker(work_rx, events_tx));

    tracing::info!("cec actor started");

    while let Some(req) = rx.recv().await {
        // For each request, open a one-shot std::sync::mpsc to get the reply
        // back from the blocking worker without holding any libcec handle
        // across the .await.
        let (reply_tx, reply_rx) = std_mpsc::sync_channel::<String>(1);

        let worker_req = match req {
            CecReq::Scan(reply) => {
                let send_result = work_tx.send(WorkerReq::Scan(reply_tx));
                if send_result.is_err() {
                    let _ = reply.send(protocol::resp_error("cec worker unavailable"));
                    continue;
                }
                // Wait for the blocking reply off-thread.
                let resp = tokio::task::spawn_blocking(move || {
                    reply_rx
                        .recv()
                        .unwrap_or_else(|_| protocol::resp_error("cec worker dropped reply"))
                })
                .await
                .unwrap_or_else(|_| protocol::resp_error("cec worker task failed"));
                let _ = reply.send(resp);
                continue;
            }
            CecReq::Device { addr, reply } => {
                let send_result = work_tx.send(WorkerReq::Device { addr, tx: reply_tx });
                if send_result.is_err() {
                    let _ = reply.send(protocol::resp_error("cec worker unavailable"));
                    continue;
                }
                let resp = tokio::task::spawn_blocking(move || {
                    reply_rx
                        .recv()
                        .unwrap_or_else(|_| protocol::resp_error("cec worker dropped reply"))
                })
                .await
                .unwrap_or_else(|_| protocol::resp_error("cec worker task failed"));
                let _ = reply.send(resp);
                continue;
            }
            CecReq::PowerOn { addr, reply } => {
                let send_result = work_tx.send(WorkerReq::PowerOn { addr, tx: reply_tx });
                if send_result.is_err() {
                    let _ = reply.send(protocol::resp_error("cec worker unavailable"));
                    continue;
                }
                let resp = tokio::task::spawn_blocking(move || {
                    reply_rx
                        .recv()
                        .unwrap_or_else(|_| protocol::resp_error("cec worker dropped reply"))
                })
                .await
                .unwrap_or_else(|_| protocol::resp_error("cec worker task failed"));
                let _ = reply.send(resp);
                continue;
            }
            CecReq::PowerOff { addr, reply } => {
                let send_result = work_tx.send(WorkerReq::PowerOff { addr, tx: reply_tx });
                if send_result.is_err() {
                    let _ = reply.send(protocol::resp_error("cec worker unavailable"));
                    continue;
                }
                let resp = tokio::task::spawn_blocking(move || {
                    reply_rx
                        .recv()
                        .unwrap_or_else(|_| protocol::resp_error("cec worker dropped reply"))
                })
                .await
                .unwrap_or_else(|_| protocol::resp_error("cec worker task failed"));
                let _ = reply.send(resp);
                continue;
            }
            CecReq::ActiveSource(reply) => {
                let send_result = work_tx.send(WorkerReq::ActiveSource(reply_tx));
                if send_result.is_err() {
                    let _ = reply.send(protocol::resp_error("cec worker unavailable"));
                    continue;
                }
                let resp = tokio::task::spawn_blocking(move || {
                    reply_rx
                        .recv()
                        .unwrap_or_else(|_| protocol::resp_error("cec worker dropped reply"))
                })
                .await
                .unwrap_or_else(|_| protocol::resp_error("cec worker task failed"));
                let _ = reply.send(resp);
                continue;
            }
        };
        // The loop uses `continue` in every arm above; `worker_req` is
        // unreachable, but the compiler needs the binding to avoid an
        // unreachable-pattern error.
        #[allow(unreachable_code)]
        let _: WorkerReq = worker_req;
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
}
