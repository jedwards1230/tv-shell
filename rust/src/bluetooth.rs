//! Bluetooth subsystem (Phase 3): a long-lived async actor owning a single
//! `bluer::Session` on the IPC runtime. It answers request/response queries over
//! an `mpsc` of [`BtReq`] (each carrying a `oneshot` reply) and pushes
//! `bt:*` [`Event`]s onto the shared broadcast bus so existing `subscribe`
//! clients receive them for free.
//!
//! This module is **Linux-only** (it speaks BlueZ over D-Bus via the `bluer`
//! crate). `main.rs` only declares it under `#[cfg(target_os = "linux")]`, so it
//! is excluded entirely from the macOS build.
//!
//! Single-owner discipline (no `Arc<Mutex>` held across `.await`): the actor's
//! `run` loop is the sole owner of the `Session`/`Adapter`, the discovery stream
//! and the per-device property streams. The IPC layer never touches BlueZ
//! directly — it only sends [`BtReq`].
//!
//! Event flow: while scanning, the actor keeps the adapter `discover_devices`
//! stream alive (dropping it stops discovery in BlueZ) and, for every device it
//! learns about, subscribes to that device's `events()` property-change stream.
//! Adapter property changes surface power/scanning toggles; device property
//! changes surface connection/pair/trust/name/rssi updates. All are translated
//! into `bt:device` / `bt:device-removed` / `bt:powered` / `bt:scanning` events.

use crate::protocol::{self, Event};
use crate::state::Reply;
use anyhow::{anyhow, Result};
use bluer::{Adapter, AdapterEvent, AdapterProperty, Address, Device, DeviceEvent, Session};
use futures::stream::{SelectAll, Stream, StreamExt};
use std::collections::HashSet;
use std::pin::Pin;
use std::str::FromStr;
use tokio::sync::{broadcast, mpsc};

/// Requests from the IPC server to the Bluetooth actor. Each carries a
/// `oneshot` reply into which the actor sends a fully-formatted wire string
/// (`resp_ok`, `resp_error`, `resp_bt_power`, or a compact-JSON body).
#[derive(Debug)]
pub enum BtReq {
    /// `bt-power-status` -> `bt:on` / `bt:off` / `error:*`.
    PowerStatus(Reply),
    /// `bt-power-on` -> `ok` / `error:*`.
    PowerOn(Reply),
    /// `bt-power-off` -> `ok` / `error:*`.
    PowerOff(Reply),
    /// `bt-scan-on` -> `ok`; results arrive later as `bt:device` events.
    ScanOn(Reply),
    /// `bt-scan-off` -> `ok`.
    ScanOff(Reply),
    /// `bt-list` -> compact JSON array of `{mac,name,paired,connected,trusted,rssi}`.
    List(Reply),
    /// `bt-connect <mac>` -> `ok` / `error:*`.
    Connect { mac: String, reply: Reply },
    /// `bt-disconnect <mac>` -> `ok` / `error:*`.
    Disconnect { mac: String, reply: Reply },
    /// `bt-pair <mac>` -> `ok` / `error:*` (just-works via BlueZ default agent).
    Pair { mac: String, reply: Reply },
    /// `bt-trust <mac>` -> `ok` / `error:*`.
    Trust { mac: String, reply: Reply },
}

/// A per-device property-change stream tagged with its source address.
///
/// `Device::events()` yields bare [`DeviceEvent`]s with no indication of which
/// device they belong to, so we `.map()` each stream to pair every event with
/// its [`Address`] and box it (each `.map()` closure is a distinct type, so a
/// homogeneous `SelectAll` needs the trait-object form).
type TaggedDeviceEvents = Pin<Box<dyn Stream<Item = (Address, DeviceEvent)> + Send>>;

/// Run the Bluetooth actor until `rx` is closed.
///
/// Owns one `bluer::Session`/`Adapter`, services [`BtReq`]s, and pushes
/// `bt:*` events onto `events_tx`. If BlueZ is absent or the session fails to
/// open, it logs and falls back to a degraded loop that answers every request
/// with `error:*` (never panics, never crashes the daemon).
pub async fn run(rx: mpsc::Receiver<BtReq>, events_tx: broadcast::Sender<Event>) -> Result<()> {
    match open_adapter().await {
        Ok((session, adapter)) => {
            tracing::info!("bluetooth actor started (adapter {})", adapter.name());
            run_with_adapter(rx, events_tx, session, adapter).await
        }
        Err(e) => {
            tracing::warn!("bluetooth unavailable ({e}); serving degraded replies");
            run_degraded(rx).await
        }
    }
}

/// Open a `Session` and resolve an adapter, preferring the default and falling
/// back to the first by name. Returns both so the caller keeps the `Session`
/// (which owns the D-Bus connection) alive for the adapter's lifetime.
async fn open_adapter() -> Result<(Session, Adapter)> {
    let session = Session::new().await?;
    let adapter = match session.default_adapter().await {
        Ok(a) => a,
        Err(_) => {
            // No default adapter — try the first by name before giving up.
            let names = session.adapter_names().await?;
            let first = names
                .into_iter()
                .next()
                .ok_or_else(|| anyhow!("no bluetooth adapter present"))?;
            session.adapter(&first)?
        }
    };
    Ok((session, adapter))
}

/// Degraded loop used when no adapter is reachable: drains `rx`, answering every
/// request with an `error:*` so the wire contract still completes.
async fn run_degraded(mut rx: mpsc::Receiver<BtReq>) -> Result<()> {
    while let Some(req) = rx.recv().await {
        reply_of(req)
            .send(protocol::resp_error("bluetooth unavailable"))
            .ok();
    }
    tracing::info!("bluetooth actor stopped");
    Ok(())
}

/// Extract the `oneshot` reply channel from any request (for uniform error
/// handling on degraded paths).
fn reply_of(req: BtReq) -> Reply {
    match req {
        BtReq::PowerStatus(r)
        | BtReq::PowerOn(r)
        | BtReq::PowerOff(r)
        | BtReq::ScanOn(r)
        | BtReq::ScanOff(r)
        | BtReq::List(r) => r,
        BtReq::Connect { reply, .. }
        | BtReq::Disconnect { reply, .. }
        | BtReq::Pair { reply, .. }
        | BtReq::Trust { reply, .. } => reply,
    }
}

/// The live event loop, owning the adapter plus the (optional) discovery stream
/// and the set of per-device property streams.
async fn run_with_adapter(
    mut rx: mpsc::Receiver<BtReq>,
    events_tx: broadcast::Sender<Event>,
    // `session` owns the D-Bus connection that `adapter`/devices depend on; it
    // must outlive them, so keep it bound for the whole loop.
    _session: Session,
    adapter: Adapter,
) -> Result<()> {
    // The discovery stream: `Some` only while scanning. Dropping it tells BlueZ
    // to stop discovery, so its lifetime *is* the scan state.
    let mut discovery: Option<Pin<Box<dyn Stream<Item = AdapterEvent> + Send>>> = None;
    // Per-device property streams, tagged with their address.
    let mut device_events: SelectAll<TaggedDeviceEvents> = SelectAll::new();
    // Addresses we already subscribed to, to avoid duplicate device streams.
    let mut subscribed: HashSet<Address> = HashSet::new();

    loop {
        tokio::select! {
            // --- IPC requests ---
            maybe_req = rx.recv() => {
                let Some(req) = maybe_req else { break };
                handle_req(
                    req,
                    &adapter,
                    &events_tx,
                    &mut discovery,
                    &mut device_events,
                    &mut subscribed,
                )
                .await;
            }

            // --- Adapter discovery events (only polled while scanning) ---
            // `poll_next_discovery` returns `Pending` forever when `discovery`
            // is `None`, so this branch is inert when not scanning.
            ev = poll_next_discovery(&mut discovery) => {
                match ev {
                    Some(AdapterEvent::DeviceAdded(addr)) => {
                        subscribe_device(
                            &adapter,
                            addr,
                            &events_tx,
                            &mut device_events,
                            &mut subscribed,
                        )
                        .await;
                    }
                    Some(AdapterEvent::DeviceRemoved(addr)) => {
                        subscribed.remove(&addr);
                        let _ = events_tx.send(Event::BtDeviceRemoved(addr.to_string()));
                    }
                    Some(AdapterEvent::PropertyChanged(AdapterProperty::Powered(on))) => {
                        let _ = events_tx.send(Event::BtPowered(on));
                    }
                    Some(AdapterEvent::PropertyChanged(AdapterProperty::Discovering(on))) => {
                        let _ = events_tx.send(Event::BtScanning(on));
                    }
                    Some(AdapterEvent::PropertyChanged(_)) => {}
                    // Stream ended unexpectedly: stop polling it.
                    None => discovery = None,
                }
            }

            // --- Per-device property changes (only polled when non-empty) ---
            // `SelectAll` yields `None` immediately when empty, which would
            // busy-loop, so gate this branch on `!is_empty()`.
            Some((addr, DeviceEvent::PropertyChanged(_))) = device_events.next(),
                if !device_events.is_empty() =>
            {
                // Any property change re-emits the device's current snapshot so
                // the QML reflects connect/pair/trust/name/rssi changes.
                if let Ok(device) = adapter.device(addr) {
                    if let Some(json) = device_json(&device).await {
                        let _ = events_tx.send(Event::BtDevice(json));
                    }
                }
            }
        }
    }

    tracing::info!("bluetooth actor stopped");
    Ok(())
}

/// Poll the (optional) discovery stream. When `discovery` is `None` we never
/// resolve, leaving the `select!` branch inert until a scan starts.
async fn poll_next_discovery(
    discovery: &mut Option<Pin<Box<dyn Stream<Item = AdapterEvent> + Send>>>,
) -> Option<AdapterEvent> {
    match discovery {
        Some(stream) => stream.next().await,
        None => std::future::pending().await,
    }
}

/// Dispatch a single request against the live adapter.
async fn handle_req(
    req: BtReq,
    adapter: &Adapter,
    events_tx: &broadcast::Sender<Event>,
    discovery: &mut Option<Pin<Box<dyn Stream<Item = AdapterEvent> + Send>>>,
    device_events: &mut SelectAll<TaggedDeviceEvents>,
    subscribed: &mut HashSet<Address>,
) {
    match req {
        BtReq::PowerStatus(reply) => {
            let resp = match adapter.is_powered().await {
                Ok(on) => protocol::resp_bt_power(on),
                Err(e) => protocol::resp_error(&format!("power status: {e}")),
            };
            let _ = reply.send(resp);
        }
        BtReq::PowerOn(reply) => {
            let _ = reply.send(map_unit(adapter.set_powered(true).await, "power on"));
        }
        BtReq::PowerOff(reply) => {
            let _ = reply.send(map_unit(adapter.set_powered(false).await, "power off"));
        }
        BtReq::ScanOn(reply) => {
            if discovery.is_some() {
                // Already scanning — idempotent ok.
                let _ = reply.send(protocol::resp_ok());
                return;
            }
            match adapter.discover_devices().await {
                Ok(stream) => {
                    *discovery = Some(Box::pin(stream));
                    let _ = events_tx.send(Event::BtScanning(true));
                    let _ = reply.send(protocol::resp_ok());
                }
                Err(e) => {
                    let _ = reply.send(protocol::resp_error(&format!("scan on: {e}")));
                }
            }
        }
        BtReq::ScanOff(reply) => {
            // Dropping the stream stops discovery in BlueZ.
            let was_scanning = discovery.take().is_some();
            device_events.clear();
            subscribed.clear();
            if was_scanning {
                let _ = events_tx.send(Event::BtScanning(false));
            }
            let _ = reply.send(protocol::resp_ok());
        }
        BtReq::List(reply) => {
            let _ = reply.send(list_devices_json(adapter).await);
        }
        BtReq::Connect { mac, reply } => {
            let resp = with_device(adapter, &mac, |d| async move {
                map_unit(d.connect().await, "connect")
            })
            .await;
            let _ = reply.send(resp);
        }
        BtReq::Disconnect { mac, reply } => {
            let resp = with_device(adapter, &mac, |d| async move {
                map_unit(d.disconnect().await, "disconnect")
            })
            .await;
            let _ = reply.send(resp);
        }
        BtReq::Pair { mac, reply } => {
            let resp = with_device(adapter, &mac, |d| async move {
                map_unit(d.pair().await, "pair")
            })
            .await;
            let _ = reply.send(resp);
        }
        BtReq::Trust { mac, reply } => {
            let resp = with_device(adapter, &mac, |d| async move {
                map_unit(d.set_trusted(true).await, "trust")
            })
            .await;
            let _ = reply.send(resp);
        }
    }
}

/// Subscribe to a newly-discovered device's property stream (once) and emit its
/// current snapshot as a `bt:device` event.
async fn subscribe_device(
    adapter: &Adapter,
    addr: Address,
    events_tx: &broadcast::Sender<Event>,
    device_events: &mut SelectAll<TaggedDeviceEvents>,
    subscribed: &mut HashSet<Address>,
) {
    let Ok(device) = adapter.device(addr) else {
        return;
    };
    if let Some(json) = device_json(&device).await {
        let _ = events_tx.send(Event::BtDevice(json));
    }
    if subscribed.insert(addr) {
        match device.events().await {
            Ok(stream) => {
                let tagged = stream.map(move |ev| (addr, ev));
                device_events.push(Box::pin(tagged));
            }
            Err(e) => {
                tracing::debug!("device {addr} events stream failed: {e}");
                subscribed.remove(&addr);
            }
        }
    }
}

/// Resolve a MAC string to a [`Device`] and run `f`, mapping a parse failure to
/// an `error:*`. Used by connect/disconnect/pair/trust.
async fn with_device<F, Fut>(adapter: &Adapter, mac: &str, f: F) -> String
where
    F: FnOnce(Device) -> Fut,
    Fut: std::future::Future<Output = String>,
{
    let addr = match Address::from_str(mac) {
        Ok(a) => a,
        Err(_) => return protocol::resp_error(&format!("invalid mac '{mac}'")),
    };
    match adapter.device(addr) {
        Ok(device) => f(device).await,
        Err(e) => protocol::resp_error(&format!("device {mac}: {e}")),
    }
}

/// Map a BlueZ `Result<()>` to the wire `ok` / `error:<ctx>: <e>`.
fn map_unit(res: bluer::Result<()>, ctx: &str) -> String {
    match res {
        Ok(()) => protocol::resp_ok(),
        Err(e) => protocol::resp_error(&format!("{ctx}: {e}")),
    }
}

/// Build the compact-JSON array body for `bt-list` from the adapter's known
/// devices. Devices that error while reading props are skipped rather than
/// failing the whole list.
async fn list_devices_json(adapter: &Adapter) -> String {
    let addrs = match adapter.device_addresses().await {
        Ok(a) => a,
        Err(e) => return protocol::resp_error(&format!("list: {e}")),
    };
    let mut items: Vec<serde_json::Value> = Vec::with_capacity(addrs.len());
    for addr in addrs {
        if let Ok(device) = adapter.device(addr) {
            items.push(device_value(&device).await);
        }
    }
    serde_json::to_string(&serde_json::Value::Array(items)).unwrap_or_else(|_| "[]".to_string())
}

/// A single device's compact-JSON object as a string (`bt:device` payload).
async fn device_json(device: &Device) -> Option<String> {
    serde_json::to_string(&device_value(device).await).ok()
}

/// Build the `{mac,name,paired,connected,trusted,rssi}` JSON value for a device.
/// Missing/erroring properties degrade to sensible defaults (name null, bools
/// false, rssi null) rather than dropping the device.
async fn device_value(device: &Device) -> serde_json::Value {
    let mac = device.address().to_string();
    // `name()` is the BlueZ remote name; fall back to alias if absent.
    let name = match device.name().await {
        Ok(Some(n)) => Some(n),
        _ => device.alias().await.ok().filter(|s| !s.is_empty()),
    };
    let paired = device.is_paired().await.unwrap_or(false);
    let connected = device.is_connected().await.unwrap_or(false);
    let trusted = device.is_trusted().await.unwrap_or(false);
    let rssi = device.rssi().await.ok().flatten();

    serde_json::json!({
        "mac": mac,
        "name": name,
        "paired": paired,
        "connected": connected,
        "trusted": trusted,
        "rssi": rssi,
    })
}
