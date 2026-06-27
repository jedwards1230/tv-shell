//! Reusable client plumbing for a remote game-shell widget **sidecar**.
//!
//! A widget that needs heavy backend logic ships a sidecar process; the daemon
//! reaches it over the LAN. [`crate::steam`] (the Steam library/launch backend,
//! `game-shell-host`, running on the gaming PC) is the first — and today only —
//! consumer.
//!
//! **The daemon is an HTTP _client_, not a process supervisor.** The sidecar runs
//! on a *different machine* (see `docs/HOST_SETUP.md`), so there is no spawn,
//! health-restart, or shutdown here — "lifecycle" is reachability (is it
//! answering?), not process management. This module owns the three things every
//! such sidecar client needs, so a future widget backend reuses them instead of
//! re-deriving them:
//!   1. holding a base URL + bearer token ([`Sidecar::from_parts`]),
//!   2. a reachability probe ([`Sidecar::probe`]) over the shared
//!      [`crate::service_health`] vocabulary, and
//!   3. bearer HTTP request helpers with response size caps and typed JSON
//!      decoding ([`Sidecar::get_json`] / [`Sidecar::get_classified`] /
//!      [`Sidecar::post`]).
//!
//! All requests use the shared [`crate::service_health::build_client`] policy
//! (short timeouts, self-signed-tolerant) so every remote-service fetch in the
//! daemon shares one client configuration.

use crate::service_health::{build_client, classify_code, probe_get, ServiceStatus};
use serde::de::DeserializeOwned;
use serde_json::Value;

/// A configured remote sidecar endpoint: its base URL and bearer token.
#[derive(Clone, Debug)]
pub struct Sidecar {
    base: String,
    token: String,
}

/// Error from a sidecar request.
#[derive(Debug)]
pub enum SidecarError {
    /// Transport failure or a non-2xx HTTP status (`error_for_status`).
    Http(reqwest::Error),
    /// The body did not decode as the expected type.
    Json(serde_json::Error),
    /// The body advertised more than the caller's size cap — refused before
    /// reading it into memory (a DoS guard against a rogue/compromised host; the
    /// request timeout alone doesn't cap body size).
    TooLarge(u64),
}

impl std::fmt::Display for SidecarError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SidecarError::Http(e) => write!(f, "http: {e}"),
            SidecarError::Json(e) => write!(f, "json: {e}"),
            SidecarError::TooLarge(n) => write!(f, "response body too large: {n} bytes"),
        }
    }
}

impl std::error::Error for SidecarError {}

impl Sidecar {
    /// Build from a base URL + bearer token. The base is trimmed and a trailing
    /// slash removed so path joins are clean; returns `None` when either the base
    /// or the token is empty (the widget is effectively unconfigured). The token
    /// is kept verbatim — the host compares it byte-for-byte, so trimming it here
    /// would silently break a token whose file legitimately contains those bytes.
    pub fn from_parts(base: &str, token: &str) -> Option<Self> {
        let base = base.trim().trim_end_matches('/').to_string();
        if base.is_empty() || token.is_empty() {
            return None;
        }
        Some(Self {
            base,
            token: token.to_string(),
        })
    }

    /// The trailing-slash-trimmed base URL — callers use it to build art/asset
    /// URLs that point back at the sidecar.
    pub fn base(&self) -> &str {
        &self.base
    }

    /// `Authorization` header value for every request.
    fn bearer(&self) -> String {
        format!("Bearer {}", self.token)
    }

    /// Reachability probe: authenticated `GET {base}/status`, classified by the
    /// shared health vocabulary (401 ⇒ `Error`/bad token, 5xx ⇒ `Unreachable`,
    /// transport failure ⇒ `Unreachable`). The lightweight check a background
    /// poller uses to answer "is the sidecar up?" without fetching real data.
    pub async fn probe(&self) -> ServiceStatus {
        let url = format!("{}/status", self.base);
        probe_get(&url, &[("Authorization", &self.bearer())]).await
    }

    /// `GET {base}{path}` with the bearer + `Accept: application/json`, returning
    /// the reachability classification and — only when reachable (`Ok`) — the
    /// response body as text. A body advertising more than `max_bytes` is dropped
    /// (`None`) while the `Ok` status is kept. Use when the caller needs BOTH the
    /// reachability signal and the body from a single request (e.g. `/status`,
    /// whose body also carries the foreground-game id).
    pub async fn get_classified(
        &self,
        path: &str,
        max_bytes: u64,
    ) -> (ServiceStatus, Option<String>) {
        let url = format!("{}{}", self.base, path);
        // A client we can't even build means we never reached the server — that's
        // Unreachable (transient), not Error (server reached us and rejected us).
        let client = match build_client() {
            Ok(c) => c,
            Err(_) => return (ServiceStatus::Unreachable, None),
        };
        let resp = match client
            .get(&url)
            .header("Authorization", self.bearer())
            .header("Accept", "application/json")
            .send()
            .await
        {
            Ok(r) => r,
            Err(_) => return (ServiceStatus::Unreachable, None),
        };
        let status = classify_code(resp.status().as_u16());
        if status != ServiceStatus::Ok {
            return (status, None);
        }
        if resp.content_length().is_some_and(|len| len > max_bytes) {
            tracing::warn!("sidecar GET {url} body too large (> {max_bytes} bytes); ignoring");
            return (status, None);
        }
        match resp.text().await {
            Ok(body) => (status, Some(body)),
            Err(_) => (status, None),
        }
    }

    /// `GET {base}{path}` with the bearer + `Accept: application/json`, then decode
    /// the body into `T`. A non-2xx is an error (`error_for_status`), a body over
    /// `max_bytes` is refused before reading, and a decode failure surfaces as
    /// [`SidecarError::Json`] (which closes daemon↔sidecar schema drift — a host
    /// that changed the shape fails loudly here instead of silently dropping
    /// fields, the way the previous untyped `Value` parse did).
    pub async fn get_json<T: DeserializeOwned>(
        &self,
        path: &str,
        max_bytes: u64,
    ) -> Result<T, SidecarError> {
        let url = format!("{}{}", self.base, path);
        let client = build_client().map_err(SidecarError::Http)?;
        let resp = client
            .get(&url)
            .header("Authorization", self.bearer())
            .header("Accept", "application/json")
            .send()
            .await
            .map_err(SidecarError::Http)?
            .error_for_status()
            .map_err(SidecarError::Http)?;
        if let Some(len) = resp.content_length() {
            if len > max_bytes {
                return Err(SidecarError::TooLarge(len));
            }
        }
        let body = resp.text().await.map_err(SidecarError::Http)?;
        serde_json::from_str(&body).map_err(SidecarError::Json)
    }

    /// `POST {base}{path}` with the bearer and an optional JSON body. Any non-2xx
    /// is an error. `Content-Type: application/json` is set only when a body is
    /// present (a bodyless action like `/open-bpm` sends neither).
    pub async fn post(&self, path: &str, body: Option<&Value>) -> Result<(), reqwest::Error> {
        let url = format!("{}{}", self.base, path);
        let client = build_client()?;
        let mut req = client.post(&url).header("Authorization", self.bearer());
        if let Some(body) = body {
            req = req
                .header("Content-Type", "application/json")
                .body(body.to_string());
        }
        req.send().await?.error_for_status()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_parts_trims_trailing_slash_and_whitespace() {
        let sc = Sidecar::from_parts("  http://host:47995/  ", "tok").unwrap();
        assert_eq!(sc.base(), "http://host:47995");
        assert_eq!(sc.bearer(), "Bearer tok");
    }

    #[test]
    fn from_parts_keeps_token_verbatim() {
        // The host compares the bearer byte-for-byte, so a token is NOT trimmed.
        let sc = Sidecar::from_parts("http://h", " spaced ").unwrap();
        assert_eq!(sc.bearer(), "Bearer  spaced ");
    }

    #[test]
    fn from_parts_none_when_base_empty() {
        assert!(Sidecar::from_parts("   ", "tok").is_none());
        assert!(Sidecar::from_parts("", "tok").is_none());
    }

    #[test]
    fn from_parts_none_when_token_empty() {
        assert!(Sidecar::from_parts("http://h", "").is_none());
    }
}
