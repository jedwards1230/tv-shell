//! Sunshine session detection (Phase 4).
//!
//! Pre-flight check the QML shell runs before launching a Moonlight stream:
//! "is this host up, are we paired, and is another app already streaming?".
//! This replaces the inline `curl`/`python3` Sunshine HTTP polls in
//! `components/StreamManager.qml`, `StreamCard.qml`, and `MoonlightSettings.qml`.
//!
//! Sunshine's `/serverinfo` endpoint (the GameStream-compatible info doc) is a
//! small XML document. The HTTP port (47989) serves it unauthenticated; the
//! HTTPS port (47984/47990) is self-signed, so the fetch uses
//! `reqwest` with `rustls-tls` + `danger_accept_invalid_certs`.
//!
//! Cross-platform: `reqwest` runs everywhere, so this is a stateless ipc.rs
//! handler (like `list-apps`) rather than a Linux-only actor. The response
//! *parser* ([`parse_serverinfo`]) is a pure function unit-tested on macOS.

use crate::protocol;
use serde_json::json;

/// Parsed view of a Sunshine `/serverinfo` response.
///
/// Fields mirror the compact-JSON object the `sunshine-status` command returns:
/// `{online,paired,currentApp,httpsPort}`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerInfo {
    /// The host responded with a parseable serverinfo document.
    pub online: bool,
    /// `<PairStatus>1</PairStatus>` — this client is paired with the host.
    pub paired: bool,
    /// Name/id of the app currently streaming, or `""` when idle. Derived from
    /// `<state>` + `<currentgame>`: busy with a non-zero game id ⇒ that id,
    /// otherwise empty.
    pub current_app: String,
    /// `<HttpsPort>` from the document (0 when absent/unparseable).
    pub https_port: u32,
}

impl ServerInfo {
    /// The "host unreachable / unparseable" result: offline, unpaired, idle.
    fn offline() -> Self {
        ServerInfo {
            online: false,
            paired: false,
            current_app: String::new(),
            https_port: 0,
        }
    }

    /// Render as the compact-JSON `sunshine-status` reply body.
    ///
    /// Carries the shared [`crate::service_health::ServiceStatus`] `status` token
    /// (mapped from `online`) alongside the legacy `online`/`paired`/`currentApp`
    /// fields, so the QML `ServiceMonitor` reads the same vocabulary as the Plex
    /// widget. The Sunshine serverinfo port is unauthenticated, so a failed fetch
    /// is always treated as `unreachable` rather than `error`.
    pub fn to_json(&self) -> String {
        use crate::service_health::ServiceStatus;
        let status = if self.online {
            ServiceStatus::Ok
        } else {
            ServiceStatus::Unreachable
        };
        json!({
            "status": status.as_str(),
            "online": self.online,
            "paired": self.paired,
            "currentApp": self.current_app,
            "httpsPort": self.https_port,
        })
        .to_string()
    }
}

/// Extract the text content of the first `<tag>…</tag>` element, trimming
/// surrounding whitespace. Returns `None` if the tag is absent.
///
/// Sunshine's serverinfo is flat and small, so a substring scan is sufficient
/// and avoids pulling in an XML crate.
fn xml_text<'a>(xml: &'a str, tag: &str) -> Option<&'a str> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let start = xml.find(&open)? + open.len();
    let end = xml[start..].find(&close)? + start;
    Some(xml[start..end].trim())
}

/// Parse a Sunshine `/serverinfo` XML body into a [`ServerInfo`].
///
/// Pure function (no I/O) so it unit-tests on every platform. An empty or
/// non-serverinfo string parses to the offline result. The `currentApp`
/// derivation mirrors the QML logic: a host is "busy" only when
/// `<state>` ends in `SUNSHINE_SERVER_BUSY` AND `<currentgame>` is a non-zero
/// id; otherwise it is idle (`currentApp = ""`).
pub fn parse_serverinfo(xml: &str) -> ServerInfo {
    // A valid serverinfo doc always carries a `<state>` element. Without it we
    // treat the host as offline/unreachable.
    let Some(state) = xml_text(xml, "state") else {
        return ServerInfo::offline();
    };

    let paired = xml_text(xml, "PairStatus")
        .map(|s| s == "1")
        .unwrap_or(false);

    let https_port = xml_text(xml, "HttpsPort")
        .and_then(|s| s.parse::<u32>().ok())
        .unwrap_or(0);

    // Busy ⇒ another app is streaming. `<state>` is e.g.
    // `SUNSHINE_SERVER_BUSY` / `SUNSHINE_SERVER_FREE` (GameStream uses
    // `MJOLNIR_SERVER_*`); match the BUSY suffix to be backend-agnostic.
    let busy = state.ends_with("SERVER_BUSY");
    let current_game = xml_text(xml, "currentgame").unwrap_or("0");
    let current_app = if busy && current_game != "0" && !current_game.is_empty() {
        current_game.to_string()
    } else {
        String::new()
    };

    ServerInfo {
        online: true,
        paired,
        current_app,
        https_port,
    }
}

/// Fetch Sunshine `/serverinfo` from `host:port` over its self-signed HTTPS
/// endpoint and return the `sunshine-status` compact-JSON reply body.
///
/// Network errors / timeouts / non-2xx degrade to the offline JSON
/// (`{"online":false,...}`) rather than erroring — the QML pre-flight only needs
/// a best-effort snapshot to decide whether to prompt about a session conflict.
pub async fn sunshine_status(host: &str, port: &str) -> String {
    match fetch_serverinfo(host, port).await {
        Ok(body) => parse_serverinfo(&body).to_json(),
        Err(e) => {
            tracing::debug!("sunshine-status fetch failed for {host}:{port}: {e}");
            ServerInfo::offline().to_json()
        }
    }
}

/// Perform the HTTPS GET against `/serverinfo`. Uses the shared health client
/// ([`crate::service_health::build_client`]) which accepts Sunshine's self-signed
/// cert. Returns the response body text.
async fn fetch_serverinfo(host: &str, port: &str) -> Result<String, reqwest::Error> {
    let url = format!("https://{host}:{port}/serverinfo");
    let client = crate::service_health::build_client()?;
    let resp = client.get(&url).send().await?.error_for_status()?;
    resp.text().await
}

/// Wrap [`sunshine_status`] for the IPC layer. `host`/`port` come from the
/// parsed `SunshineStatus { host, port }` command.
pub async fn handle_sunshine_status(host: &str, port: &str) -> String {
    // Defensive: an empty host/port shouldn't reach here (the parser routes
    // those to `SunshineStatusUsage`), but guard anyway so we never build a
    // bogus URL.
    if host.is_empty() || port.is_empty() {
        return protocol::resp_sunshine_status_usage();
    }
    sunshine_status(host, port).await
}

#[cfg(test)]
mod tests {
    use super::*;

    const BUSY: &str = r#"<?xml version="1.0"?>
<root status_code="200">
  <appversion>7.1.4</appversion>
  <state>SUNSHINE_SERVER_BUSY</state>
  <PairStatus>1</PairStatus>
  <currentgame>881448767</currentgame>
  <HttpsPort>47984</HttpsPort>
</root>"#;

    const FREE_PAIRED: &str = r#"<?xml version="1.0"?>
<root status_code="200">
  <state>SUNSHINE_SERVER_FREE</state>
  <PairStatus>1</PairStatus>
  <currentgame>0</currentgame>
  <HttpsPort>47990</HttpsPort>
</root>"#;

    const FREE_UNPAIRED: &str = r#"<?xml version="1.0"?>
<root status_code="200">
  <state>SUNSHINE_SERVER_FREE</state>
  <PairStatus>0</PairStatus>
  <currentgame>0</currentgame>
  <HttpsPort>47990</HttpsPort>
</root>"#;

    #[test]
    fn parses_busy_host() {
        let info = parse_serverinfo(BUSY);
        assert!(info.online);
        assert!(info.paired);
        assert_eq!(info.current_app, "881448767");
        assert_eq!(info.https_port, 47984);
    }

    #[test]
    fn parses_free_paired_host() {
        let info = parse_serverinfo(FREE_PAIRED);
        assert!(info.online);
        assert!(info.paired);
        assert_eq!(info.current_app, "");
        assert_eq!(info.https_port, 47990);
    }

    #[test]
    fn parses_free_unpaired_host() {
        let info = parse_serverinfo(FREE_UNPAIRED);
        assert!(info.online);
        assert!(!info.paired);
        assert_eq!(info.current_app, "");
    }

    #[test]
    fn busy_with_zero_game_is_idle() {
        // BUSY state but currentgame=0 (transient) ⇒ treated as idle.
        let xml = "<state>SUNSHINE_SERVER_BUSY</state><currentgame>0</currentgame>";
        assert_eq!(parse_serverinfo(xml).current_app, "");
    }

    #[test]
    fn missing_state_is_offline() {
        assert_eq!(parse_serverinfo(""), ServerInfo::offline());
        assert_eq!(parse_serverinfo("garbage"), ServerInfo::offline());
        assert!(!parse_serverinfo("<root></root>").online);
    }

    #[test]
    fn absent_https_port_is_zero() {
        let xml = "<state>SUNSHINE_SERVER_FREE</state>";
        assert_eq!(parse_serverinfo(xml).https_port, 0);
    }

    #[test]
    fn json_shape_offline() {
        // preserve_order: status,online,paired,currentApp,httpsPort.
        assert_eq!(
            ServerInfo::offline().to_json(),
            r#"{"status":"unreachable","online":false,"paired":false,"currentApp":"","httpsPort":0}"#
        );
    }

    #[test]
    fn json_shape_busy() {
        assert_eq!(
            parse_serverinfo(BUSY).to_json(),
            r#"{"status":"ok","online":true,"paired":true,"currentApp":"881448767","httpsPort":47984}"#
        );
    }
}
