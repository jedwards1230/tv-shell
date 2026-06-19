//! game-shell-host — a thin, cross-platform sidecar that answers two questions
//! for the game-shell TV client: "what Steam games are installed?" and "launch
//! this one." Moonlight remains the stream engine; this service never touches
//! Sunshine config, so other Moonlight clients are unaffected.
//!
//! Endpoints (all require `Authorization: Bearer <token>`):
//!   GET  /library  → { games: [LibraryEntry, ...] }   (VDF/ACF enumeration)
//!   POST /launch   { appid }  → { ok: true }  (navigates Big Picture to the
//!                                              game's page; the user presses Play)
//!   GET  /status   → { version, running_appid, streaming }
//!
//! Config (env):
//!   GAME_SHELL_HOST_TOKEN — bearer token. If unset, a random one is generated
//!                           and logged on startup.
//!   GAME_SHELL_HOST_PORT  — listen port (default 47995).
//!   GAME_SHELL_HOST_BIND  — listen address (default 0.0.0.0 = all LAN ifaces).

mod launch;
mod steam;

use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    routing::{get, post},
    Json, Router,
};
use game_shell_protocol::{LaunchRequest, LibraryEntry, LibraryResponse};
use serde_json::json;
use std::sync::Arc;
use subtle::ConstantTimeEq;

/// Default listen port. Picked outside Sunshine/Moonlight's 47984–47990 range to
/// avoid any collision with a co-hosted Sunshine.
const DEFAULT_PORT: u16 = 47995;

/// Shared service state: the bearer token (for constant-time compare).
struct AppState {
    token: String,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    let token = resolve_token();
    let port: u16 = std::env::var("GAME_SHELL_HOST_PORT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(DEFAULT_PORT);
    let bind = std::env::var("GAME_SHELL_HOST_BIND").unwrap_or_else(|_| "0.0.0.0".to_string());

    let state = Arc::new(AppState { token });

    let app = Router::new()
        .route("/library", get(library))
        .route("/launch", post(launch_game))
        .route("/status", get(status))
        .with_state(state);

    let addr = format!("{bind}:{port}");
    tracing::info!("game-shell-host listening on {addr}");
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

/// Resolve the bearer token from the env, or generate + log a fresh one. The
/// generated token is logged once at startup so an operator can copy it into the
/// daemon's config; it's never written to disk.
fn resolve_token() -> String {
    if let Ok(t) = std::env::var("GAME_SHELL_HOST_TOKEN") {
        let t = t.trim().to_string();
        if !t.is_empty() {
            return t;
        }
    }
    let generated = generate_token();
    tracing::warn!(
        "GAME_SHELL_HOST_TOKEN unset — generated a random token for this run: {generated}"
    );
    generated
}

/// Generate a 256-bit hex token from the OS time + process id, hashed. Good
/// enough for a per-run dev token; production deployments set the env var. We
/// avoid pulling in a RNG crate to keep the dependency graph minimal.
fn generate_token() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let pid = std::process::id() as u128;
    // Mix two 128-bit values into a 64-hex-char string via a simple splitmix-ish
    // scramble. Not cryptographically strong, but unpredictable enough for a
    // throwaway token; operators are warned to set the env var.
    let mut out = String::with_capacity(64);
    let mut x = nanos ^ (pid.wrapping_mul(0x9E37_79B9_7F4A_7C15));
    for _ in 0..4 {
        x ^= x >> 30;
        x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
        x ^= x >> 27;
        x = x.wrapping_mul(0x94D0_49BB_1331_11EB);
        x ^= x >> 31;
        out.push_str(&format!("{:016x}", (x as u64)));
    }
    out
}

/// Constant-time bearer check. Returns `Ok(())` when the `Authorization` header
/// is `Bearer <token>` and `<token>` matches; otherwise `Err(401)`.
fn authorize(state: &AppState, headers: &HeaderMap) -> Result<(), StatusCode> {
    let presented = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
        .unwrap_or("");
    // ConstantTimeEq over bytes; exact match only — the stored token is trimmed
    // at resolution, so do NOT trim the presented token (a whitespace-padded copy
    // must fail). Length mismatch is handled by ct_eq returning 0.
    let ok: bool = presented.as_bytes().ct_eq(state.token.as_bytes()).into();
    if ok {
        Ok(())
    } else {
        Err(StatusCode::UNAUTHORIZED)
    }
}

/// `GET /library` — enumerate installed Steam games.
async fn library(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<LibraryResponse>, StatusCode> {
    authorize(&state, &headers)?;
    // VDF parsing touches the filesystem; run it off the async reactor.
    let games: Vec<LibraryEntry> = tokio::task::spawn_blocking(steam::enumerate)
        .await
        .unwrap_or_default();
    Ok(Json(LibraryResponse { games }))
}

/// `POST /launch` — navigate Big Picture to a Steam game's page by appid. This no
/// longer auto-starts the game (it fires `steam://nav/games/details/<appid>`); the
/// user presses Play. On Linux it waits for Big Picture to be up first.
async fn launch_game(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(req): Json<LaunchRequest>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    authorize(&state, &headers)?;
    match launch::launch(req.appid) {
        Ok(()) => Ok(Json(json!({ "ok": true, "appid": req.appid }))),
        Err(e) => {
            tracing::warn!("launch {} failed: {e}", req.appid);
            Err(StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}

/// `GET /status` — version + the currently-running Steam appid (or null) +
/// whether a Moonlight/Sunshine stream is active. The running id is detected
/// per-OS (Linux: scanning `/proc` for Steam's `reaper` `SteamLaunch AppId=<n>`
/// launcher; Windows: `registry.vdf`'s `RunningAppID`), so it reflects the running
/// game regardless of how it was started. `running_appid: null` ⇒ nothing running
/// (or detection found no match). `streaming` is true when Sunshine reports a live
/// session (active OR resumable) via its GameStream `serverinfo` state — the same
/// signal the Moonlight client reads (false when Sunshine is idle or unreachable).
async fn status(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, StatusCode> {
    authorize(&state, &headers)?;
    // Both probes touch the OS off-band (a `/proc`/VDF read and a blocking
    // loopback HTTP GET to Sunshine); keep them off the async reactor.
    let running = tokio::task::spawn_blocking(steam::running_appid)
        .await
        .unwrap_or(None);
    let streaming = tokio::task::spawn_blocking(steam::streaming)
        .await
        .unwrap_or(false);
    Ok(Json(json!({
        "version": env!("CARGO_PKG_VERSION"),
        "running_appid": running,
        "streaming": streaming,
    })))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::header::AUTHORIZATION;

    fn state(tok: &str) -> AppState {
        AppState {
            token: tok.to_string(),
        }
    }

    fn bearer(tok: &str) -> HeaderMap {
        let mut h = HeaderMap::new();
        h.insert(AUTHORIZATION, format!("Bearer {tok}").parse().unwrap());
        h
    }

    #[test]
    fn authorize_accepts_matching_token() {
        assert!(authorize(&state("sekret"), &bearer("sekret")).is_ok());
    }

    #[test]
    fn authorize_rejects_wrong_token() {
        assert_eq!(
            authorize(&state("sekret"), &bearer("nope")),
            Err(StatusCode::UNAUTHORIZED)
        );
    }

    #[test]
    fn authorize_rejects_token_with_trailing_spaces() {
        // The presented token must match exactly; a whitespace-padded copy of the
        // correct token must NOT be accepted (no trim on the presented side).
        assert_eq!(
            authorize(&state("sekret"), &bearer("sekret   ")),
            Err(StatusCode::UNAUTHORIZED)
        );
    }

    #[test]
    fn authorize_rejects_missing_header() {
        assert_eq!(
            authorize(&state("sekret"), &HeaderMap::new()),
            Err(StatusCode::UNAUTHORIZED)
        );
    }

    #[test]
    fn authorize_rejects_non_bearer() {
        let mut h = HeaderMap::new();
        h.insert(AUTHORIZATION, "Basic sekret".parse().unwrap());
        assert_eq!(
            authorize(&state("sekret"), &h),
            Err(StatusCode::UNAUTHORIZED)
        );
    }

    #[test]
    fn generated_token_is_64_hex_chars() {
        let t = generate_token();
        assert_eq!(t.len(), 64);
        assert!(t.chars().all(|c| c.is_ascii_hexdigit()));
    }
}
