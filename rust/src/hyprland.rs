//! Hyprland compositor subsystem (Phase 4): a long-lived async actor that owns
//! the Hyprland IPC connection (via the `hyprland` crate). It answers
//! request/response queries over an `mpsc` of [`HyprReq`] and pushes `hypr:*`
//! [`Event`]s onto the shared broadcast bus.
//!
//! READ ONLY: active-window class/title/address and the full client list, plus
//! active-window / fullscreen change events. One-shot compositor *actions*
//! (`hyprctl dispatch exec/closewindow/focuswindow/fullscreen`) deliberately
//! stay shell-outs in the QML.
//!
//! This REPLACES the `hyprctl clients -j` shell-out in
//! `components/HyprctlClients.qml` and feeds `AppLifecycleManager.qml`'s
//! window-event watching.
//!
//! Linux-only (Hyprland IPC socket); `main.rs` declares it under
//! `#[cfg(target_os = "linux")]`. Single-owner discipline mirrors the Phase 3
//! actors (`network.rs` / `power.rs`): the `run` loop owns the data getters and
//! the event listener runs on its own task, pushing onto the broadcast bus.

use crate::protocol::Event;
use crate::state::Reply;
use anyhow::Result;
use hyprland::data::{Client, Clients};
use hyprland::event_listener::AsyncEventListener;
use hyprland::shared::{HyprData, HyprDataActiveOptional};
use serde_json::json;
use std::time::Duration;
use tokio::sync::{broadcast, mpsc};

/// Requests from the IPC server to the Hyprland actor. Each carries a `oneshot`
/// reply with a fully-formatted wire string.
#[derive(Debug)]
pub enum HyprReq {
    /// `hypr-active` -> compact JSON object `{class,title,address}` (`{}` if no
    /// active window).
    Active(Reply),
    /// `hypr-clients` -> compact JSON array of `{class,title,address,workspace}`.
    Clients(Reply),
}

/// Run the Hyprland actor until `rx` is closed.
///
/// Owns the `hyprland` crate's data getters, services [`HyprReq`]s, and (via a
/// spawned async event listener) pushes `hypr:activewindow` / `hypr:fullscreen`
/// events onto `events_tx`. Never panics: if Hyprland isn't running, queries
/// reply with a best-effort empty document and the listener simply exits.
pub async fn run(
    mut rx: mpsc::Receiver<HyprReq>,
    events_tx: broadcast::Sender<Event>,
) -> Result<()> {
    // Spawn the event listener on its own task so the request loop never blocks
    // on it. It owns its own listener (single-owner; no shared mutable state).
    // Retry with capped backoff so it self-heals if Hyprland isn't running yet
    // at daemon start or restarts later — otherwise `hypr:*` events would never
    // resume without a daemon restart.
    {
        let events_tx = events_tx.clone();
        tokio::spawn(async move {
            let mut backoff = Duration::from_secs(1);
            loop {
                match watch_events(events_tx.clone()).await {
                    Ok(()) => backoff = Duration::from_secs(1), // ended cleanly; re-attach
                    Err(e) => tracing::warn!("hyprland: event listener stopped: {e}; retrying"),
                }
                tokio::time::sleep(backoff).await;
                backoff = (backoff * 2).min(Duration::from_secs(30));
            }
        });
    }

    tracing::info!("hyprland actor started");

    while let Some(req) = rx.recv().await {
        match req {
            HyprReq::Active(reply) => {
                let _ = reply.send(active_window_json().await);
            }
            HyprReq::Clients(reply) => {
                let _ = reply.send(clients_json().await);
            }
        }
    }

    tracing::info!("hyprland actor stopped");
    Ok(())
}

/// Build the `hypr-active` compact-JSON object `{class,title,address}`, or `{}`
/// when there's no active window. Degrades to `{}` on any IPC failure (e.g. the
/// Hyprland socket is absent), so the QML page stays usable.
async fn active_window_json() -> String {
    match Client::get_active_async().await {
        Ok(Some(client)) => client_active_json(&client),
        Ok(None) => "{}".to_string(),
        Err(e) => {
            tracing::debug!("hyprland: get_active failed: {e}");
            "{}".to_string()
        }
    }
}

/// Serialize one active [`Client`] as `{class,title,address}` (compact JSON).
fn client_active_json(client: &Client) -> String {
    json!({
        "class": client.class,
        "title": client.title,
        "address": client.address.to_string(),
    })
    .to_string()
}

/// Build the `hypr-clients` compact-JSON array, mirroring `hyprctl clients -j`
/// (at least `class,title,address,workspace`). Degrades to `[]` on IPC failure
/// (e.g. the Hyprland socket is absent).
async fn clients_json() -> String {
    match Clients::get_async().await {
        Ok(clients) => {
            let list: Vec<serde_json::Value> = clients.iter().map(client_entry_json).collect();
            serde_json::Value::Array(list).to_string()
        }
        Err(e) => {
            tracing::debug!("hyprland: get clients failed: {e}");
            "[]".to_string()
        }
    }
}

/// Serialize one [`Client`] as `{class,title,address,workspace}` (compact JSON).
/// `workspace` is the workspace's name, matching what the QML reads from the
/// `hyprctl clients -j` `workspace.name` field.
fn client_entry_json(client: &Client) -> serde_json::Value {
    json!({
        "class": client.class,
        "title": client.title,
        "address": client.address.to_string(),
        "workspace": client.workspace.name,
    })
}

/// Watch Hyprland events and fan `hypr:*` events onto the broadcast bus.
///
/// Registers active-window-change and fullscreen-change handlers on the crate's
/// async event listener, then runs it. Owns the listener exclusively. Each
/// handler clones `events_tx` and pushes the matching [`Event`]; a closed bus
/// (no subscribers) is fine — `broadcast::Sender::send` just returns an error we
/// ignore.
///
/// If Hyprland isn't running, `start_listener_async` returns an error and this
/// function exits; the request handlers still degrade gracefully on their own.
async fn watch_events(events_tx: broadcast::Sender<Event>) -> Result<()> {
    let mut listener = AsyncEventListener::new();

    // Active window changed: emit the new active window's class. `None` (no
    // focused window) maps to an empty class, matching the QML's wire contract.
    {
        let events_tx = events_tx.clone();
        listener.add_active_window_change_handler(move |data| {
            let events_tx = events_tx.clone();
            // `WindowEventData { window_class, window_title, window_address }`.
            let class = data.map(|d| d.window_class).unwrap_or_default();
            Box::pin(async move {
                let _ = events_tx.send(Event::HyprActiveWindow(class));
            })
        });
    }

    // Fullscreen state changed: emit `hypr:fullscreen:<0|1>`.
    {
        let events_tx = events_tx.clone();
        listener.add_fullscreen_state_change_handler(move |fullscreen| {
            let events_tx = events_tx.clone();
            Box::pin(async move {
                let _ = events_tx.send(Event::HyprFullscreen(fullscreen));
            })
        });
    }

    listener.start_listener_async().await?;
    Ok(())
}
