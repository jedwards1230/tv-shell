//! LAN HTTP control bridge (#151): a small, opt-in HTTP/1.1 listener that maps
//! `POST /intent/<target>` and `POST /key/<name>` onto the daemon's existing
//! intent and key broadcast paths.
//!
//! **Opt-in**: the bridge only starts when the `GAME_SHELL_HTTP_BIND`
//! environment variable is set to a `host:port` address. When it is unset, no
//! socket is opened and no control surface is exposed.
//!
//! **Auth**: when `GAME_SHELL_HTTP_TOKEN` is set, every request must carry
//! `Authorization: Bearer <token>` (exact match); requests without a valid
//! token receive 401. When the token variable is unset, any request from the
//! bound interface is accepted — the operator is expected to bind to a trusted
//! LAN interface, not a public one.
//!
//! **No new heavy dependencies**: this module uses only `tokio::net::TcpListener`
//! (tokio is already a direct dependency with `features = ["full"]`). `reqwest`
//! is client-only (`health.rs`) and `hyper` is only a transitive dep; neither
//! is used here.
//!
//! **Cross-platform**: this module has no Linux-only imports. The `serve`
//! function can be called on any platform. The binary (`main.rs`) is
//! Linux-gated, so in practice the listener only runs on Linux — but the pure
//! parser and unit tests compile and run on macOS / CI.

use crate::state::Control;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::{mpsc, oneshot};

// ─── Error types ────────────────────────────────────────────────────────────

/// Errors the HTTP request parser can produce, mapped to HTTP status codes.
#[derive(Debug, PartialEq, Eq)]
pub enum HttpError {
    /// The path did not match any known route → 404.
    NotFound,
    /// The method was not POST → 405.
    MethodNotAllowed,
}

// ─── Parsed action ──────────────────────────────────────────────────────────

/// The action decoded from a successful `POST /intent/<target>` or
/// `POST /key/<name>` request.
#[derive(Debug, PartialEq, Eq)]
pub enum HttpAction {
    /// Forward to `Control::Intent { name }`.
    /// The `name` is the full remainder after `/intent/`, percent-decoded.
    Intent(String),
    /// Forward to `Control::Key { name }`.
    /// The `name` is the single token after `/key/`, percent-decoded.
    Key(String),
}

// ─── Pure parser (unit-testable on any host) ────────────────────────────────

/// Decode a single `%XX` escape at position `i` in the byte slice.
/// Returns `(decoded_byte, consumed_len)` or `None` on malformed input.
fn decode_pct(bytes: &[u8], i: usize) -> Option<(u8, usize)> {
    if i + 2 >= bytes.len() {
        return None;
    }
    let hi = (bytes[i + 1] as char).to_digit(16)?;
    let lo = (bytes[i + 2] as char).to_digit(16)?;
    Some(((hi as u8) << 4 | lo as u8, 3))
}

/// Decode a percent-encoded URL path component (e.g. `settings%3Abluetooth`
/// → `settings:bluetooth`). Only the path segment (no query string) is
/// expected.
pub fn url_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'+' {
            // `+` as space is query-string only; in a path it is literal.
            out.push(b'+');
            i += 1;
        } else if bytes[i] == b'%' {
            if let Some((byte, len)) = decode_pct(bytes, i) {
                out.push(byte);
                i += len;
            } else {
                // Malformed escape — pass through literally.
                out.push(bytes[i]);
                i += 1;
            }
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    // Path components are UTF-8; lossy-convert to avoid panicking on
    // garbage input.
    String::from_utf8_lossy(&out).into_owned()
}

/// Parse an HTTP request line (`METHOD SP PATH SP HTTP/...`) and map it to an
/// [`HttpAction`].
///
/// - Accepts `POST` only; any other method → [`HttpError::MethodNotAllowed`].
/// - Strips the query string (everything after `?`) before routing.
/// - `/intent/<target>` → [`HttpAction::Intent`] (`<target>` is the full
///   remainder, percent-decoded; an empty target → [`HttpError::NotFound`]).
/// - `/key/<name>`     → [`HttpAction::Key`]    (percent-decoded; empty →
///   [`HttpError::NotFound`]).
/// - Any other path   → [`HttpError::NotFound`].
pub fn parse_request_line(method: &str, path: &str) -> Result<HttpAction, HttpError> {
    if method != "POST" {
        return Err(HttpError::MethodNotAllowed);
    }

    // Strip query string.
    let path = path.split('?').next().unwrap_or(path);

    if let Some(rest) = path.strip_prefix("/intent/") {
        let name = url_decode(rest);
        if name.is_empty() {
            return Err(HttpError::NotFound);
        }
        return Ok(HttpAction::Intent(name));
    }

    if let Some(rest) = path.strip_prefix("/key/") {
        let name = url_decode(rest);
        if name.is_empty() {
            return Err(HttpError::NotFound);
        }
        return Ok(HttpAction::Key(name));
    }

    Err(HttpError::NotFound)
}

// ─── HTTP response helpers ───────────────────────────────────────────────────

/// Render a minimal HTTP/1.1 response with the given status code and body.
/// The connection is always closed after the response.
pub fn http_response(status: u16, body: &str) -> String {
    let reason = match status {
        200 => "OK",
        400 => "Bad Request",
        401 => "Unauthorized",
        404 => "Not Found",
        405 => "Method Not Allowed",
        503 => "Service Unavailable",
        _ => "Unknown",
    };
    format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
}

// ─── Per-connection handler ──────────────────────────────────────────────────

/// Read up to `MAX_REQUEST_BYTES` from the TCP stream and return the raw
/// header block (up to and including the blank line). Returns `None` on EOF
/// or if no header terminator is found within the limit.
const MAX_REQUEST_BYTES: usize = 4096;

async fn read_headers<S>(stream: &mut S) -> Option<Vec<u8>>
where
    S: AsyncReadExt + Unpin,
{
    let mut buf = vec![0u8; MAX_REQUEST_BYTES];
    let mut filled = 0;

    loop {
        if filled == buf.len() {
            // Request too large — return what we have to let the parser
            // attempt to reject it gracefully.
            break;
        }
        match stream.read(&mut buf[filled..]).await {
            Ok(0) => return None, // EOF
            Ok(n) => {
                filled += n;
                // Look for the blank line that terminates HTTP headers.
                if buf[..filled].windows(4).any(|w| w == b"\r\n\r\n") {
                    break;
                }
            }
            Err(_) => return None,
        }
    }
    Some(buf[..filled].to_vec())
}

/// Extract the `Authorization` header value from raw header bytes.
fn extract_authorization(raw: &[u8]) -> Option<String> {
    let text = std::str::from_utf8(raw).ok()?;
    for line in text.lines() {
        // Case-insensitive match on the header name.
        let Some(colon_pos) = line.find(':') else {
            continue;
        };
        let header_name = line[..colon_pos].trim();
        if header_name.eq_ignore_ascii_case("authorization") {
            return Some(line[colon_pos + 1..].trim().to_owned());
        }
    }
    None
}

/// Send a control request and await the runtime's response line.
async fn request<F>(control_tx: &mpsc::Sender<Control>, make: F) -> Option<String>
where
    F: FnOnce(oneshot::Sender<String>) -> Control,
{
    let (reply_tx, reply_rx) = oneshot::channel();
    control_tx.send(make(reply_tx)).await.ok()?;
    reply_rx.await.ok()
}

/// Handle a single TCP connection: parse the HTTP/1.1 request, check auth,
/// dispatch to the control channel, and write a minimal response.
async fn handle_connection(
    mut stream: tokio::net::TcpStream,
    token: Option<&str>,
    control_tx: &mpsc::Sender<Control>,
) {
    let raw = match read_headers(&mut stream).await {
        Some(b) => b,
        None => {
            tracing::debug!("http: client disconnected before headers");
            return;
        }
    };

    // Parse the request line (first line of the header block).
    let text = match std::str::from_utf8(&raw) {
        Ok(t) => t,
        Err(_) => {
            let resp = http_response(400, "invalid UTF-8 in request");
            let _ = stream.write_all(resp.as_bytes()).await;
            return;
        }
    };

    let first_line = text.lines().next().unwrap_or("");
    let parts: Vec<&str> = first_line.splitn(3, ' ').collect();
    if parts.len() < 2 {
        let resp = http_response(400, "malformed request line");
        let _ = stream.write_all(resp.as_bytes()).await;
        return;
    }
    let method = parts[0];
    let path = parts[1];

    // Auth check.
    if let Some(expected) = token {
        let bearer = format!("Bearer {expected}");
        let auth = extract_authorization(&raw);
        if auth.as_deref() != Some(bearer.as_str()) {
            let resp = http_response(401, "unauthorized");
            let _ = stream.write_all(resp.as_bytes()).await;
            return;
        }
    }

    // Route.
    let action = match parse_request_line(method, path) {
        Ok(a) => a,
        Err(HttpError::NotFound) => {
            let resp = http_response(404, "not found");
            let _ = stream.write_all(resp.as_bytes()).await;
            return;
        }
        Err(HttpError::MethodNotAllowed) => {
            let resp = http_response(405, "method not allowed");
            let _ = stream.write_all(resp.as_bytes()).await;
            return;
        }
    };

    // Dispatch to the daemon control channel.
    let reply = match action {
        HttpAction::Intent(name) => {
            request(control_tx, move |reply| Control::Intent { name, reply }).await
        }
        HttpAction::Key(name) => {
            request(control_tx, move |reply| Control::Key { name, reply }).await
        }
    };

    let resp = match reply {
        None => http_response(503, "daemon unavailable"),
        Some(r) if r.starts_with("error:") => {
            let body = r.trim_start_matches("error:").to_owned();
            http_response(400, &body)
        }
        Some(_) => http_response(200, "ok"),
    };

    let _ = stream.write_all(resp.as_bytes()).await;
}

// ─── Public serve entry point ────────────────────────────────────────────────

/// Bind a TCP listener to `addr` and serve the LAN HTTP control bridge until
/// the process exits.
///
/// Each accepted connection is handled in its own spawned task. Errors are
/// logged at `debug` level and never panic the daemon. The `token` parameter
/// is the optional bearer token from `GAME_SHELL_HTTP_TOKEN`; `None` means no
/// auth (the operator is responsible for binding to a trusted interface).
pub async fn serve(
    addr: std::net::SocketAddr,
    token: Option<String>,
    control_tx: mpsc::Sender<Control>,
) {
    let listener = match TcpListener::bind(addr).await {
        Ok(l) => l,
        Err(e) => {
            tracing::error!("http bridge: failed to bind {addr}: {e}");
            return;
        }
    };
    tracing::info!("Listening on http://{addr}");

    loop {
        match listener.accept().await {
            Ok((stream, peer)) => {
                tracing::debug!("http: connection from {peer}");
                let token = token.clone();
                let control_tx = control_tx.clone();
                tokio::spawn(async move {
                    handle_connection(stream, token.as_deref(), &control_tx).await;
                });
            }
            Err(e) => {
                tracing::debug!("http: accept error: {e}");
            }
        }
    }
}

// ─── Unit tests (pure parser — runs on any host) ────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── parse_request_line ───────────────────────────────────────────────────

    #[test]
    fn intent_simple() {
        assert_eq!(
            parse_request_line("POST", "/intent/menu"),
            Ok(HttpAction::Intent("menu".into()))
        );
    }

    #[test]
    fn intent_with_colon() {
        assert_eq!(
            parse_request_line("POST", "/intent/settings:bluetooth"),
            Ok(HttpAction::Intent("settings:bluetooth".into()))
        );
    }

    #[test]
    fn intent_percent_encoded_colon() {
        // Home Assistant encodes `:` as `%3A` (or `%3a`).
        assert_eq!(
            parse_request_line("POST", "/intent/settings%3Abluetooth"),
            Ok(HttpAction::Intent("settings:bluetooth".into()))
        );
        assert_eq!(
            parse_request_line("POST", "/intent/settings%3abluetooth"),
            Ok(HttpAction::Intent("settings:bluetooth".into()))
        );
    }

    #[test]
    fn intent_deep_link() {
        assert_eq!(
            parse_request_line("POST", "/intent/overlay:volume"),
            Ok(HttpAction::Intent("overlay:volume".into()))
        );
    }

    #[test]
    fn key_simple() {
        assert_eq!(
            parse_request_line("POST", "/key/up"),
            Ok(HttpAction::Key("up".into()))
        );
    }

    #[test]
    fn key_with_query_string_stripped() {
        // Query strings are stripped before routing.
        assert_eq!(
            parse_request_line("POST", "/key/select?foo=bar"),
            Ok(HttpAction::Key("select".into()))
        );
    }

    #[test]
    fn get_method_not_allowed() {
        assert_eq!(
            parse_request_line("GET", "/intent/menu"),
            Err(HttpError::MethodNotAllowed)
        );
    }

    #[test]
    fn unknown_path_not_found() {
        assert_eq!(
            parse_request_line("POST", "/foo"),
            Err(HttpError::NotFound)
        );
    }

    #[test]
    fn empty_intent_leaf_not_found() {
        assert_eq!(
            parse_request_line("POST", "/intent/"),
            Err(HttpError::NotFound)
        );
    }

    #[test]
    fn empty_key_leaf_not_found() {
        assert_eq!(
            parse_request_line("POST", "/key/"),
            Err(HttpError::NotFound)
        );
    }

    #[test]
    fn root_path_not_found() {
        assert_eq!(parse_request_line("POST", "/"), Err(HttpError::NotFound));
    }

    // ── url_decode ───────────────────────────────────────────────────────────

    #[test]
    fn url_decode_plain() {
        assert_eq!(url_decode("hello"), "hello");
    }

    #[test]
    fn url_decode_colon_percent() {
        assert_eq!(url_decode("settings%3Abluetooth"), "settings:bluetooth");
        assert_eq!(url_decode("settings%3abluetooth"), "settings:bluetooth");
    }

    #[test]
    fn url_decode_plus_is_literal_in_path() {
        assert_eq!(url_decode("hello+world"), "hello+world");
    }

    #[test]
    fn url_decode_malformed_pct_passes_through() {
        // `%ZZ` is not valid hex — pass the `%` through literally.
        assert_eq!(url_decode("%ZZ"), "%ZZ");
    }

    // ── http_response ────────────────────────────────────────────────────────

    #[test]
    fn http_response_200_well_formed() {
        let resp = http_response(200, "ok");
        assert!(resp.starts_with("HTTP/1.1 200 OK\r\n"));
        assert!(resp.contains("Content-Length: 2\r\n"));
        assert!(resp.contains("Connection: close\r\n"));
        assert!(resp.ends_with("\r\n\r\nok"));
    }

    #[test]
    fn http_response_400_well_formed() {
        let body = "unknown intent 'foo'";
        let resp = http_response(400, body);
        assert!(resp.starts_with("HTTP/1.1 400 Bad Request\r\n"));
        assert!(resp.contains(&format!("Content-Length: {}\r\n", body.len())));
        assert!(resp.ends_with(&format!("\r\n\r\n{body}")));
    }

    #[test]
    fn http_response_401_well_formed() {
        let resp = http_response(401, "unauthorized");
        assert!(resp.starts_with("HTTP/1.1 401 Unauthorized\r\n"));
    }

    #[test]
    fn http_response_404_well_formed() {
        let resp = http_response(404, "not found");
        assert!(resp.starts_with("HTTP/1.1 404 Not Found\r\n"));
    }

    #[test]
    fn http_response_405_well_formed() {
        let resp = http_response(405, "method not allowed");
        assert!(resp.starts_with("HTTP/1.1 405 Method Not Allowed\r\n"));
    }

    #[test]
    fn http_response_503_well_formed() {
        let resp = http_response(503, "daemon unavailable");
        assert!(resp.starts_with("HTTP/1.1 503 Service Unavailable\r\n"));
    }

    // ── extract_authorization ────────────────────────────────────────────────

    #[test]
    fn extract_auth_present() {
        let raw = b"POST /intent/menu HTTP/1.1\r\nHost: localhost\r\nAuthorization: Bearer secret\r\n\r\n";
        assert_eq!(
            extract_authorization(raw),
            Some("Bearer secret".to_owned())
        );
    }

    #[test]
    fn extract_auth_absent() {
        let raw = b"POST /intent/menu HTTP/1.1\r\nHost: localhost\r\n\r\n";
        assert_eq!(extract_authorization(raw), None);
    }
}
