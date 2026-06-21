//! Steam library proxy for the home-screen Steam widget.
//!
//! Mirrors [`crate::plex`]: a stateless, cross-platform `reqwest` handler served
//! from `ipc.rs` like `list-apps`, not a Linux-only actor. The shell's
//! `SteamWidget` polls the `steam-library` IPC command and renders poster cards
//! for the installed Steam games on the gaming PC; selecting one fires
//! `steam-launch <appid>` (also handled here) and then starts the existing
//! Moonlight stream.
//!
//! The actual enumeration + launch live in `game-shell-host` (a thin
//! cross-platform sidecar on the gaming PC). This daemon module just proxies to
//! that host's `GET /library` / `POST /launch` / `POST /open-bpm` / `POST /quit`
//! over HTTP with a bearer token.
//!
//! **Config via env** (set in `~/.config/game-shell/daemon.env`):
//! - `GAME_SHELL_STEAM_URL`   — game-shell-host base, e.g. `http://192.0.2.1:47995`
//! - `GAME_SHELL_STEAM_TOKEN` — the host's bearer token (`GAME_SHELL_HOST_TOKEN`)
//!
//! The reply carries a `status` field from the shared [`crate::service_health`]
//! vocabulary (`disabled`/`ok`/`unreachable`/`error`) so the widget tells a down
//! host apart from an empty library. When unconfigured the command returns
//! `{"status":"disabled",…}` so the widget collapses to nothing. The token lives
//! only in the daemon's environment; QML never reads it.
//!
//! The response *normalizer* ([`normalize_library`]) is a pure function,
//! unit-tested on every platform; only the live fetch needs a reachable host.

use serde_json::{json, Value};

/// Recently-played rail cap. The rows only show a handful at 4K.
const RECENT_LIMIT: usize = 12;

/// Resolve `(base_url, token)` from `[steam]` in `config.toml`, or `None` when
/// the widget is unconfigured. Trailing slash on the base is trimmed so URL joins
/// are clean. The token is by inline `token` or `token_file` (see daemon_config).
///
/// `pub(crate)` so [`crate::service_health`] can reuse the same resolution for a
/// reachability probe — one source of truth for "is Steam configured?".
pub(crate) fn config() -> Option<(String, String)> {
    let cfg = crate::daemon_config::global();
    let base = cfg
        .steam
        .url
        .as_deref()?
        .trim()
        .trim_end_matches('/')
        .to_string();
    // steam_token() is Result; startup validate() already vetted token files, so
    // a hard error or missing token here just means "widget not configured".
    let token = cfg.steam_token().ok().flatten()?;
    if base.is_empty() || token.is_empty() {
        return None;
    }
    Some((base, token))
}

/// IPC entry point for `steam-library`. Returns
/// `{"status":<status>,"recentlyPlayed":[…],"allGames":[…]}` where `status` is
/// the shared [`crate::service_health::ServiceStatus`] vocabulary.
///
/// A lightweight `GET /status` reachability probe runs first; only on `Ok` is the
/// library fetched, so a down host yields empty rails *with a status the widget
/// can render*. Unconfigured ⇒ `disabled` (widget collapses).
pub async fn handle_steam_library() -> String {
    use crate::service_health::ServiceStatus;

    let Some((base, token)) = config() else {
        return json!({
            "status": ServiceStatus::Disabled.as_str(),
            "recentlyPlayed": [],
            "allGames": [],
            "runningAppid": Value::Null,
            "streaming": false,
        })
        .to_string();
    };

    // Reachability gate — `GET /status` is small and authenticated, so its HTTP
    // status classifies cleanly (401 → Error/bad token, 5xx → Unreachable). We
    // also read its body to capture `running_appid` (the foreground game on the
    // host) and `streaming` (an active Moonlight/Sunshine stream) so the widget
    // can badge the "Playing" card and reflect stream state without a second
    // request.
    let (probe, running_appid, streaming) = fetch_status(&base, &token).await;
    if probe != ServiceStatus::Ok {
        return json!({
            "status": probe.as_str(),
            "recentlyPlayed": [],
            "allGames": [],
            "runningAppid": Value::Null,
            "streaming": false,
        })
        .to_string();
    }

    match fetch_library(&base, &token).await {
        Ok(games) => normalize_library(&games, &base, running_appid, streaming),
        Err(e) => {
            tracing::debug!("steam-library fetch failed: {e}");
            json!({
                "status": ServiceStatus::Unreachable.as_str(),
                "recentlyPlayed": [],
                "allGames": [],
                "runningAppid": Value::Null,
                "streaming": false,
            })
            .to_string()
        }
    }
}

/// GET `{base}/status` and return the reachability classification, the host's
/// `running_appid` (the foreground Steam game, or `None`), and `streaming` (true
/// when a Moonlight/Sunshine stream is active on the host). The body is only
/// parsed on a 2xx/3xx; any error degrades to `(status, None, false)` so the
/// caller still gets a clean status to surface. The poster widget uses the running
/// id to badge the "Playing" card and `streaming` to reflect stream state — source
/// of truth is the host, not which card the user tapped.
async fn fetch_status(
    base: &str,
    token: &str,
) -> (crate::service_health::ServiceStatus, Option<u64>, bool) {
    use crate::service_health::{classify_code, ServiceStatus};

    let url = format!("{base}/status");
    let client = match crate::service_health::build_client() {
        Ok(c) => c,
        // A client-build failure (TLS init, proxy config) means we couldn't even
        // attempt the probe — that's Unreachable (transient), not Error (which
        // means the server reached us and rejected us, e.g. a 401).
        Err(_) => return (ServiceStatus::Unreachable, None, false),
    };
    let resp = match client
        .get(&url)
        .header("Authorization", format!("Bearer {token}"))
        .header("Accept", "application/json")
        .send()
        .await
    {
        Ok(r) => r,
        Err(_) => return (ServiceStatus::Unreachable, None, false),
    };
    let status = classify_code(resp.status().as_u16());
    if status != ServiceStatus::Ok {
        return (status, None, false);
    }
    // Bound the body before reading it: /status is a tiny JSON, so refuse a
    // response advertising more than 64 KiB — a misconfigured/compromised host
    // could otherwise stream a huge body into memory (the 6s timeout alone
    // doesn't cap size). Guards the common Content-Length-bearing case.
    const MAX_STATUS_BODY: u64 = 64 * 1024;
    if resp
        .content_length()
        .is_some_and(|len| len > MAX_STATUS_BODY)
    {
        tracing::warn!("steam /status body too large (> {MAX_STATUS_BODY} bytes); ignoring");
        return (status, None, false);
    }
    // Parse the body best-effort; a missing/non-numeric/zero running_appid is
    // None, and a missing/non-bool streaming defaults to false.
    let (running, streaming) = match resp.text().await {
        Ok(body) => match serde_json::from_str::<Value>(&body).ok() {
            Some(v) => {
                let running = v
                    .get("running_appid")
                    .and_then(|r| r.as_u64())
                    .filter(|&id| id != 0);
                let streaming = v
                    .get("streaming")
                    .and_then(|s| s.as_bool())
                    .unwrap_or(false);
                (running, streaming)
            }
            None => (None, false),
        },
        Err(_) => (None, false),
    };
    (status, running, streaming)
}

/// IPC entry point for `steam-launch <appid>`. POSTs `{appid}` to the host's
/// `/launch` and returns `ok` / `error:*`. The Moonlight stream start stays in
/// QML — this only kicks off the game on the host.
pub async fn handle_steam_launch(appid: u32) -> String {
    let Some((base, token)) = config() else {
        return crate::protocol::resp_error("steam not configured");
    };
    match post_launch(&base, &token, appid).await {
        Ok(()) => crate::protocol::resp_ok(),
        Err(e) => {
            tracing::debug!("steam-launch {appid} failed: {e}");
            crate::protocol::resp_error(&format!("steam-launch failed: {e}"))
        }
    }
}

/// IPC entry point for `steam-bigpicture`. POSTs to the host's `/open-bpm` (no
/// body) to reset Steam to the Big Picture HOME screen, and returns a compact-JSON
/// status object (`{"status":"ok"}` / `{"status":"error","reason":…}`). Mirrors
/// [`handle_steam_launch`] but lands on the BPM home rather than a game's page;
/// the Moonlight stream start stays in QML. Degrades gracefully when the host is
/// unconfigured (`disabled`) or unreachable (`error`), exactly like steam-launch.
pub async fn handle_steam_bigpicture() -> String {
    use crate::service_health::ServiceStatus;

    let Some((base, token)) = config() else {
        return json!({
            "status": ServiceStatus::Disabled.as_str(),
            "reason": "steam not configured",
        })
        .to_string();
    };
    match post_open_bpm(&base, &token).await {
        Ok(()) => json!({ "status": ServiceStatus::Ok.as_str() }).to_string(),
        Err(e) => {
            tracing::debug!("steam-bigpicture failed: {e}");
            json!({
                "status": ServiceStatus::Error.as_str(),
                "reason": format!("steam-bigpicture failed: {e}"),
            })
            .to_string()
        }
    }
}

/// POST `{base}/open-bpm` with the bearer token and no body. Any non-2xx is an
/// error. Companion to [`post_launch`] — opens Big Picture's home screen.
async fn post_open_bpm(base: &str, token: &str) -> Result<(), reqwest::Error> {
    let url = format!("{base}/open-bpm");
    let client = crate::service_health::build_client()?;
    client
        .post(&url)
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await?
        .error_for_status()?;
    Ok(())
}

/// IPC entry point for `steam-quit <appid>`. POSTs `{appid}` to the host's `/quit`
/// to gracefully terminate the running game (SIGTERM to its process group — like
/// Steam's Stop button), and returns a compact-JSON status object
/// (`{"status":"ok"}` / `{"status":"error","reason":…}`). Mirrors
/// [`handle_steam_bigpicture`]; the Moonlight stream close stays in QML. Degrades
/// gracefully when the host is unconfigured (`disabled`) or unreachable (`error`).
pub async fn handle_steam_quit(appid: u32) -> String {
    use crate::service_health::ServiceStatus;

    let Some((base, token)) = config() else {
        return json!({
            "status": ServiceStatus::Disabled.as_str(),
            "reason": "steam not configured",
        })
        .to_string();
    };
    match post_quit(&base, &token, appid).await {
        Ok(()) => json!({ "status": ServiceStatus::Ok.as_str() }).to_string(),
        Err(e) => {
            tracing::debug!("steam-quit {appid} failed: {e}");
            json!({
                "status": ServiceStatus::Error.as_str(),
                "reason": format!("steam-quit failed: {e}"),
            })
            .to_string()
        }
    }
}

/// POST `{base}/quit` with `{appid}` and the bearer token. Any non-2xx is an
/// error. Companion to [`post_launch`] — gracefully terminates the running game.
async fn post_quit(base: &str, token: &str, appid: u32) -> Result<(), reqwest::Error> {
    let url = format!("{base}/quit");
    let client = crate::service_health::build_client()?;
    client
        .post(&url)
        .header("Authorization", format!("Bearer {token}"))
        .header("Content-Type", "application/json")
        .body(json!({ "appid": appid }).to_string())
        .send()
        .await?
        .error_for_status()?;
    Ok(())
}

/// Error type for a host fetch: a transport failure (`reqwest`) or a body that
/// did not parse as JSON (`serde_json`).
#[derive(Debug)]
enum FetchError {
    Http(reqwest::Error),
    Json(serde_json::Error),
    /// Response body advertised more bytes than the sanity cap — refused before
    /// reading it into memory (DoS guard against a rogue/compromised host).
    TooLarge(u64),
}

impl std::fmt::Display for FetchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FetchError::Http(e) => write!(f, "http: {e}"),
            FetchError::Json(e) => write!(f, "json: {e}"),
            FetchError::TooLarge(n) => write!(f, "response body too large: {n} bytes"),
        }
    }
}

/// GET `{base}/library` with the bearer token and parse the JSON body. The body
/// is read as text and parsed via `serde_json` (matching `plex.rs`'s `.text()`
/// usage — the crate's `reqwest` has no `json` feature).
async fn fetch_library(base: &str, token: &str) -> Result<Value, FetchError> {
    let url = format!("{base}/library");
    let client = crate::service_health::build_client().map_err(FetchError::Http)?;
    let resp = client
        .get(&url)
        .header("Authorization", format!("Bearer {token}"))
        .header("Accept", "application/json")
        .send()
        .await
        .map_err(FetchError::Http)?
        .error_for_status()
        .map_err(FetchError::Http)?;
    // Bound the body before reading it, like `fetch_status` does for /status —
    // /library is legitimately larger (one entry per game), so use a higher cap:
    // 10 MiB covers ~1000 games. Refuses a rogue/compromised host streaming an
    // unbounded body into memory (the 6s timeout alone doesn't cap size).
    const MAX_LIBRARY_BODY: u64 = 10 * 1024 * 1024;
    if let Some(len) = resp.content_length() {
        if len > MAX_LIBRARY_BODY {
            return Err(FetchError::TooLarge(len));
        }
    }
    let body = resp.text().await.map_err(FetchError::Http)?;
    serde_json::from_str(&body).map_err(FetchError::Json)
}

/// POST `{base}/launch` with `{appid}` and the bearer token. Any non-2xx is an
/// error.
async fn post_launch(base: &str, token: &str, appid: u32) -> Result<(), reqwest::Error> {
    let url = format!("{base}/launch");
    let client = crate::service_health::build_client()?;
    client
        .post(&url)
        .header("Authorization", format!("Bearer {token}"))
        .header("Content-Type", "application/json")
        .body(json!({ "appid": appid }).to_string())
        .send()
        .await?
        .error_for_status()?;
    Ok(())
}

/// Normalize a `LibraryResponse` (`{games:[…]}`) into the widget's two rails:
/// `recentlyPlayed` (sorted by `last_played` desc, top [`RECENT_LIMIT`]) and
/// `allGames` (sorted by name). `running_appid` (the host's foreground game, or
/// `None`) is passed through to the reply's `runningAppid` so the widget can
/// badge the "Playing" card; `streaming` (whether a stream is active on the host)
/// is passed through to `streaming`. `base` is the host base URL (already
/// trailing-slash-trimmed by [`config`]) used to build each card's `localArt`
/// fallback URL (`{base}/art/{appid}`). Pure function — no I/O — so it unit-tests
/// anywhere. A body with no `games` array yields empty rails (but `status:ok`,
/// matching an installed-but-empty library).
pub fn normalize_library(
    root: &Value,
    base: &str,
    running_appid: Option<u64>,
    streaming: bool,
) -> String {
    use crate::service_health::ServiceStatus;

    let running = running_appid.map(Value::from).unwrap_or(Value::Null);

    let games = root.get("games").and_then(|g| g.as_array());
    let Some(games) = games else {
        return json!({
            "status": ServiceStatus::Ok.as_str(),
            "recentlyPlayed": [],
            "allGames": [],
            "runningAppid": running,
            "streaming": streaming,
        })
        .to_string();
    };

    // Project each entry to the widget shape and keep only installed games (a
    // partially-downloaded title isn't launchable yet).
    let mut cards: Vec<Value> = games
        .iter()
        // Default a missing `installed` to FALSE (exclude) — safer than letting a
        // possibly-uninstalled, unlaunchable game through. The host always sends
        // the field, so this default only ever guards malformed input.
        .filter(|g| {
            g.get("installed")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
        })
        // filter_map: `card` returns None for a missing/zero appid (an unaddressable
        // entry) so it's skipped rather than emitted as a broken card.
        .filter_map(|g| card(g, base))
        .collect();

    // allGames: by name (case-insensitive).
    cards.sort_by(|a, b| {
        let an = a["name"].as_str().unwrap_or("").to_lowercase();
        let bn = b["name"].as_str().unwrap_or("").to_lowercase();
        an.cmp(&bn)
    });
    let all_games = cards.clone();

    // recentlyPlayed: by last_played desc, drop never-played, cap.
    let mut recent: Vec<Value> = cards
        .into_iter()
        .filter(|c| c["lastPlayed"].as_u64().unwrap_or(0) > 0)
        .collect();
    recent.sort_by(|a, b| {
        let al = a["lastPlayed"].as_u64().unwrap_or(0);
        let bl = b["lastPlayed"].as_u64().unwrap_or(0);
        bl.cmp(&al)
    });
    recent.truncate(RECENT_LIMIT);

    json!({
        "status": ServiceStatus::Ok.as_str(),
        "recentlyPlayed": recent,
        "allGames": all_games,
        "runningAppid": running,
        "streaming": streaming,
    })
    .to_string()
}

/// Map one host `LibraryEntry` to the widget card shape `{appid,name,art,
/// localArt,headerArt,lastPlayed}`. The `art`/`headerArt` poster + header URLs are
/// built off the appid against Steam's public CDN (no token), and `localArt`
/// points at the host's own `/art/{appid}` endpoint (`{base}/art/{appid}`) which
/// serves the on-disk cache art for titles the CDN is missing — QML uses it as a
/// fallback between `art` and `headerArt`. All three bind to `Image.source`.
fn card(entry: &Value, base: &str) -> Option<Value> {
    // A missing or zero appid can't address Steam's CDN or our /art endpoint, so
    // skip the entry (filter_map drops the None) rather than emit a broken card
    // pointing at `.../apps/0/...`.
    let appid = entry
        .get("appid")
        .and_then(|v| v.as_u64())
        .filter(|&id| id != 0)?;
    let name = entry.get("name").and_then(|v| v.as_str()).unwrap_or("");
    let last_played = entry.get("last_played").and_then(|v| v.as_u64());
    Some(json!({
        "appid": appid,
        "name": name,
        "art": library_art_url(appid),
        "localArt": local_art_url(base, appid),
        "headerArt": header_art_url(appid),
        "lastPlayed": last_played,
    }))
}

/// Portrait library poster (600×900). Verified HTTP 200 on Steam's CDN.
fn library_art_url(appid: u64) -> String {
    format!("https://steamcdn-a.akamaihd.net/steam/apps/{appid}/library_600x900.jpg")
}

/// 16:9 header image — the QML fallback when the portrait poster 404s (older
/// titles may lack `library_600x900`).
fn header_art_url(appid: u64) -> String {
    format!("https://steamcdn-a.akamaihd.net/steam/apps/{appid}/header.jpg")
}

/// Local portrait art served by the host's public `/art/{appid}` endpoint. Used
/// when Steam's CDN lacks library art for a (usually newer) title — the art is
/// still cached on the gaming PC. `base` is already trailing-slash-trimmed.
fn local_art_url(base: &str, appid: u64) -> String {
    format!("{base}/art/{appid}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Host base URL for tests — matches `config()`'s already-trimmed shape.
    const BASE: &str = "http://host:47995";

    fn lib(games: Value) -> Value {
        json!({ "games": games })
    }

    fn parse(out: &str) -> Value {
        serde_json::from_str(out).unwrap()
    }

    #[test]
    fn empty_library_is_ok_empty() {
        let out = parse(&normalize_library(&lib(json!([])), BASE, None, false));
        assert_eq!(out["status"], "ok");
        assert_eq!(out["recentlyPlayed"].as_array().unwrap().len(), 0);
        assert_eq!(out["allGames"].as_array().unwrap().len(), 0);
        assert!(out["runningAppid"].is_null());
        assert_eq!(out["streaming"], false);
    }

    #[test]
    fn missing_games_array_is_ok_empty() {
        let out = parse(&normalize_library(&json!({}), BASE, None, false));
        assert_eq!(out["status"], "ok");
        assert_eq!(out["allGames"].as_array().unwrap().len(), 0);
    }

    #[test]
    fn running_appid_passed_through() {
        let out = parse(&normalize_library(
            &lib(json!([
                { "appid": 730, "name": "CS2", "last_played": 0, "installed": true },
            ])),
            BASE,
            Some(730),
            false,
        ));
        assert_eq!(out["runningAppid"], 730);
        // And passes through on the empty-games path too.
        let empty = parse(&normalize_library(&json!({}), BASE, Some(440), false));
        assert_eq!(empty["runningAppid"], 440);
    }

    #[test]
    fn streaming_passed_through() {
        // streaming flows through on both the populated and empty-games paths.
        let out = parse(&normalize_library(
            &lib(json!([
                { "appid": 730, "name": "CS2", "last_played": 0, "installed": true },
            ])),
            BASE,
            None,
            true,
        ));
        assert_eq!(out["streaming"], true);
        let empty = parse(&normalize_library(&json!({}), BASE, None, true));
        assert_eq!(empty["streaming"], true);
    }

    #[test]
    fn all_games_sorted_by_name_and_art_built() {
        let out = parse(&normalize_library(
            &lib(json!([
                { "appid": 620, "name": "Portal 2", "last_played": 0, "installed": true },
                { "appid": 400, "name": "Portal", "last_played": 0, "installed": true },
            ])),
            BASE,
            None,
            false,
        ));
        let all = out["allGames"].as_array().unwrap();
        assert_eq!(all.len(), 2);
        // Sorted: "Portal" before "Portal 2".
        assert_eq!(all[0]["name"], "Portal");
        assert_eq!(all[1]["name"], "Portal 2");
        // Art URLs built off appid.
        assert_eq!(
            all[0]["art"],
            "https://steamcdn-a.akamaihd.net/steam/apps/400/library_600x900.jpg"
        );
        assert_eq!(
            all[0]["headerArt"],
            "https://steamcdn-a.akamaihd.net/steam/apps/400/header.jpg"
        );
        // localArt points at the host's public /art/{appid} endpoint.
        assert_eq!(all[0]["localArt"], "http://host:47995/art/400");
        assert_eq!(all[1]["localArt"], "http://host:47995/art/620");
    }

    #[test]
    fn recently_played_sorted_desc_and_filtered() {
        let out = parse(&normalize_library(
            &lib(json!([
                { "appid": 1, "name": "Old", "last_played": 100, "installed": true },
                { "appid": 2, "name": "Newest", "last_played": 300, "installed": true },
                { "appid": 3, "name": "Never", "last_played": 0, "installed": true },
                { "appid": 4, "name": "Mid", "last_played": 200, "installed": true },
            ])),
            BASE,
            None,
            false,
        ));
        let recent = out["recentlyPlayed"].as_array().unwrap();
        // Never-played dropped; rest by last_played desc.
        assert_eq!(recent.len(), 3);
        assert_eq!(recent[0]["name"], "Newest");
        assert_eq!(recent[1]["name"], "Mid");
        assert_eq!(recent[2]["name"], "Old");
    }

    #[test]
    fn uninstalled_games_excluded() {
        let out = parse(&normalize_library(
            &lib(json!([
                { "appid": 1, "name": "Installed", "last_played": 0, "installed": true },
                { "appid": 2, "name": "Downloading", "last_played": 0, "installed": false },
            ])),
            BASE,
            None,
            false,
        ));
        let all = out["allGames"].as_array().unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0]["name"], "Installed");
    }

    #[test]
    fn recently_played_capped() {
        // appids start at 1 — appid 0 is unaddressable and is now skipped, so use
        // real (non-zero) ids to exercise the cap with a full 30-game library.
        let many: Vec<Value> = (1..31)
            .map(|i| json!({ "appid": i, "name": format!("G{i}"), "last_played": 1000 + i, "installed": true }))
            .collect();
        let out = parse(&normalize_library(&lib(json!(many)), BASE, None, false));
        assert_eq!(
            out["recentlyPlayed"].as_array().unwrap().len(),
            RECENT_LIMIT
        );
        // allGames is uncapped.
        assert_eq!(out["allGames"].as_array().unwrap().len(), 30);
    }
}
