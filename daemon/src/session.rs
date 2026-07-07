//! logind session-active watcher.
//!
//! The daemon holds an exclusive `EVIOCGRAB` on the gamepad so no input leaks to
//! the compositor. But the tv-shell session can be left *running in the
//! background* — e.g. the user VT-switches to Plasma/Bigscreen without logging
//! tv-shell out. While backgrounded, a held grab starves the foreground
//! session of the controller (the classic "tv-shell -> plasma and my pad is
//! dead" bug).
//!
//! This actor watches our own logind session's `Active` property and tells the
//! input runtime to release every pad's grab while we're inactive and re-take it
//! when we return. Mirrors the other zbus actors (`network.rs`): owns its own
//! system-bus connection, logs and exits cleanly if logind is absent, and never
//! panics the daemon.

use crate::state::Control;
use anyhow::{Context, Result};
use futures::stream::StreamExt;
use tokio::sync::mpsc;
use tracing::info;
use zbus::Connection;

/// logind manager — resolves our PID to its session object path.
#[zbus::proxy(
    interface = "org.freedesktop.login1.Manager",
    default_service = "org.freedesktop.login1",
    default_path = "/org/freedesktop/login1"
)]
trait LogindManager {
    /// `GetSessionByPID` — the D-Bus name keeps the upper-case `PID`, which the
    /// snake_case->PascalCase default would mangle to `GetSessionByPid`.
    #[zbus(name = "GetSessionByPID")]
    fn get_session_by_pid(&self, pid: u32) -> zbus::Result<zbus::zvariant::OwnedObjectPath>;
}

/// A single logind session — we only care about whether it is the foreground
/// (active) session on its seat.
#[zbus::proxy(
    interface = "org.freedesktop.login1.Session",
    default_service = "org.freedesktop.login1"
)]
trait LogindSession {
    #[zbus(property)]
    fn active(&self) -> zbus::Result<bool>;
}

/// Watch our logind session's `Active` state and forward transitions to the
/// input runtime as [`Control::SetSessionActive`]. Returns `Ok(())` when the
/// input runtime has gone away (control channel closed) or the change stream
/// ends; returns `Err` only if logind itself can't be reached, in which case the
/// caller logs and the daemon keeps running (grab stays held — same as before
/// this feature existed).
pub async fn run(control_tx: mpsc::Sender<Control>) -> Result<()> {
    let conn = Connection::system()
        .await
        .context("connect system bus for logind")?;
    let manager = LogindManagerProxy::new(&conn)
        .await
        .context("build logind Manager proxy")?;
    let path = manager
        .get_session_by_pid(std::process::id())
        .await
        .context("resolve our logind session")?;
    let session = LogindSessionProxy::builder(&conn)
        .path(path.clone())?
        .build()
        .await
        .context("build logind Session proxy")?;

    // Seed the input runtime with the current state. If logind can't tell us,
    // assume active (true) — the pre-feature behaviour of always holding the
    // grab — rather than starting released.
    let mut active = session.active().await.unwrap_or(true);
    info!("logind session {} active={active}", path.as_str());
    if control_tx
        .send(Control::SetSessionActive(active))
        .await
        .is_err()
    {
        return Ok(()); // input runtime already gone
    }

    let mut changes = session.receive_active_changed().await;
    while let Some(change) = changes.next().await {
        let Ok(next) = change.get().await else {
            continue;
        };
        if next == active {
            continue;
        }
        active = next;
        info!("logind session active -> {active}");
        if control_tx
            .send(Control::SetSessionActive(active))
            .await
            .is_err()
        {
            break; // input runtime gone; nothing more to forward
        }
    }
    Ok(())
}
