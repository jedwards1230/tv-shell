//! Generic remote-service health: a shared HTTP probe, a status vocabulary, and
//! a background poller that broadcasts `health:<json>` events.
//!
//! **Why this exists.** Home-screen widgets that read a remote service (the Plex
//! widget's `plex-hubs`, the Moonlight cards' `sunshine-status`) each used to
//! conflate "server unreachable" with "no data" — a failed fetch degraded to an
//! empty result, so the UI silently collapsed instead of telling the user the
//! *server* was down. This module gives every such service one shared
//! reachability classification ([`ServiceStatus`]) and one wire vocabulary, so a
//! widget can render a graceful "can't reach X right now" state.
//!
//! **Two consumers of the vocabulary:**
//! - Per-request handlers ([`crate::plex`], [`crate::health`]) embed a `status`
//!   field in their reply so the shell paints correctly on the *first* fetch.
//! - This module's [`run`] background poller probes always-on services (Plex)
//!   on a timer and broadcasts `health:<json>` events on the global bus, so the
//!   status stays live for any number of subscribers without each widget
//!   polling — and survives independently of whether a widget is mounted.
//!
//! The poll cadence and the per-request fetch are deliberately separate: the
//! poller answers "is the service reachable?" cheaply (a HEAD-like GET of a
//! lightweight endpoint); the per-request handler still fetches the actual data
//! (hubs / serverinfo) when a widget needs it.

use crate::protocol::Event;
use serde_json::json;
use std::time::Duration;
use tokio::sync::broadcast;

/// How long between health polls for an always-on service. Matches the shell's
/// historical Plex-widget refresh cadence so behaviour is unchanged in volume,
/// only in correctness.
const POLL_INTERVAL: Duration = Duration::from_secs(60);

/// Standard probe timeouts. A health probe should fail *fast* — a hung server
/// is "unreachable" for UX purposes well before a human would wait.
const PROBE_TIMEOUT: Duration = Duration::from_secs(6);
const PROBE_CONNECT_TIMEOUT: Duration = Duration::from_secs(3);

/// Reachability classification shared by every remote-service check.
///
/// The distinction the old code lacked: `Unreachable` (server-side problem —
/// down, restarting, behind a proxy with no backend) is separate from `Error`
/// (we reached *something* but it rejected us — bad token, bad request) and from
/// `Disabled` (the service isn't configured at all, so showing nothing is
/// correct). Only `Ok` means "go ahead and fetch / show data".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServiceStatus {
    /// Not configured (no URL/token). The widget should collapse silently.
    Disabled,
    /// Reachable and serving. Safe to fetch and render data.
    Ok,
    /// Configured but the server isn't answering (transport error, timeout, or
    /// a 5xx like Traefik's `503 no available server`). Show a "can't reach"
    /// notice — this is transient / server-side.
    Unreachable,
    /// Reached, but the request was rejected (auth failure, 4xx). Show a
    /// "check configuration" notice — this usually needs human attention.
    Error,
}

impl ServiceStatus {
    /// Stable wire/JSON token. This is the contract the QML `ServiceMonitor`
    /// matches on — keep it in sync with `shell/components/lib/ServiceMonitor.qml`.
    pub fn as_str(self) -> &'static str {
        match self {
            ServiceStatus::Disabled => "disabled",
            ServiceStatus::Ok => "ok",
            ServiceStatus::Unreachable => "unreachable",
            ServiceStatus::Error => "error",
        }
    }
}

/// Map an HTTP status code to a health classification. Pure — unit-tested.
///
/// - `2xx`/`3xx` ⇒ `Ok` (served, possibly a redirect to the real doc).
/// - `5xx` ⇒ `Unreachable` — the canonical case is a reverse proxy returning
///   `503 no available server` when the backend pod is down; from the user's
///   point of view the *service* is unavailable, not misconfigured.
/// - everything else (`4xx`: 401/403 auth, 404, 400 …) ⇒ `Error` — we reached a
///   live endpoint that rejected the request; that's a config/credential issue.
pub fn classify_code(code: u16) -> ServiceStatus {
    match code {
        200..=399 => ServiceStatus::Ok,
        500..=599 => ServiceStatus::Unreachable,
        _ => ServiceStatus::Error,
    }
}

/// Build the standard health/probe HTTP client: short timeouts and
/// `danger_accept_invalid_certs` so a self-signed origin (Sunshine's HTTPS port,
/// a Plex server reached directly on `:32400`) classifies on its HTTP status
/// rather than failing the TLS handshake. Shared by [`probe_get`] and
/// [`crate::health`] so every remote-service fetch uses one client policy.
pub(crate) fn build_client() -> Result<reqwest::Client, reqwest::Error> {
    reqwest::Client::builder()
        .danger_accept_invalid_certs(true)
        .timeout(PROBE_TIMEOUT)
        .connect_timeout(PROBE_CONNECT_TIMEOUT)
        .build()
}

/// GET `url` with optional headers and classify the outcome. Transport failures
/// (DNS/connect/TLS/timeout) are `Unreachable` — the server-side-issue bucket.
pub async fn probe_get(url: &str, headers: &[(&str, &str)]) -> ServiceStatus {
    let client = match build_client() {
        Ok(c) => c,
        // A client we can't even build is an internal error, not a server one.
        Err(_) => return ServiceStatus::Error,
    };
    let mut req = client.get(url);
    for (k, v) in headers {
        req = req.header(*k, *v);
    }
    match req.send().await {
        Ok(resp) => classify_code(resp.status().as_u16()),
        Err(_) => ServiceStatus::Unreachable,
    }
}

/// Render a `health:<json>` event payload: `{"service":<name>,"status":<status>}`.
/// Kept here so the event shape has a single source of truth.
pub fn health_json(service: &str, status: ServiceStatus) -> String {
    json!({ "service": service, "status": status.as_str() }).to_string()
}

/// Probe Plex for reachability. Returns `Disabled` when unconfigured, otherwise
/// the classification of a lightweight `GET /identity` (cheaper than the full
/// hubs fetch and unauthenticated-friendly — it still 503s when the backend is
/// down, which is exactly what we want to surface).
pub async fn probe_plex() -> ServiceStatus {
    match crate::plex::config() {
        None => ServiceStatus::Disabled,
        Some((base, token)) => {
            let url = format!("{base}/identity");
            probe_get(&url, &[("X-Plex-Token", &token), ("Accept", "application/json")]).await
        }
    }
}

/// Background health poller: probe each always-on service on a timer and emit a
/// `health:<json>` event whenever its status changes. Fire-and-forget, spawned
/// alongside the D-Bus actors in `main.rs`; it never panics and degrades to
/// no-op events if a service is unconfigured.
///
/// Emit-on-change (not every tick) keeps the bus quiet; a subscriber that joins
/// mid-run primes its first paint from the per-request handler's `status` field
/// instead of waiting for the next change.
pub async fn run(events_tx: broadcast::Sender<Event>) {
    let mut ticker = tokio::time::interval(POLL_INTERVAL);
    // The first tick fires immediately, so a subscriber present at startup gets
    // the initial status right away.
    let mut last_plex: Option<ServiceStatus> = None;

    loop {
        ticker.tick().await;

        let plex = probe_plex().await;
        if last_plex != Some(plex) {
            last_plex = Some(plex);
            // Send failures are fine to ignore: a closed bus only happens at
            // shutdown, and a lagged subscriber is the subscriber's problem.
            let _ = events_tx.send(Event::ServiceHealth(health_json("plex", plex)));
            tracing::debug!("health: plex -> {}", plex.as_str());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_2xx_3xx_is_ok() {
        assert_eq!(classify_code(200), ServiceStatus::Ok);
        assert_eq!(classify_code(204), ServiceStatus::Ok);
        assert_eq!(classify_code(301), ServiceStatus::Ok);
        assert_eq!(classify_code(302), ServiceStatus::Ok);
    }

    #[test]
    fn classify_5xx_is_unreachable() {
        // The motivating case: Traefik "503 no available server" when the Plex
        // backend pod is down.
        assert_eq!(classify_code(503), ServiceStatus::Unreachable);
        assert_eq!(classify_code(500), ServiceStatus::Unreachable);
        assert_eq!(classify_code(502), ServiceStatus::Unreachable);
    }

    #[test]
    fn classify_4xx_is_error() {
        // Reached a live endpoint that rejected us — config/credential issue.
        assert_eq!(classify_code(401), ServiceStatus::Error);
        assert_eq!(classify_code(403), ServiceStatus::Error);
        assert_eq!(classify_code(404), ServiceStatus::Error);
        assert_eq!(classify_code(400), ServiceStatus::Error);
    }

    #[test]
    fn status_wire_tokens_are_stable() {
        assert_eq!(ServiceStatus::Disabled.as_str(), "disabled");
        assert_eq!(ServiceStatus::Ok.as_str(), "ok");
        assert_eq!(ServiceStatus::Unreachable.as_str(), "unreachable");
        assert_eq!(ServiceStatus::Error.as_str(), "error");
    }

    #[test]
    fn health_json_shape() {
        let s = health_json("plex", ServiceStatus::Unreachable);
        let v: serde_json::Value = serde_json::from_str(&s).unwrap();
        assert_eq!(v["service"], "plex");
        assert_eq!(v["status"], "unreachable");
    }
}
