//! Steam library proxy for the home-screen Steam widget.
//!
//! Mirrors [`crate::plex`]: a stateless, cross-platform handler served from
//! `ipc.rs` like `list-apps`, not a Linux-only actor. The shell's `SteamWidget`
//! polls the `steam-library` IPC command and renders poster cards for the
//! installed Steam games on the gaming PC; selecting one fires `steam-launch
//! <appid>` (also handled here) and then starts the existing Moonlight stream.
//!
//! The actual enumeration + launch live in `game-shell-host` — a thin
//! cross-platform sidecar that runs **on the gaming PC** (a *different machine*;
//! see `docs/HOST_SETUP.md`). This daemon module is an **HTTP client** to that
//! host: it proxies the host's `GET /library` / `GET /status` / `POST /launch` /
//! `POST /open-bpm` / `POST /quit` over the LAN via the reusable
//! [`crate::sidecar::Sidecar`] client. It does **not** spawn, supervise, or
//! restart the host — the host is deployed independently on the gaming PC.
//!
//! **Config** lives in `~/.config/game-shell/config.toml` under `[steam]`:
//! - `url`        — game-shell-host base, e.g. `http://192.0.2.1:47995`
//! - `token_file` — path to a `0600` file holding the host's bearer token
//!   (`GAME_SHELL_HOST_TOKEN`). The token lives only in the daemon's config;
//!   QML never reads it.
//!
//! The reply carries a `status` field from the shared [`crate::service_health`]
//! vocabulary (`disabled`/`ok`/`unreachable`/`error`) so the widget tells a down
//! host apart from an empty library. When unconfigured the command returns
//! `{"status":"disabled",…}` so the widget collapses to nothing.
//!
//! Host responses are deserialized into the shared [`game_shell_protocol`] types
//! (`LibraryResponse` / `StatusResponse`), so the daemon↔host JSON contract is
//! single-sourced and can't silently drift. The response *normalizer*
//! ([`normalize_library`]) is a pure function, unit-tested on every platform; only
//! the live fetch needs a reachable host.

use crate::service_health::ServiceStatus;
use crate::sidecar::Sidecar;
use game_shell_protocol::{LibraryEntry, LibraryResponse, StatusResponse};
use serde_json::{json, Value};

/// Recently-played rail cap. The rows only show a handful at 4K.
const RECENT_LIMIT: usize = 12;

/// `/status` is a tiny JSON; refuse a body advertising more than this (a
/// misconfigured/compromised host could otherwise stream a huge body into memory
/// — the request timeout alone doesn't cap size).
const MAX_STATUS_BODY: u64 = 64 * 1024;

/// `/library` is legitimately larger (one entry per game); 10 MiB covers ~1000
/// games while still refusing a rogue host's unbounded body.
const MAX_LIBRARY_BODY: u64 = 10 * 1024 * 1024;

/// Resolve the Steam sidecar (base URL + bearer token) from `[steam]` in
/// `config.toml`, or `None` when the widget is unconfigured.
///
/// `pub(crate)` so [`crate::service_health`] can reuse the same resolution for the
/// reachability probe — one source of truth for "is Steam configured?".
pub(crate) fn sidecar() -> Option<Sidecar> {
    let cfg = crate::daemon_config::global();
    let base = cfg.steam.url.as_deref()?;
    // steam_token() is Result; startup validate() already vetted token files, so a
    // hard error or missing token here just means "widget not configured".
    let token = cfg.steam_token().ok().flatten()?;
    Sidecar::from_parts(base, &token)
}

/// IPC entry point for `steam-library`. Returns
/// `{"status":<status>,"recentlyPlayed":[…],"allGames":[…]}` where `status` is
/// the shared [`crate::service_health::ServiceStatus`] vocabulary.
///
/// A lightweight `GET /status` reachability probe runs first; only on `Ok` is the
/// library fetched, so a down host yields empty rails *with a status the widget
/// can render*. Unconfigured ⇒ `disabled` (widget collapses).
pub async fn handle_steam_library() -> String {
    let Some(sc) = sidecar() else {
        return empty_library(ServiceStatus::Disabled);
    };

    // Reachability gate — `GET /status` is small and authenticated, so its HTTP
    // status classifies cleanly (401 → Error/bad token, 5xx → Unreachable). We
    // also read its body to capture `running_appid` (the foreground game on the
    // host) and `streaming` (an active Moonlight/Sunshine stream) so the widget
    // can badge the "Playing" card and reflect stream state without a second
    // request.
    let (probe, running_appid, streaming) = fetch_status(&sc).await;
    if probe != ServiceStatus::Ok {
        return empty_library(probe);
    }

    match sc
        .get_json::<LibraryResponse>("/library", MAX_LIBRARY_BODY)
        .await
    {
        Ok(resp) => normalize_library(&resp, sc.base(), running_appid, streaming),
        Err(e) => {
            tracing::debug!("steam-library fetch failed: {e}");
            empty_library(ServiceStatus::Unreachable)
        }
    }
}

/// Empty-rails reply for a non-`Ok` library state (`disabled`/`unreachable`/
/// `error`): no running game, not streaming. The shape matches the populated
/// reply so the widget binds the same fields either way.
fn empty_library(status: ServiceStatus) -> String {
    json!({
        "status": status.as_str(),
        "recentlyPlayed": [],
        "allGames": [],
        "runningAppid": Value::Null,
        "streaming": false,
    })
    .to_string()
}

/// `GET /status` on the sidecar: returns the reachability classification plus the
/// foreground-game id and stream flag parsed from the same response (one request
/// serves both the probe and the metadata the widget badges with). The body is
/// deserialized into [`StatusResponse`]; a too-large or unparseable body degrades
/// to `(status, None, false)` so the caller still gets a clean status to surface.
/// `running_appid` 0 is treated as "nothing running" (an unaddressable id).
async fn fetch_status(sc: &Sidecar) -> (ServiceStatus, Option<u32>, bool) {
    let (status, body) = sc.get_classified("/status", MAX_STATUS_BODY).await;
    if status != ServiceStatus::Ok {
        return (status, None, false);
    }
    match body.and_then(|b| serde_json::from_str::<StatusResponse>(&b).ok()) {
        Some(s) => (status, s.running_appid.filter(|&id| id != 0), s.streaming),
        None => (status, None, false),
    }
}

/// IPC entry point for `steam-launch <appid>`. POSTs `{appid}` to the host's
/// `/launch` and returns `ok` / `error:*`. The Moonlight stream start stays in
/// QML — this only kicks off the game on the host.
pub async fn handle_steam_launch(appid: u32) -> String {
    let Some(sc) = sidecar() else {
        return crate::protocol::resp_error("steam not configured");
    };
    match sc.post("/launch", Some(&launch_body(appid))).await {
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
    let Some(sc) = sidecar() else {
        return json!({
            "status": ServiceStatus::Disabled.as_str(),
            "reason": "steam not configured",
        })
        .to_string();
    };
    match sc.post("/open-bpm", None).await {
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

/// IPC entry point for `steam-quit <appid>`. POSTs `{appid}` to the host's `/quit`
/// to gracefully terminate the running game (SIGTERM to its process group — like
/// Steam's Stop button), and returns a compact-JSON status object
/// (`{"status":"ok"}` / `{"status":"error","reason":…}`). Mirrors
/// [`handle_steam_bigpicture`]; the Moonlight stream close stays in QML. Degrades
/// gracefully when the host is unconfigured (`disabled`) or unreachable (`error`).
pub async fn handle_steam_quit(appid: u32) -> String {
    let Some(sc) = sidecar() else {
        return json!({
            "status": ServiceStatus::Disabled.as_str(),
            "reason": "steam not configured",
        })
        .to_string();
    };
    match sc.post("/quit", Some(&launch_body(appid))).await {
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

/// The `{appid}` POST body for `/launch` and `/quit`, built from the shared
/// [`game_shell_protocol::LaunchRequest`] so the request shape is single-sourced
/// with the host (which deserializes the same type). Serializes to `{"appid":N}`.
fn launch_body(appid: u32) -> Value {
    serde_json::to_value(game_shell_protocol::LaunchRequest { appid })
        .expect("LaunchRequest always serializes")
}

/// Normalize a [`LibraryResponse`] into the widget's two rails: `recentlyPlayed`
/// (sorted by `last_played` desc, top [`RECENT_LIMIT`]) and `allGames` (sorted by
/// name). `running_appid` (the host's foreground game, or `None`) is passed
/// through to the reply's `runningAppid` so the widget can badge the "Playing"
/// card; `streaming` (whether a stream is active on the host) is passed through to
/// `streaming`. `base` is the host base URL (already trailing-slash-trimmed by the
/// [`Sidecar`]) used to build each card's `localArt` fallback URL
/// (`{base}/art/{appid}`). Pure function — no I/O — so it unit-tests anywhere. An
/// empty library yields empty rails (but `status:ok`, matching an
/// installed-but-empty library).
pub fn normalize_library(
    resp: &LibraryResponse,
    base: &str,
    running_appid: Option<u32>,
    streaming: bool,
) -> String {
    let running = running_appid.map(Value::from).unwrap_or(Value::Null);

    // Project each entry to the widget shape and keep only installed games (a
    // partially-downloaded title isn't launchable yet). `card` returns None for a
    // zero appid (an unaddressable entry) so it's skipped rather than emitted as a
    // broken card.
    let mut cards: Vec<Value> = resp
        .games
        .iter()
        .filter(|e| e.installed)
        .filter_map(|e| card(e, base))
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

/// Map one host [`LibraryEntry`] to the widget card shape `{appid,name,art,
/// localArt,headerArt,lastPlayed}`. The `art`/`headerArt` poster + header URLs are
/// built off the appid against Steam's public CDN (no token), and `localArt`
/// points at the host's own `/art/{appid}` endpoint (`{base}/art/{appid}`) which
/// serves the on-disk cache art for titles the CDN is missing — QML uses it as a
/// fallback between `art` and `headerArt`. All three bind to `Image.source`.
fn card(entry: &LibraryEntry, base: &str) -> Option<Value> {
    // A zero appid can't address Steam's CDN or our /art endpoint, so skip the
    // entry (filter_map drops the None) rather than emit a broken card pointing at
    // `.../apps/0/...`.
    let appid = entry.appid;
    if appid == 0 {
        return None;
    }
    Some(json!({
        "appid": appid,
        "name": entry.name,
        "art": library_art_url(appid),
        "localArt": local_art_url(base, appid),
        "headerArt": header_art_url(appid),
        "lastPlayed": entry.last_played,
    }))
}

/// Portrait library poster (600×900). Verified HTTP 200 on Steam's CDN.
fn library_art_url(appid: u32) -> String {
    format!("https://steamcdn-a.akamaihd.net/steam/apps/{appid}/library_600x900.jpg")
}

/// 16:9 header image — the QML fallback when the portrait poster 404s (older
/// titles may lack `library_600x900`).
fn header_art_url(appid: u32) -> String {
    format!("https://steamcdn-a.akamaihd.net/steam/apps/{appid}/header.jpg")
}

/// Local portrait art served by the host's public `/art/{appid}` endpoint. Used
/// when Steam's CDN lacks library art for a (usually newer) title — the art is
/// still cached on the gaming PC. `base` is already trailing-slash-trimmed.
fn local_art_url(base: &str, appid: u32) -> String {
    format!("{base}/art/{appid}")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Host base URL for tests — matches the [`Sidecar`]'s already-trimmed shape.
    const BASE: &str = "http://host:47995";

    fn entry(appid: u32, name: &str, last_played: Option<u64>, installed: bool) -> LibraryEntry {
        LibraryEntry {
            appid,
            name: name.to_string(),
            last_played,
            size_on_disk: None,
            installed,
        }
    }

    fn resp(games: Vec<LibraryEntry>) -> LibraryResponse {
        LibraryResponse { games }
    }

    fn parse(out: &str) -> Value {
        serde_json::from_str(out).unwrap()
    }

    #[test]
    fn empty_library_is_ok_empty() {
        let out = parse(&normalize_library(&resp(vec![]), BASE, None, false));
        assert_eq!(out["status"], "ok");
        assert_eq!(out["recentlyPlayed"].as_array().unwrap().len(), 0);
        assert_eq!(out["allGames"].as_array().unwrap().len(), 0);
        assert!(out["runningAppid"].is_null());
        assert_eq!(out["streaming"], false);
    }

    #[test]
    fn default_library_is_ok_empty() {
        // A default (no games) LibraryResponse — what a `{}` body deserializes to —
        // yields empty rails with status ok.
        let out = parse(&normalize_library(
            &LibraryResponse::default(),
            BASE,
            None,
            false,
        ));
        assert_eq!(out["status"], "ok");
        assert_eq!(out["allGames"].as_array().unwrap().len(), 0);
    }

    #[test]
    fn running_appid_passed_through() {
        let out = parse(&normalize_library(
            &resp(vec![entry(730, "CS2", Some(0), true)]),
            BASE,
            Some(730),
            false,
        ));
        assert_eq!(out["runningAppid"], 730);
        // And passes through on the empty-games path too.
        let empty = parse(&normalize_library(
            &LibraryResponse::default(),
            BASE,
            Some(440),
            false,
        ));
        assert_eq!(empty["runningAppid"], 440);
    }

    #[test]
    fn streaming_passed_through() {
        // streaming flows through on both the populated and empty-games paths.
        let out = parse(&normalize_library(
            &resp(vec![entry(730, "CS2", Some(0), true)]),
            BASE,
            None,
            true,
        ));
        assert_eq!(out["streaming"], true);
        let empty = parse(&normalize_library(
            &LibraryResponse::default(),
            BASE,
            None,
            true,
        ));
        assert_eq!(empty["streaming"], true);
    }

    #[test]
    fn all_games_sorted_by_name_and_art_built() {
        let out = parse(&normalize_library(
            &resp(vec![
                entry(620, "Portal 2", Some(0), true),
                entry(400, "Portal", Some(0), true),
            ]),
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
            &resp(vec![
                entry(1, "Old", Some(100), true),
                entry(2, "Newest", Some(300), true),
                entry(3, "Never", Some(0), true),
                entry(4, "Mid", Some(200), true),
            ]),
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
            &resp(vec![
                entry(1, "Installed", Some(0), true),
                entry(2, "Downloading", Some(0), false),
            ]),
            BASE,
            None,
            false,
        ));
        let all = out["allGames"].as_array().unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0]["name"], "Installed");
    }

    #[test]
    fn zero_appid_skipped() {
        // A zero appid is unaddressable (no CDN/`/art` URL), so it's dropped even
        // when installed.
        let out = parse(&normalize_library(
            &resp(vec![
                entry(0, "Bogus", Some(0), true),
                entry(10, "Real", Some(0), true),
            ]),
            BASE,
            None,
            false,
        ));
        let all = out["allGames"].as_array().unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0]["name"], "Real");
    }

    #[test]
    fn recently_played_capped() {
        // appids start at 1 — appid 0 is unaddressable and is skipped, so use real
        // (non-zero) ids to exercise the cap with a full 30-game library.
        let many: Vec<LibraryEntry> = (1..31)
            .map(|i| entry(i, &format!("G{i}"), Some(1000 + i as u64), true))
            .collect();
        let out = parse(&normalize_library(&resp(many), BASE, None, false));
        assert_eq!(
            out["recentlyPlayed"].as_array().unwrap().len(),
            RECENT_LIMIT
        );
        // allGames is uncapped.
        assert_eq!(out["allGames"].as_array().unwrap().len(), 30);
    }
}
