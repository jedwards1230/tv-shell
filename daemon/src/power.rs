//! Power / idle subsystem (Phase 3): a long-lived async actor owning a single
//! `zbus::Connection` to the system bus, talking to logind
//! (`org.freedesktop.login1`) and UPower (`org.freedesktop.UPower`). It answers
//! request/response queries over an `mpsc` of [`PowerReq`] and pushes
//! `power:*` [`Event`]s onto the shared broadcast bus.
//!
//! v1 scope: logind `CanSuspend`/`Suspend` and UPower battery state. Reboot and
//! poweroff deliberately stay `systemctl` shell-outs in the QML. Idle inhibitors
//! are deferred (not needed by the current pages).
//!
//! No-battery handling: game-client-1 is a desktop. If UPower is unreachable, or
//! its display device reports `IsPresent=false`, or the device isn't a battery,
//! `power-battery` returns `{"present":false}` and no `power:battery` events are
//! emitted. An absent UPower service must never error the daemon.
//!
//! Linux-only (system D-Bus); `main.rs` declares it under
//! `#[cfg(target_os = "linux")]`. Single-owner discipline: the `run` loop owns
//! the `Connection` and any property streams. The UPower property-change streams
//! are folded into the actor's `select!` loop, so no shared state ever crosses
//! an `.await`.

#[cfg(feature = "cec")]
use crate::cec::CecReq;
use crate::protocol::{self, Event};
use crate::state::Reply;
use anyhow::Result;
use futures::StreamExt;
use serde_json::json;
use tokio::sync::{broadcast, mpsc};
use zbus::Connection;

/// Requests from the IPC server to the Power actor. Each carries a `oneshot`
/// reply with a fully-formatted wire string.
#[derive(Debug)]
pub enum PowerReq {
    /// `power-can-suspend` -> `yes` / `no` / `error:*`.
    CanSuspend(Reply),
    /// `power-suspend` -> `ok` / `error:*` (logind `Suspend(false)`).
    Suspend(Reply),
    /// `power-battery` -> compact JSON object (`{"present":false}` on a desktop,
    /// or `{"present":true,"percentage":..,"state":..,"onBattery":..,"icon":..}`).
    Battery(Reply),
}

// ---------------------------------------------------------------------------
// D-Bus proxies (logind + UPower).
// ---------------------------------------------------------------------------

/// org.freedesktop.login1.Manager at /org/freedesktop/login1.
#[zbus::proxy(
    interface = "org.freedesktop.login1.Manager",
    default_service = "org.freedesktop.login1",
    default_path = "/org/freedesktop/login1"
)]
trait LogindManager {
    /// `CanSuspend() -> s` ("yes" / "no" / "challenge" / "na").
    fn can_suspend(&self) -> zbus::Result<String>;

    /// `Suspend(interactive: b)`. We pass `false` (non-interactive).
    fn suspend(&self, interactive: bool) -> zbus::Result<()>;

    /// `PrepareForSleep(start: b)`: emitted by logind around a system sleep
    /// transition. `start == true` fires just BEFORE the system suspends;
    /// `start == false` fires just AFTER it resumes. Drives the daemon-owned CEC
    /// lifecycle (standby on suspend, wake on resume).
    #[zbus(signal)]
    fn prepare_for_sleep(&self, start: bool) -> zbus::Result<()>;
}

/// org.freedesktop.UPower at /org/freedesktop/UPower.
#[zbus::proxy(
    interface = "org.freedesktop.UPower",
    default_service = "org.freedesktop.UPower",
    default_path = "/org/freedesktop/UPower"
)]
trait UPower {
    /// `GetDisplayDevice() -> o`: the composite display device object path.
    fn get_display_device(&self) -> zbus::Result<zbus::zvariant::OwnedObjectPath>;

    /// `OnBattery: b` — whether the system is running on battery.
    #[zbus(property)]
    fn on_battery(&self) -> zbus::Result<bool>;
}

/// org.freedesktop.UPower.Device on a per-device object path.
#[zbus::proxy(
    interface = "org.freedesktop.UPower.Device",
    default_service = "org.freedesktop.UPower"
)]
trait UPowerDevice {
    /// `IsPresent: b`.
    #[zbus(property)]
    fn is_present(&self) -> zbus::Result<bool>;

    /// `Percentage: d` (0..100).
    #[zbus(property)]
    fn percentage(&self) -> zbus::Result<f64>;

    /// `State: u` (0 unknown, 1 charging, 2 discharging, 3 empty,
    /// 4 fully charged, 5 pending-charge, 6 pending-discharge).
    #[zbus(property)]
    fn state(&self) -> zbus::Result<u32>;

    /// `Type: u` (2 = battery). Explicit `name` since the Rust identifier carries
    /// a trailing underscore to dodge the keyword.
    #[zbus(property, name = "Type")]
    fn type_(&self) -> zbus::Result<u32>;

    /// `IconName: s` (Icon Naming Spec).
    #[zbus(property)]
    fn icon_name(&self) -> zbus::Result<String>;
}

/// UPower `Type` value for a battery.
const UPOWER_TYPE_BATTERY: u32 = 2;

/// Map a UPower `State` u32 to a UI-friendly lowercase word.
fn battery_state_word(state: u32) -> &'static str {
    match state {
        1 => "charging",
        2 => "discharging",
        3 => "empty",
        4 => "full",
        5 => "pending-charge",
        6 => "pending-discharge",
        _ => "unknown",
    }
}

/// The compact-JSON body for an absent battery: `{"present":false}`.
fn battery_absent_json() -> String {
    json!({ "present": false }).to_string()
}

/// Build the compact-JSON body for a present battery. `percentage` is rounded to
/// an integer (UIs render whole percents); `state` is the lowercase word.
fn battery_present_json(percentage: f64, state: u32, on_battery: bool, icon: &str) -> String {
    json!({
        "present": true,
        "percentage": percentage.round() as i64,
        "state": battery_state_word(state),
        "onBattery": on_battery,
        "icon": icon,
    })
    .to_string()
}

/// A snapshot of the UPower display device, or `None` if there's no real
/// battery (no UPower, no display device, not present, or not a battery type).
struct BatterySnapshot {
    percentage: f64,
    state: u32,
    on_battery: bool,
    icon: String,
}

impl BatterySnapshot {
    fn to_json(&self) -> String {
        battery_present_json(self.percentage, self.state, self.on_battery, &self.icon)
    }
}

/// Read the current battery snapshot from UPower, degrading gracefully. Returns
/// `Ok(None)` (never an error) when there is no real battery — an absent UPower
/// service, an absent/non-battery display device, or a read failure all map to
/// "no battery present", so the daemon never errors on a desktop.
async fn read_battery(conn: &Connection) -> Option<BatterySnapshot> {
    let upower = UPowerProxy::new(conn).await.ok()?;
    let device_path = upower.get_display_device().await.ok()?;
    let device = UPowerDeviceProxy::builder(conn)
        .path(device_path)
        .ok()?
        .build()
        .await
        .ok()?;

    // Only a present battery-type device counts. Any read failure -> no battery.
    if !device.is_present().await.ok()? {
        return None;
    }
    if device.type_().await.ok()? != UPOWER_TYPE_BATTERY {
        return None;
    }

    let percentage = device.percentage().await.ok()?;
    let state = device.state().await.ok()?;
    let icon = device.icon_name().await.unwrap_or_default();
    // `OnBattery` lives on the manager, not the device; treat a read failure as
    // "not on battery" rather than dropping the whole snapshot.
    let on_battery = upower.on_battery().await.unwrap_or(false);

    Some(BatterySnapshot {
        percentage,
        state,
        on_battery,
        icon,
    })
}

/// Reply to `power-can-suspend`: maps logind `CanSuspend()` to `yes`/`no`.
/// logind returns "yes"/"challenge" when suspend is allowed (challenge = allowed
/// after authentication), "no"/"na" otherwise. A bus error degrades to `no`
/// rather than erroring (the QML only needs a boolean to enable/disable a
/// button).
async fn handle_can_suspend(logind: Option<&LogindManagerProxy<'_>>) -> String {
    let Some(logind) = logind else {
        return protocol::resp_yes_no(false);
    };
    match logind.can_suspend().await {
        Ok(s) => protocol::resp_yes_no(matches!(s.as_str(), "yes" | "challenge")),
        Err(e) => {
            tracing::warn!("logind CanSuspend failed: {e}");
            protocol::resp_yes_no(false)
        }
    }
}

/// Reply to `power-suspend`: invokes logind `Suspend(false)`. A bus error is
/// reported as `error:*`.
async fn handle_suspend(logind: Option<&LogindManagerProxy<'_>>) -> String {
    let Some(logind) = logind else {
        return protocol::resp_error("logind unavailable");
    };
    match logind.suspend(false).await {
        Ok(()) => protocol::resp_ok(),
        Err(e) => {
            tracing::warn!("logind Suspend failed: {e}");
            protocol::resp_error(&format!("suspend failed: {e}"))
        }
    }
}

/// Run the Power actor until `rx` is closed.
///
/// Owns one `zbus::Connection`, services [`PowerReq`]s against logind + UPower,
/// and pushes `power:battery` events onto `events_tx` only when a real battery
/// is present. Logs and retries (never panics) if logind/UPower are absent;
/// query failures reply with `error:*` (except battery, which degrades to
/// `{"present":false}`).
pub async fn run(
    mut rx: mpsc::Receiver<PowerReq>,
    events_tx: broadcast::Sender<Event>,
    // Optional CEC actor handle for the daemon-owned lifecycle (#94 follow-up):
    // on logind `PrepareForSleep` this forwards `StandbyAll` (suspend) /
    // `WakeSequence` (resume). Cloned from the CEC channel in `main.rs` and only
    // present under `--features cec`; the CEC actor itself no-ops these unless
    // `GAME_SHELL_CEC_LIFECYCLE` is enabled, so this is inert on dev/CI.
    #[cfg(feature = "cec")] cec_tx: Option<mpsc::Sender<CecReq>>,
) -> Result<()> {
    // A missing system bus is fatal for this actor (nothing it can do), but it
    // must not panic the daemon: log and exit cleanly so `main.rs` just warns.
    let conn = match Connection::system().await {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!("power: no system D-Bus ({e}); power commands degrade to errors");
            // Drain requests with errors so callers don't hang.
            return drain_unavailable(rx).await;
        }
    };

    // logind proxy is optional: if it can't be built we degrade can-suspend to
    // `no` and suspend to an error, but the actor keeps running for battery.
    let logind = match LogindManagerProxy::new(&conn).await {
        Ok(p) => Some(p),
        Err(e) => {
            tracing::warn!("power: logind proxy unavailable ({e}); suspend disabled");
            None
        }
    };

    tracing::info!("power actor started (logind + UPower)");

    // Subscribe to UPower property changes only when a real battery is present.
    // We build the device proxy and its property streams up front; on a desktop
    // with no battery these are simply absent and no `power:battery` events flow.
    let mut battery_streams = BatteryStreams::new(&conn).await;
    match &battery_streams {
        Some(_) => tracing::info!("power: battery present; streaming power:battery changes"),
        None => tracing::info!("power: no battery present (desktop); no power:battery events"),
    }

    // logind `PrepareForSleep` -> CEC lifecycle (suspend = standby, resume =
    // wake). Runs as its OWN task, not a `select!` arm: `tokio::select!` cannot
    // take a `#[cfg(...)]` branch (the macro rejects the attribute even on a
    // no-`cec` build). The task owns a cloned system-bus connection + its own
    // logind proxy. Inert unless GAME_SHELL_CEC_LIFECYCLE is set (the CEC actor
    // no-ops the reqs). On a default (no-`cec`) build the `cec_tx` param and
    // this whole block compile out.
    #[cfg(feature = "cec")]
    if let Some(cec_tx) = cec_tx.clone() {
        let conn = conn.clone();
        tokio::spawn(async move {
            let proxy = match LogindManagerProxy::new(&conn).await {
                Ok(p) => p,
                Err(e) => {
                    tracing::warn!("power: PrepareForSleep proxy unavailable ({e}); CEC suspend/resume lifecycle disabled");
                    return;
                }
            };
            let mut sleep_stream = match proxy.receive_prepare_for_sleep().await {
                Ok(stream) => stream,
                Err(e) => {
                    tracing::warn!("power: PrepareForSleep subscription failed ({e}); CEC suspend/resume lifecycle disabled");
                    return;
                }
            };
            while let Some(signal) = sleep_stream.next().await {
                let start = match signal.args() {
                    Ok(args) => args.start,
                    Err(e) => {
                        tracing::warn!("power: malformed PrepareForSleep signal: {e}");
                        continue;
                    }
                };
                let req_label = if start {
                    "standby (suspend)"
                } else {
                    "wake (resume)"
                };
                tracing::info!("power: PrepareForSleep start={start} -> CEC {req_label}");
                let (tx, rx) = tokio::sync::oneshot::channel();
                let req = if start {
                    CecReq::StandbyAll(tx)
                } else {
                    CecReq::WakeSequence(tx)
                };
                if cec_tx.send(req).await.is_err() {
                    tracing::warn!("power: CEC actor gone; stopping PrepareForSleep listener");
                    break;
                }
                if let Ok(resp) = rx.await {
                    if resp.starts_with("error:") {
                        tracing::warn!("power: CEC lifecycle {req_label} failed: {resp}");
                    }
                }
            }
        });
    }

    loop {
        // `next_battery_change` resolves only when a battery is present;
        // otherwise it parks forever, so the `select!` just services requests.
        tokio::select! {
            maybe_req = rx.recv() => {
                let Some(req) = maybe_req else { break };
                match req {
                    PowerReq::CanSuspend(reply) => {
                        let _ = reply.send(handle_can_suspend(logind.as_ref()).await);
                    }
                    PowerReq::Suspend(reply) => {
                        let _ = reply.send(handle_suspend(logind.as_ref()).await);
                    }
                    PowerReq::Battery(reply) => {
                        let body = match read_battery(&conn).await {
                            Some(snap) => snap.to_json(),
                            None => battery_absent_json(),
                        };
                        let _ = reply.send(body);
                    }
                }
            }
            // A property changed on the battery device — re-read and broadcast.
            () = next_battery_change(&mut battery_streams) => {
                if let Some(snap) = read_battery(&conn).await {
                    let _ = events_tx.send(Event::PowerBattery(snap.to_json()));
                }
                // If the battery vanished mid-session we simply stop emitting;
                // a fresh `power-battery` query will report `{"present":false}`.
            }
        }
    }

    tracing::info!("power actor stopped");
    Ok(())
}

/// Drain pending requests with degraded replies when there's no system bus.
/// `can-suspend` -> `no`, `suspend` -> error, `battery` -> `{"present":false}`.
async fn drain_unavailable(mut rx: mpsc::Receiver<PowerReq>) -> Result<()> {
    while let Some(req) = rx.recv().await {
        let reply_body = match req {
            PowerReq::CanSuspend(reply) => {
                let _ = reply.send(protocol::resp_yes_no(false));
                continue;
            }
            PowerReq::Suspend(reply) => {
                let _ = reply.send(protocol::resp_error("system D-Bus unavailable"));
                continue;
            }
            PowerReq::Battery(reply) => reply,
        };
        let _ = reply_body.send(battery_absent_json());
    }
    Ok(())
}

/// Property-change streams for the UPower battery device. Holds the device proxy
/// alive (its property streams borrow it) and merges the relevant change streams
/// into one. When there's no battery this is `None`, and `next_change()` parks
/// forever so the actor's `select!` only services requests.
struct BatteryStreams<'a> {
    // Keep the device proxy alive so the streams it spawned stay valid.
    _device: UPowerDeviceProxy<'a>,
    // The three `receive_*_changed()` streams have different item types
    // (`PropertyStream<'_, f64/u32/String>`); after `.map(|_| ())` they're
    // distinct `Map` types, so we box each to unify them under `SelectAll`.
    changes: futures::stream::SelectAll<futures::stream::BoxStream<'a, ()>>,
}

impl<'a> BatteryStreams<'a> {
    /// Build the merged change stream, or `None` if there's no present battery.
    async fn new(conn: &'a Connection) -> Option<Self> {
        let upower = UPowerProxy::new(conn).await.ok()?;
        let device_path = upower.get_display_device().await.ok()?;
        let device = UPowerDeviceProxy::builder(conn)
            .path(device_path)
            .ok()?
            .build()
            .await
            .ok()?;

        if !device.is_present().await.ok()? {
            return None;
        }
        if device.type_().await.ok()? != UPOWER_TYPE_BATTERY {
            return None;
        }

        // `receive_*_changed()` yields a `PropertyStream<'_, T>`; map each to
        // `()` and box so they share one stream type, then merge with `SelectAll`.
        let mut changes = futures::stream::SelectAll::new();
        changes.push(
            device
                .receive_percentage_changed()
                .await
                .map(|_| ())
                .boxed(),
        );
        changes.push(device.receive_state_changed().await.map(|_| ()).boxed());
        changes.push(device.receive_icon_name_changed().await.map(|_| ()).boxed());

        Some(Self {
            _device: device,
            changes,
        })
    }

    /// Resolve when any watched property changes. If there's no battery, this
    /// future never resolves (so the `select!` arm is effectively disabled).
    async fn next_change(&mut self) {
        // `SelectAll::next()` yields `Some(())` on each change; loop forever on
        // `None` (all streams ended) to keep the arm pending rather than hot.
        loop {
            match self.changes.next().await {
                Some(()) => return,
                None => std::future::pending::<()>().await,
            }
        }
    }
}

/// Resolve when the battery (if any) changes a watched property. When there's no
/// battery (`None`), this parks forever so the actor's `select!` arm stays
/// pending and only requests are serviced.
async fn next_battery_change(streams: &mut Option<BatteryStreams<'_>>) {
    match streams {
        Some(s) => s.next_change().await,
        None => std::future::pending::<()>().await,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_words() {
        assert_eq!(battery_state_word(0), "unknown");
        assert_eq!(battery_state_word(1), "charging");
        assert_eq!(battery_state_word(2), "discharging");
        assert_eq!(battery_state_word(3), "empty");
        assert_eq!(battery_state_word(4), "full");
        assert_eq!(battery_state_word(5), "pending-charge");
        assert_eq!(battery_state_word(6), "pending-discharge");
        assert_eq!(battery_state_word(99), "unknown");
    }

    #[test]
    fn absent_json_shape() {
        assert_eq!(battery_absent_json(), r#"{"present":false}"#);
    }

    #[test]
    fn present_json_shape() {
        // preserve_order keeps insertion order: present,percentage,state,onBattery,icon.
        assert_eq!(
            battery_present_json(73.6, 2, true, "battery-good-symbolic"),
            r#"{"present":true,"percentage":74,"state":"discharging","onBattery":true,"icon":"battery-good-symbolic"}"#
        );
        // Rounds to nearest integer; full + not-on-battery.
        assert_eq!(
            battery_present_json(99.4, 4, false, "battery-full-charged-symbolic"),
            r#"{"present":true,"percentage":99,"state":"full","onBattery":false,"icon":"battery-full-charged-symbolic"}"#
        );
    }
}
