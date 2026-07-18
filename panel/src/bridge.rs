//! Daemon HTTP bridge client — the DEV-OPS data tier for the panel.
//!
//! The daemon exposes an opt-in HTTP bridge (`[http].bind` in config.toml,
//! default off). Routes used here (authoritative: `daemon/src/http.rs`):
//!
//! - `POST /dev/deploy?ref=<ref>`
//! - `POST /dev/build`
//! - `POST /dev/restart-shell`
//! - `POST /dev/restart-daemon`
//! - `GET  /dev/logs?lines=N&filter=<str>` — text log body (default 100,
//!   capped 1000)
//! - `GET  /dev/status` — JSON status blob
//!
//! When the daemon has auth enabled, an `Authorization: Bearer <token>`
//! header is required; this client attaches one whenever a token is
//! configured.

use std::time::Duration;

/// Client timeout. 190s to comfortably exceed the daemon's own `/dev/build`
/// timeout budget (180s, `DEV_TIMEOUT_SECS` in `daemon/src/http.rs`).
const CLIENT_TIMEOUT: Duration = Duration::from_secs(190);

/// Errors a bridge call can produce.
#[derive(Debug)]
pub enum BridgeError {
    /// No `[http].bind` is configured — the bridge client has no base URL.
    NotConfigured,
    /// The bridge could not be reached (connect refused/timeout/DNS/etc).
    Unreachable(String),
    /// The bridge responded with a non-2xx status.
    Status(u16, String),
}

impl BridgeError {
    /// `true` only for [`BridgeError::NotConfigured`] — i.e. whether the
    /// client has a base URL at all is a separate concern from whether it's
    /// currently reachable.
    pub fn is_configured(&self) -> bool {
        !matches!(self, BridgeError::NotConfigured)
    }
}

impl std::fmt::Display for BridgeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BridgeError::NotConfigured => write!(f, "HTTP bridge not configured"),
            BridgeError::Unreachable(msg) => write!(f, "HTTP bridge unreachable: {msg}"),
            BridgeError::Status(code, body) => write!(f, "HTTP bridge returned {code}: {body}"),
        }
    }
}

impl std::error::Error for BridgeError {}

/// A client for the daemon's opt-in LAN HTTP dev-ops bridge.
pub struct BridgeClient {
    base: Option<String>,
    token: Option<String>,
    http: reqwest::Client,
}

impl BridgeClient {
    /// Build a client. `base` is `Some("http://host:port")` when
    /// `[http].bind` is configured; `token` is the bearer token to attach
    /// (when the daemon has auth enabled).
    pub fn new(base: Option<String>, token: Option<String>) -> Self {
        // A builder failure (bad system CA store, OOM) is rare but real. Fall
        // back to a default client so the panel still starts — the bridge is an
        // optional, opt-in surface and must not brick the panel — but log loudly:
        // the default client has NO request timeout, so the CLIENT_TIMEOUT ceiling
        // on dev ops is lost until the process is restarted.
        let http = reqwest::Client::builder()
            .timeout(CLIENT_TIMEOUT)
            .build()
            .unwrap_or_else(|e| {
                tracing::warn!(
                    "bridge: reqwest client build failed ({e}); falling back to a \
                     default client WITHOUT the {CLIENT_TIMEOUT:?} request timeout"
                );
                reqwest::Client::new()
            });
        Self { base, token, http }
    }

    /// `GET /dev/status` — JSON status blob (returned as raw text; callers
    /// that need structure can `serde_json::from_str` it).
    pub async fn dev_status(&self) -> Result<String, BridgeError> {
        self.get("/dev/status", &[]).await
    }

    /// `GET /dev/logs?lines=N&filter=<str>` — text log body.
    pub async fn dev_logs(
        &self,
        lines: usize,
        filter: Option<&str>,
    ) -> Result<String, BridgeError> {
        let lines_str = lines.to_string();
        let mut query: Vec<(&str, &str)> = vec![("lines", &lines_str)];
        if let Some(f) = filter {
            query.push(("filter", f));
        }
        self.get("/dev/logs", &query).await
    }

    /// `POST /dev/deploy?ref=<ref>`.
    pub async fn deploy(&self, git_ref: Option<&str>) -> Result<String, BridgeError> {
        let query: Vec<(&str, &str)> = match git_ref {
            Some(r) => vec![("ref", r)],
            None => vec![],
        };
        self.post("/dev/deploy", &query).await
    }

    /// `POST /dev/build`.
    pub async fn build(&self) -> Result<String, BridgeError> {
        self.post("/dev/build", &[]).await
    }

    /// `POST /dev/restart-shell`.
    pub async fn restart_shell(&self) -> Result<String, BridgeError> {
        self.post("/dev/restart-shell", &[]).await
    }

    /// `POST /dev/restart-daemon`.
    pub async fn restart_daemon(&self) -> Result<String, BridgeError> {
        self.post("/dev/restart-daemon", &[]).await
    }

    async fn get(&self, path: &str, query: &[(&str, &str)]) -> Result<String, BridgeError> {
        let base = self.base.as_ref().ok_or(BridgeError::NotConfigured)?;
        let url = format!("{base}{path}");
        let mut req = self.http.get(&url).query(query);
        if let Some(token) = &self.token {
            req = req.bearer_auth(token);
        }
        Self::send(req).await
    }

    async fn post(&self, path: &str, query: &[(&str, &str)]) -> Result<String, BridgeError> {
        let base = self.base.as_ref().ok_or(BridgeError::NotConfigured)?;
        let url = format!("{base}{path}");
        let mut req = self.http.post(&url).query(query);
        if let Some(token) = &self.token {
            req = req.bearer_auth(token);
        }
        Self::send(req).await
    }

    async fn send(req: reqwest::RequestBuilder) -> Result<String, BridgeError> {
        let resp = req
            .send()
            .await
            .map_err(|e| BridgeError::Unreachable(e.to_string()))?;
        let status = resp.status();
        let body = resp
            .text()
            .await
            .map_err(|e| BridgeError::Unreachable(e.to_string()))?;
        if status.is_success() {
            Ok(body)
        } else {
            Err(BridgeError::Status(status.as_u16(), body))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn not_configured_when_base_is_none() {
        let client = BridgeClient::new(None, None);
        let err = client.dev_status().await.unwrap_err();
        assert!(matches!(err, BridgeError::NotConfigured));
        assert!(!err.is_configured());
    }
}
