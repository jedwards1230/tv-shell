//! LAN HTTP control bridge (#151): a small, opt-in HTTP/1.1 listener that maps
//! `POST /intent/<target>`, `POST /key/<name>`, and `GET /screenshot` onto the
//! daemon's existing intent/key broadcast paths and the `grim` screenshotter.
//!
//! **Opt-in**: the bridge only starts when the `GAME_SHELL_HTTP_BIND`
//! environment variable is set to a `host:port` address. When it is unset, no
//! socket is opened and no control surface is exposed.
//!
//! **Auth**: controlled by `GAME_SHELL_HTTP_AUTH_ENABLED` (default: enabled).
//! Set to `0` or `false` to skip auth entirely (local-only dev). When enabled
//! (the default), `GAME_SHELL_HTTP_TOKEN` must be set; every request must carry
//! `Authorization: Bearer <token>` (constant-time comparison, #151). If auth is
//! enabled but no token is configured, all requests are rejected with 401 and a
//! loud warning is logged. When the token variable is unset AND auth is disabled,
//! any request from the bound interface is accepted — the operator is responsible
//! for binding to a trusted interface. A `tracing::warn!` is emitted when
//! binding to an unspecified address without a token.
//!
//! **Screenshot**: `GET /screenshot` (also `GET /screenshot.png`) shells out to
//! `grim -` which writes a PNG to stdout. The PNG bytes are returned with
//! `Content-Type: image/png`. Auth applies to this route (it exposes screen
//! content). On grim failure a 500 text response is returned.
//!
//! **DoS hardening (#151)**:
//! - Each accepted connection is wrapped in a 5-second `tokio::time::timeout`
//!   so a slow sender that never sends the blank-line HTTP terminator cannot
//!   hold a task slot indefinitely.
//! - An atomic `ACTIVE_CONNS` counter caps concurrent connections at
//!   `MAX_ACTIVE_CONNS` (128). Connections beyond the cap receive an immediate
//!   503 and are dropped, preventing fd / task-pool exhaustion.
//!
//! **No new heavy dependencies**: this module uses only `tokio::net::TcpListener`,
//! `tokio::time`, `tokio::process`, and the `subtle` crate (already a transitive
//! dep, now direct). `reqwest` is client-only (`health.rs`) and `hyper` is only
//! a transitive dep; neither is used here.
//!
//! **Cross-platform**: this module has no Linux-only imports. The `serve`
//! function can be called on any platform. The binary (`main.rs`) is
//! Linux-gated, so in practice the listener only runs on Linux — but the pure
//! parser and unit tests compile and run on macOS / CI.

use crate::state::Control;
use subtle::ConstantTimeEq;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::{mpsc, oneshot};

// ─── Error types ────────────────────────────────────────────────────────────

/// Errors the HTTP request parser can produce, mapped to HTTP status codes.
#[derive(Debug, PartialEq, Eq)]
pub enum HttpError {
    /// The path did not match any known route → 404.
    NotFound,
    /// The method was not POST (or GET for screenshot routes) → 405.
    MethodNotAllowed,
}

// ─── Parsed action ──────────────────────────────────────────────────────────

/// The action decoded from a successful request.
#[derive(Debug, PartialEq, Eq)]
pub enum HttpAction {
    /// Forward to `Control::Intent { name }`.
    /// The `name` is the full remainder after `/intent/`, percent-decoded.
    Intent(String),
    /// Forward to `Control::Key { name }`.
    /// The `name` is the single token after `/key/`, percent-decoded.
    Key(String),
    /// Capture the current screen via `grim -` and return the PNG.
    Screenshot,
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
/// - `GET /screenshot`       → [`HttpAction::Screenshot`]
/// - `GET /screenshot.png`   → [`HttpAction::Screenshot`]
/// - Non-GET for `/screenshot[.png]` → [`HttpError::MethodNotAllowed`]
/// - `POST /intent/<target>` → [`HttpAction::Intent`] (`<target>` percent-decoded;
///   empty target → [`HttpError::NotFound`]).
/// - `POST /key/<name>`      → [`HttpAction::Key`] (percent-decoded; empty →
///   [`HttpError::NotFound`]).
/// - Non-POST for any other path → [`HttpError::MethodNotAllowed`].
/// - Unknown POST path       → [`HttpError::NotFound`].
///
/// Query strings (everything after `?`) are stripped before routing.
pub fn parse_request_line(method: &str, path: &str) -> Result<HttpAction, HttpError> {
    // Strip query string.
    let path = path.split('?').next().unwrap_or(path);

    // Screenshot routes: GET only.
    if path == "/screenshot" || path == "/screenshot.png" {
        if method == "GET" {
            return Ok(HttpAction::Screenshot);
        } else {
            return Err(HttpError::MethodNotAllowed);
        }
    }

    // All remaining routes require POST.
    if method != "POST" {
        return Err(HttpError::MethodNotAllowed);
    }

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

/// Render a minimal HTTP/1.1 response with the given status code and text body.
/// The connection is always closed after the response.
pub fn http_response(status: u16, body: &str) -> String {
    let reason = match status {
        200 => "OK",
        400 => "Bad Request",
        401 => "Unauthorized",
        404 => "Not Found",
        405 => "Method Not Allowed",
        500 => "Internal Server Error",
        503 => "Service Unavailable",
        _ => "Unknown",
    };
    format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
}

/// Build the header block for a binary PNG response.
/// Returns the raw header bytes (including the final CRLFCRLF). The caller must
/// write these bytes then immediately write the raw PNG bytes — do NOT use
/// `http_response` for binary bodies since it assumes a UTF-8 `&str` body.
fn png_response_header(png_len: usize) -> Vec<u8> {
    let header = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: image/png\r\nContent-Length: {png_len}\r\nConnection: close\r\n\r\n"
    );
    header.into_bytes()
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

/// Constant-time comparison of two strings to prevent timing oracles.
/// Uses `subtle::ConstantTimeEq` on the byte representations (#151).
fn ct_eq_str(a: &str, b: &str) -> bool {
    // ConstantTimeEq on slices requires equal lengths; a length mismatch
    // leaks only the length (not which character differs), which is
    // acceptable for a fixed-format "Bearer <token>" prefix.
    a.as_bytes().ct_eq(b.as_bytes()).into()
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
/// dispatch to the control channel or screenshotter, and write a response.
async fn handle_connection(
    mut stream: tokio::net::TcpStream,
    token: Option<&str>,
    auth_enabled: bool,
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

    // Auth check — constant-time comparison to prevent timing oracles (#151).
    // When auth is enabled but no token is configured, reject all requests
    // (secure by default — you cannot authenticate without a token).
    if auth_enabled {
        match token {
            None => {
                // Auth enabled but no token set — reject all requests.
                let resp = http_response(401, "unauthorized");
                let _ = stream.write_all(resp.as_bytes()).await;
                return;
            }
            Some(expected) => {
                let bearer = format!("Bearer {expected}");
                let auth = extract_authorization(&raw);
                let provided = auth.as_deref().unwrap_or("");
                if !ct_eq_str(provided, &bearer) {
                    let resp = http_response(401, "unauthorized");
                    let _ = stream.write_all(resp.as_bytes()).await;
                    return;
                }
            }
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

    // Dispatch to the daemon control channel or screenshotter.
    match action {
        HttpAction::Intent(name) => {
            let reply =
                request(control_tx, move |reply| Control::Intent { name, reply }).await;
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
        HttpAction::Key(name) => {
            let reply =
                request(control_tx, move |reply| Control::Key { name, reply }).await;
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
        HttpAction::Screenshot => {
            // Shell out to grim(1) — captures the Wayland display to stdout as PNG.
            // `grim -` writes the PNG to stdout; we capture it and stream it back.
            // The daemon runs inside the graphical session so WAYLAND_DISPLAY is set.
            let result = tokio::process::Command::new("grim")
                .arg("-")
                .output()
                .await;
            match result {
                Ok(out) if out.status.success() => {
                    let png = out.stdout;
                    // Binary-safe response: write the header then the raw PNG bytes.
                    // We do NOT use http_response() here — that function takes a &str
                    // body and would corrupt arbitrary binary data.
                    let header = png_response_header(png.len());
                    let _ = stream.write_all(&header).await;
                    let _ = stream.write_all(&png).await;
                }
                Ok(out) => {
                    // grim exited non-zero — include stderr in the 500 body.
                    let stderr = String::from_utf8_lossy(&out.stderr);
                    let body = format!("grim failed: {}", stderr.trim());
                    let resp = http_response(500, &body);
                    let _ = stream.write_all(resp.as_bytes()).await;
                }
                Err(e) => {
                    let body = format!("grim error: {e}");
                    let resp = http_response(500, &body);
                    let _ = stream.write_all(resp.as_bytes()).await;
                }
            }
        }
    }
}

// ─── Public serve entry point ────────────────────────────────────────────────

/// Maximum number of concurrently active HTTP connections (#151 DoS hardening).
const MAX_ACTIVE_CONNS: usize = 128;

/// Read timeout for each accepted connection in seconds (#151 DoS hardening).
const READ_TIMEOUT_SECS: u64 = 5;

/// Parse `GAME_SHELL_HTTP_AUTH_ENABLED` from the environment.
/// Returns `true` (auth enabled) unless the value is exactly `"0"` or `"false"`.
/// When unset, auth is enabled (secure by default).
pub fn read_auth_enabled() -> bool {
    match std::env::var("GAME_SHELL_HTTP_AUTH_ENABLED") {
        Ok(val) => val != "0" && val != "false",
        Err(_) => true, // unset → enabled
    }
}

/// Bind a TCP listener to `addr` and serve the LAN HTTP control bridge until
/// the process exits.
///
/// Each accepted connection is handled in its own spawned task, wrapped in a
/// `READ_TIMEOUT_SECS`-second timeout so a slow sender cannot hold a task slot
/// indefinitely. Concurrent connections are capped at `MAX_ACTIVE_CONNS`;
/// connections beyond the cap receive an immediate 503 and are dropped.
///
/// The `token` parameter is the optional bearer token from
/// `GAME_SHELL_HTTP_TOKEN`. The `auth_enabled` parameter is read from
/// `GAME_SHELL_HTTP_AUTH_ENABLED` (default `true`). When auth is enabled but no
/// token is configured, all requests are rejected with 401 and a loud warning is
/// logged. When auth is disabled, a warning is logged and auth is skipped
/// entirely (for local-only dev). A warning is also emitted when binding to an
/// unspecified address (0.0.0.0 or ::) without a token.
pub async fn serve(
    addr: std::net::SocketAddr,
    token: Option<String>,
    auth_enabled: bool,
    control_tx: mpsc::Sender<Control>,
) {
    if !auth_enabled {
        tracing::warn!(
            "http bridge: AUTH DISABLED (GAME_SHELL_HTTP_AUTH_ENABLED=0) — \
             any host on the network can send control commands without authentication"
        );
    } else if token.is_none() {
        // Auth is enabled but no token is configured — all requests will be
        // rejected with 401. Log a loud warning so the operator knows to set it.
        tracing::warn!(
            "http bridge: auth is ENABLED but GAME_SHELL_HTTP_TOKEN is not set — \
             all requests will be rejected with 401 (set the token or disable auth \
             with GAME_SHELL_HTTP_AUTH_ENABLED=0)"
        );
    } else if addr.ip().is_unspecified() {
        // Token is set but we're binding to 0.0.0.0/:: — still worth a note.
        tracing::warn!(
            "http bridge: binding to {} with bearer auth — \
             any host on the network can attempt authentication",
            addr
        );
    }

    let listener = match TcpListener::bind(addr).await {
        Ok(l) => l,
        Err(e) => {
            tracing::error!("http bridge: failed to bind {addr}: {e}");
            return;
        }
    };
    tracing::info!("Listening on http://{addr}");

    // Atomic connection counter for the cap (#151 DoS hardening).
    let active_conns = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));

    loop {
        match listener.accept().await {
            Ok((stream, peer)) => {
                let current = active_conns.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                if current >= MAX_ACTIVE_CONNS {
                    active_conns.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
                    tracing::debug!(
                        "http: connection from {peer} rejected — at cap ({MAX_ACTIVE_CONNS})"
                    );
                    // Spawn a minimal task to send 503 and drop the stream.
                    tokio::spawn(async move {
                        let mut s = stream;
                        let resp = http_response(503, "too many connections");
                        let _ = s.write_all(resp.as_bytes()).await;
                    });
                    continue;
                }
                tracing::debug!("http: connection from {peer}");
                let token = token.clone();
                let control_tx = control_tx.clone();
                let conns = active_conns.clone();
                tokio::spawn(async move {
                    // Wrap the handler in a timeout so a slow sender that
                    // never sends the blank-line terminator cannot hold a
                    // task slot indefinitely (#151).
                    let _ = tokio::time::timeout(
                        std::time::Duration::from_secs(READ_TIMEOUT_SECS),
                        handle_connection(stream, token.as_deref(), auth_enabled, &control_tx),
                    )
                    .await;
                    conns.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
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
    fn get_screenshot() {
        assert_eq!(
            parse_request_line("GET", "/screenshot"),
            Ok(HttpAction::Screenshot)
        );
    }

    #[test]
    fn get_screenshot_png() {
        assert_eq!(
            parse_request_line("GET", "/screenshot.png"),
            Ok(HttpAction::Screenshot)
        );
    }

    #[test]
    fn get_screenshot_with_query_stripped() {
        assert_eq!(
            parse_request_line("GET", "/screenshot?foo=bar"),
            Ok(HttpAction::Screenshot)
        );
    }

    #[test]
    fn post_screenshot_method_not_allowed() {
        assert_eq!(
            parse_request_line("POST", "/screenshot"),
            Err(HttpError::MethodNotAllowed)
        );
    }

    #[test]
    fn get_method_not_allowed_for_intent() {
        assert_eq!(
            parse_request_line("GET", "/intent/menu"),
            Err(HttpError::MethodNotAllowed)
        );
    }

    #[test]
    fn get_unknown_path_method_not_allowed() {
        // A GET to an unknown path still gets 405 (not 404) — method check
        // happens before path check for non-screenshot routes.
        assert_eq!(
            parse_request_line("GET", "/foo"),
            Err(HttpError::MethodNotAllowed)
        );
    }

    #[test]
    fn unknown_path_not_found() {
        assert_eq!(parse_request_line("POST", "/foo"), Err(HttpError::NotFound));
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
    fn http_response_500_well_formed() {
        let resp = http_response(500, "grim failed: exit 1");
        assert!(resp.starts_with("HTTP/1.1 500 Internal Server Error\r\n"));
    }

    #[test]
    fn http_response_503_well_formed() {
        let resp = http_response(503, "daemon unavailable");
        assert!(resp.starts_with("HTTP/1.1 503 Service Unavailable\r\n"));
    }

    // ── png_response_header ──────────────────────────────────────────────────

    #[test]
    fn png_response_header_well_formed() {
        let header = png_response_header(1234);
        let s = std::str::from_utf8(&header).unwrap();
        assert!(s.starts_with("HTTP/1.1 200 OK\r\n"));
        assert!(s.contains("Content-Type: image/png\r\n"));
        assert!(s.contains("Content-Length: 1234\r\n"));
        assert!(s.contains("Connection: close\r\n"));
        assert!(s.ends_with("\r\n\r\n"));
    }

    // ── extract_authorization ────────────────────────────────────────────────

    #[test]
    fn extract_auth_present() {
        let raw = b"POST /intent/menu HTTP/1.1\r\nHost: localhost\r\nAuthorization: Bearer secret\r\n\r\n";
        assert_eq!(extract_authorization(raw), Some("Bearer secret".to_owned()));
    }

    #[test]
    fn extract_auth_absent() {
        let raw = b"POST /intent/menu HTTP/1.1\r\nHost: localhost\r\n\r\n";
        assert_eq!(extract_authorization(raw), None);
    }

    // ── ct_eq_str ────────────────────────────────────────────────────────────

    #[test]
    fn ct_eq_str_equal() {
        assert!(ct_eq_str("Bearer secret123", "Bearer secret123"));
    }

    #[test]
    fn ct_eq_str_different() {
        assert!(!ct_eq_str("Bearer secret123", "Bearer secret124"));
        assert!(!ct_eq_str("Bearer a", "Bearer bb"));
        assert!(!ct_eq_str("", "Bearer x"));
    }

    // ── read_auth_enabled / parse_auth_enabled_val ───────────────────────────

    #[test]
    fn auth_enabled_val_disabled_for_zero_and_false() {
        assert!(!parse_auth_enabled_val("0"));
        assert!(!parse_auth_enabled_val("false"));
    }

    #[test]
    fn auth_enabled_val_enabled_for_other_values() {
        assert!(parse_auth_enabled_val("1"));
        assert!(parse_auth_enabled_val("true"));
        assert!(parse_auth_enabled_val("yes"));
        assert!(parse_auth_enabled_val(""));
    }
}

#[cfg(test)]
/// Helper used in unit tests to exercise the auth-enabled parsing logic
/// without touching the real environment.
fn parse_auth_enabled_val(val: &str) -> bool {
    val != "0" && val != "false"
}
