//! Plex "hubs" fetch for the home-screen Plex widget (On Deck + Recently Added).
//!
//! Mirrors [`crate::health`] (Sunshine): a stateless, cross-platform `reqwest`
//! handler served from `ipc.rs` like `list-apps`, not a Linux-only actor. The
//! shell's `PlexWidget` polls the `plex-hubs` IPC command and renders two rows
//! of poster cards — "On Deck" (continue-watching / up-next) and "Recently
//! Added".
//!
//! **Config via env** (set in `~/.config/game-shell/daemon.env`):
//! - `GAME_SHELL_PLEX_URL`   — server base, e.g. `https://plex.example.com`
//! - `GAME_SHELL_PLEX_TOKEN` — an `X-Plex-Token`
//!
//! The reply carries a `status` field from the shared [`crate::service_health`]
//! vocabulary (`disabled`/`ok`/`unreachable`/`error`) so the widget can tell a
//! down server apart from an empty library — a lightweight `/identity` probe
//! decides reachability before the hubs are fetched. When unconfigured the
//! command returns `{"status":"disabled",…}` so the widget collapses to nothing.
//! The token lives only in the daemon's environment; QML never reads it — the
//! daemon bakes ready-to-load tokenized art URLs into the reply, so the shell
//! just binds them to `Image.source`.
//!
//! The response *parser* ([`parse_items`]) is a pure function, unit-tested on
//! every platform; only the live fetch needs a reachable server.

use serde_json::{json, Value};
use std::time::Duration;

/// Max items returned per hub (the rows only show a handful at 4K; the cap keeps
/// the reply small and the art-prefetch bounded).
const ON_DECK_LIMIT: usize = 16;
const RECENT_LIMIT: usize = 24;

/// Resolve `(base_url, token)` from the environment, or `None` when the widget
/// is unconfigured. Trailing slash on the base is trimmed so URL joins are clean.
///
/// `pub(crate)` so [`crate::service_health`] reuses the exact same resolution
/// for its reachability probe — one source of truth for "is Plex configured?".
pub(crate) fn config() -> Option<(String, String)> {
    let base = std::env::var("GAME_SHELL_PLEX_URL").ok()?;
    let token = std::env::var("GAME_SHELL_PLEX_TOKEN").ok()?;
    let base = base.trim().trim_end_matches('/').to_string();
    let token = token.trim().to_string();
    if base.is_empty() || token.is_empty() {
        return None;
    }
    Some((base, token))
}

/// IPC entry point for `plex-hubs`. Returns
/// `{"status":<status>,"onDeck":[…],"recentlyAdded":[…]}` where `status` is the
/// shared [`ServiceStatus`] vocabulary.
///
/// A lightweight `/identity` reachability probe runs first: only on `Ok` are the
/// two hubs fetched, so a down server (`unreachable`/`error`) yields empty hubs
/// *with a status the widget can render*, rather than empty arrays that look
/// identical to a genuinely-empty-but-healthy library. Unconfigured ⇒
/// `disabled` (widget collapses).
pub async fn handle_plex_hubs() -> String {
    use crate::service_health::{probe_get, ServiceStatus};

    let Some((base, token)) = config() else {
        return json!({ "status": ServiceStatus::Disabled.as_str(), "onDeck": [], "recentlyAdded": [] })
            .to_string();
    };

    // Reachability gate — see the doc comment. `/identity` is small and serves
    // even unauthenticated, so its HTTP status is a clean reachability signal
    // (and still 503s through a proxy when the backend pod is down).
    let identity = format!("{base}/identity");
    let status = probe_get(
        &identity,
        &[("X-Plex-Token", &token), ("Accept", "application/json")],
    )
    .await;
    if status != ServiceStatus::Ok {
        return json!({ "status": status.as_str(), "onDeck": [], "recentlyAdded": [] }).to_string();
    }

    let (on_deck, recent) = tokio::join!(
        fetch_hub(&base, &token, "/library/onDeck", ON_DECK_LIMIT),
        fetch_hub(&base, &token, "/library/recentlyAdded", RECENT_LIMIT),
    );

    json!({
        "status": ServiceStatus::Ok.as_str(),
        "onDeck": on_deck,
        "recentlyAdded": recent,
    })
    .to_string()
}

/// Fetch one hub endpoint and normalize it. Any error (network/TLS/parse)
/// degrades to an empty list — a partial widget (one row) is better than none.
async fn fetch_hub(base: &str, token: &str, path: &str, limit: usize) -> Vec<Value> {
    match fetch_json(base, token, path).await {
        Ok(v) => parse_items(&v, base, token, limit),
        Err(e) => {
            tracing::debug!("plex hub {path} fetch failed: {e}");
            Vec::new()
        }
    }
}

/// Error type for a hub fetch: a transport failure (`reqwest`) or a body that
/// did not parse as JSON (`serde_json`). Both degrade to an empty hub.
#[derive(Debug)]
enum FetchError {
    Http(reqwest::Error),
    Json(serde_json::Error),
}

impl std::fmt::Display for FetchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FetchError::Http(e) => write!(f, "http: {e}"),
            FetchError::Json(e) => write!(f, "json: {e}"),
        }
    }
}

/// GET `{base}{path}` with the Plex token header and parse the JSON body. The
/// body is read as text and parsed via `serde_json` (the crate's `reqwest` has
/// no `json` feature, matching `health.rs`'s `.text()` usage).
/// `danger_accept_invalid_certs` keeps the fetch working if the server is ever
/// reached on its self-signed `:32400` directly rather than via a valid-cert
/// reverse proxy.
async fn fetch_json(base: &str, token: &str, path: &str) -> Result<Value, FetchError> {
    let url = format!("{base}{path}");
    let client = reqwest::Client::builder()
        .danger_accept_invalid_certs(true)
        .timeout(Duration::from_secs(6))
        .connect_timeout(Duration::from_secs(3))
        .build()
        .map_err(FetchError::Http)?;
    let body = client
        .get(&url)
        .header("Accept", "application/json")
        .header("X-Plex-Token", token)
        .send()
        .await
        .map_err(FetchError::Http)?
        .error_for_status()
        .map_err(FetchError::Http)?
        .text()
        .await
        .map_err(FetchError::Http)?;
    serde_json::from_str(&body).map_err(FetchError::Json)
}

/// Parse a Plex `MediaContainer` JSON body into a list of normalized widget
/// items (`{title,subtitle,kind,art,progress}`), capped at `limit`. Pure
/// function — no I/O — so it unit-tests anywhere. A body with no `Metadata`
/// array yields an empty list.
pub fn parse_items(root: &Value, base: &str, token: &str, limit: usize) -> Vec<Value> {
    let Some(meta) = root
        .get("MediaContainer")
        .and_then(|m| m.get("Metadata"))
        .and_then(|m| m.as_array())
    else {
        return Vec::new();
    };
    meta.iter()
        .take(limit)
        .map(|it| normalize(it, base, token))
        .collect()
}

/// Non-empty string field accessor.
fn str_field<'a>(v: &'a Value, key: &str) -> Option<&'a str> {
    v.get(key)
        .and_then(|x| x.as_str())
        .filter(|s| !s.is_empty())
}

/// Integer field accessor (Plex emits these as JSON numbers).
fn int_field(v: &Value, key: &str) -> Option<i64> {
    v.get(key).and_then(|x| x.as_i64())
}

/// Map one Plex metadata item to the widget's normalized shape. Titles/subtitles
/// are derived per media type so episodes read "Show / S3 · E1 · Title" and
/// movies read "Title / Year".
fn normalize(it: &Value, base: &str, token: &str) -> Value {
    let kind = str_field(it, "type").unwrap_or("");

    let (title, subtitle) = match kind {
        "episode" => {
            let show = str_field(it, "grandparentTitle")
                .or_else(|| str_field(it, "title"))
                .unwrap_or("Unknown")
                .to_string();
            let code = match (int_field(it, "parentIndex"), int_field(it, "index")) {
                (Some(s), Some(e)) => format!("S{s} · E{e}"),
                _ => str_field(it, "parentTitle").unwrap_or("").to_string(),
            };
            let ep_title = str_field(it, "title").unwrap_or("");
            let sub = match (code.is_empty(), ep_title.is_empty()) {
                (false, false) => format!("{code} · {ep_title}"),
                (false, true) => code,
                (true, false) => ep_title.to_string(),
                (true, true) => String::new(),
            };
            (show, sub)
        }
        "season" => {
            let show = str_field(it, "parentTitle")
                .or_else(|| str_field(it, "title"))
                .unwrap_or("Unknown")
                .to_string();
            (show, str_field(it, "title").unwrap_or("").to_string())
        }
        _ => {
            // movie / show / clip / album …
            let title = str_field(it, "title").unwrap_or("Unknown").to_string();
            let sub = int_field(it, "year")
                .map(|y| y.to_string())
                .unwrap_or_default();
            (title, sub)
        }
    };

    // Prefer a portrait poster so a row stays visually uniform even when it mixes
    // movies and episodes — for an episode that means the *series* poster
    // (grandparentThumb), not the 16:9 episode still.
    let art_path = match kind {
        "episode" => str_field(it, "grandparentThumb")
            .or_else(|| str_field(it, "parentThumb"))
            .or_else(|| str_field(it, "thumb")),
        "season" => str_field(it, "thumb").or_else(|| str_field(it, "parentThumb")),
        _ => str_field(it, "thumb"),
    };
    let art = art_path
        .map(|p| art_url(base, token, p))
        .unwrap_or_default();

    // Resume progress: On Deck movies/episodes carry viewOffset against duration.
    let progress = match (int_field(it, "viewOffset"), int_field(it, "duration")) {
        (Some(off), Some(dur)) if dur > 0 => (off as f64 / dur as f64).clamp(0.0, 1.0),
        _ => 0.0,
    };

    json!({
        "title": title,
        "subtitle": subtitle,
        "kind": kind,
        "art": art,
        "progress": progress,
    })
}

/// Build a tokenized, fixed-size poster URL via Plex's photo transcoder. A
/// consistent 300×450 keeps payloads small and every card the same aspect; the
/// transcoder falls back to the source image when it can't resize.
fn art_url(base: &str, token: &str, thumb: &str) -> String {
    let enc = pct_encode(thumb);
    format!(
        "{base}/photo/:/transcode?width=300&height=450&minSize=1&upscale=1&url={enc}&X-Plex-Token={token}"
    )
}

/// Percent-encode a path for use as a query-parameter value (RFC 3986
/// unreserved set passes through; everything else, including `/`, is escaped).
fn pct_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const BASE: &str = "https://plex.example";
    const TOKEN: &str = "TESTTOKEN";

    fn container(items: Value) -> Value {
        json!({ "MediaContainer": { "size": 1, "Metadata": items } })
    }

    #[test]
    fn empty_container_is_empty() {
        let v = json!({ "MediaContainer": { "size": 0 } });
        assert!(parse_items(&v, BASE, TOKEN, 10).is_empty());
    }

    #[test]
    fn movie_title_year_and_progress() {
        let v = container(json!([{
            "type": "movie",
            "title": "Skyfall",
            "year": 2012,
            "thumb": "/library/metadata/30543/thumb/1780293820",
            "viewOffset": 434643,
            "duration": 8590260,
        }]));
        let items = parse_items(&v, BASE, TOKEN, 10);
        assert_eq!(items.len(), 1);
        let it = &items[0];
        assert_eq!(it["title"], "Skyfall");
        assert_eq!(it["subtitle"], "2012");
        assert_eq!(it["kind"], "movie");
        // ~5% watched.
        let p = it["progress"].as_f64().unwrap();
        assert!(p > 0.04 && p < 0.06, "progress was {p}");
        // Tokenized transcode URL with the thumb percent-encoded.
        let art = it["art"].as_str().unwrap();
        assert!(art.starts_with("https://plex.example/photo/:/transcode?"));
        assert!(art.contains("%2Flibrary%2Fmetadata%2F30543%2Fthumb%2F1780293820"));
        assert!(art.ends_with("X-Plex-Token=TESTTOKEN"));
    }

    #[test]
    fn episode_uses_show_title_and_series_poster() {
        let v = container(json!([{
            "type": "episode",
            "title": "Once Upon a Time...",
            "grandparentTitle": "Dexter",
            "parentTitle": "Season 6",
            "parentIndex": 6,
            "index": 2,
            "thumb": "/library/metadata/28996/thumb/1779084520",
            "grandparentThumb": "/library/metadata/100/thumb/1",
        }]));
        let items = parse_items(&v, BASE, TOKEN, 10);
        let it = &items[0];
        assert_eq!(it["title"], "Dexter");
        assert_eq!(it["subtitle"], "S6 · E2 · Once Upon a Time...");
        // Uses the series poster, not the episode still.
        assert!(it["art"]
            .as_str()
            .unwrap()
            .contains("%2Flibrary%2Fmetadata%2F100%2Fthumb%2F1"));
        // No resume info → progress 0.
        assert_eq!(it["progress"].as_f64().unwrap(), 0.0);
    }

    #[test]
    fn season_uses_show_as_title() {
        let v = container(json!([{
            "type": "season",
            "title": "Season 3",
            "parentTitle": "Interview With The Vampire",
            "thumb": "/library/metadata/29859/thumb/1780469639",
        }]));
        let it = &parse_items(&v, BASE, TOKEN, 10)[0];
        assert_eq!(it["title"], "Interview With The Vampire");
        assert_eq!(it["subtitle"], "Season 3");
    }

    #[test]
    fn missing_metadata_array_is_empty() {
        let v = json!({ "MediaContainer": {} });
        assert!(parse_items(&v, BASE, TOKEN, 10).is_empty());
    }

    #[test]
    fn limit_caps_items() {
        let many: Vec<Value> = (0..50)
            .map(|i| json!({ "type": "movie", "title": format!("M{i}"), "thumb": "/t" }))
            .collect();
        let v = container(json!(many));
        assert_eq!(parse_items(&v, BASE, TOKEN, 5).len(), 5);
    }

    #[test]
    fn item_without_thumb_has_empty_art() {
        let v = container(json!([{ "type": "movie", "title": "No Art" }]));
        let it = &parse_items(&v, BASE, TOKEN, 10)[0];
        assert_eq!(it["art"], "");
    }
}
