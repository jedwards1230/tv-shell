//! Runtime fetch + caching for the upstream SDL_GameControllerDB.
//!
//! The daemon ships a small `assets/gamecontrollerdb.txt` baseline (compiled
//! in via `include_str!`). This module extends that with a *cached upstream*
//! copy fetched from GitHub at startup (and on demand via `controllerdb-refresh`
//! IPC). The runtime match set is:
//!
//!   cached upstream ∪ bundled baseline ∪ GAME_SHELL_GAMECONTROLLERDB env override
//!
//! The bundled baseline is always present (offline floor) so the daemon never
//! starts with zero known pads.
//!
//! Cache location: `~/.local/share/game-shell/gamecontrollerdb.txt` alongside a
//! `gamecontrollerdb.last_updated` timestamp (Unix seconds, plain text).
//!
//! This module is **cross-platform** (no Linux-only imports) so it compiles and
//! is unit-tested on macOS.

use crate::device::ControllerDb;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

/// The upstream SDL_GameControllerDB URL.
pub const UPSTREAM_URL: &str =
    "https://raw.githubusercontent.com/mdqinc/SDL_GameControllerDB/master/gamecontrollerdb.txt";

/// Maximum download size guard (8 MiB — the file is currently ~1 MiB).
const MAX_DOWNLOAD_BYTES: u64 = 8 * 1024 * 1024;

/// The bundled baseline DB (same file referenced by `device::load_db`).
const BUNDLED_DB: &str = include_str!("../assets/gamecontrollerdb.txt");

/// Status reported by `controllerdb-status` IPC.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DbStatus {
    /// How the current DB was loaded: `"upstream_cache"`, `"bundled_baseline"`,
    /// or `"env_override"` (the last one indicates the env var extended the set).
    pub source: String,
    /// Total number of recognized (vendor, product) pairs.
    pub entry_count: usize,
    /// Unix timestamp (seconds) of the last successful upstream download, or 0
    /// when the cache has never been populated.
    pub last_downloaded: u64,
    /// The upstream URL this daemon fetches from.
    pub upstream_url: String,
    /// Error message from the last failed refresh attempt, or `null` / absent
    /// when the last refresh succeeded.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Return the XDG cache directory for game-shell state files.
///
/// `~/.local/share/game-shell/`
pub fn state_dir() -> PathBuf {
    let base = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/tmp"));
    base.join(".local/share/game-shell")
}

/// Path to the cached upstream DB file.
pub fn cache_db_path() -> PathBuf {
    state_dir().join("gamecontrollerdb.txt")
}

/// Path to the last-updated timestamp file (plain-text Unix seconds).
fn cache_ts_path() -> PathBuf {
    state_dir().join("gamecontrollerdb.last_updated")
}

/// Read the last-updated timestamp from the cache, returning 0 on any error.
pub fn read_last_downloaded() -> u64 {
    std::fs::read_to_string(cache_ts_path())
        .ok()
        .and_then(|s| s.trim().parse::<u64>().ok())
        .unwrap_or(0)
}

/// Write a fresh timestamp to the cache metadata file. Errors are logged and
/// swallowed — a missing timestamp just means the UI shows "never".
fn write_last_downloaded(ts: u64) {
    if let Err(e) = std::fs::write(cache_ts_path(), ts.to_string()) {
        tracing::warn!("failed to write controllerdb timestamp: {e}");
    }
}

/// Current Unix timestamp in seconds.
fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Load the controller DB from all sources (cached upstream ∪ bundled baseline
/// ∪ env override) in offline-safe order. Also returns the `source` label for
/// the status reply.
///
/// This is a pure, synchronous helper — the async wrapper `fetch_and_update`
/// is used for network fetches.
pub fn load_merged_db() -> (ControllerDb, String) {
    let mut db = ControllerDb::parse(BUNDLED_DB);
    let mut source = "bundled_baseline".to_string();

    // Layer in the cached upstream DB, if present.
    if let Ok(text) = std::fs::read_to_string(cache_db_path()) {
        if !text.trim().is_empty() {
            let upstream = ControllerDb::parse(&text);
            if !upstream.is_empty() {
                db.merge(&upstream);
                source = "upstream_cache".to_string();
            }
        }
    }

    // Layer in the operator override, if any.
    if let Some(path) = std::env::var_os("GAME_SHELL_GAMECONTROLLERDB") {
        if let Ok(text) = std::fs::read_to_string(&path) {
            let extra = ControllerDb::parse(&text);
            if !extra.is_empty() {
                db.merge(&extra);
                source = "env_override".to_string();
            }
        }
    }

    (db, source)
}

/// Fetch the upstream DB over HTTPS, write it to the cache, and return the
/// refreshed `ControllerDb` + the raw text.
///
/// On any error, returns `Err` with a human-readable message; the caller
/// keeps the existing DB and surfaces the error in the status reply. The
/// download is size-capped at [`MAX_DOWNLOAD_BYTES`].
#[cfg(not(target_os = "linux"))]
pub async fn fetch_upstream() -> Result<String, String> {
    // On non-Linux the daemon doesn't run for real, but the function must
    // compile. Return a stub error so tests can verify the function exists.
    Err("fetch only supported on Linux (reqwest not wired on this platform)".into())
}

#[cfg(target_os = "linux")]
pub async fn fetch_upstream() -> Result<String, String> {
    use reqwest::Client;

    let client = Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| format!("failed to build HTTP client: {e}"))?;

    let resp = client
        .get(UPSTREAM_URL)
        .send()
        .await
        .map_err(|e| format!("fetch failed: {e}"))?;

    if !resp.status().is_success() {
        return Err(format!("upstream returned HTTP {}", resp.status()));
    }

    // Cap the download size to guard against a malicious/corrupted response.
    let bytes = resp
        .bytes()
        .await
        .map_err(|e| format!("reading body failed: {e}"))?;

    if bytes.len() as u64 > MAX_DOWNLOAD_BYTES {
        return Err(format!(
            "download too large ({} bytes > {} limit)",
            bytes.len(),
            MAX_DOWNLOAD_BYTES
        ));
    }

    let text = String::from_utf8_lossy(&bytes).into_owned();

    // Sanity-check: must parse into a non-empty DB (guards against e.g. an
    // HTML 404 page that happens to return a 2xx redirect).
    let parsed = ControllerDb::parse(&text);
    if parsed.is_empty() {
        return Err("downloaded file contained no recognized controller entries".into());
    }

    tracing::info!(
        "controllerdb: fetched {} entries from upstream",
        parsed.len()
    );
    Ok(text)
}

/// Fetch the upstream DB, persist it to the cache, and return the updated DB
/// text. On failure returns `Err` and leaves the cache unchanged.
pub async fn refresh() -> Result<String, String> {
    let text = fetch_upstream().await?;

    // Persist to the cache (create parent directory if needed).
    let cache_path = cache_db_path();
    if let Some(parent) = cache_path.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            return Err(format!("failed to create cache dir: {e}"));
        }
    }
    std::fs::write(&cache_path, &text).map_err(|e| format!("failed to write cache file: {e}"))?;
    write_last_downloaded(now_unix());

    Ok(text)
}

/// Build the compact JSON object for `controllerdb-status`.
pub fn status_json(
    db: &ControllerDb,
    source: &str,
    last_downloaded: u64,
    error: Option<&str>,
) -> String {
    let status = DbStatus {
        source: source.to_string(),
        entry_count: db.len(),
        last_downloaded,
        upstream_url: UPSTREAM_URL.to_string(),
        error: error.map(str::to_string),
    };
    serde_json::to_string(&status).expect("controllerdb status serialize")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_serializes_without_error_field_when_none() {
        let db = ControllerDb::parse(
            "030000005e0400008e02000010010000,Xbox 360 Controller,platform:Linux,\n",
        );
        let json = status_json(&db, "upstream_cache", 1700000000, None);
        let v: serde_json::Value = serde_json::from_str(&json).expect("valid json");
        assert_eq!(v["source"], "upstream_cache");
        assert_eq!(v["entryCount"], 1);
        assert_eq!(v["lastDownloaded"], 1700000000u64);
        assert_eq!(v["upstreamUrl"], UPSTREAM_URL);
        // `error` key must be absent when None (skip_serializing_if).
        assert!(v.get("error").is_none());
    }

    #[test]
    fn status_includes_error_field_when_some() {
        let db = ControllerDb::default();
        let json = status_json(&db, "bundled_baseline", 0, Some("network timeout"));
        let v: serde_json::Value = serde_json::from_str(&json).expect("valid json");
        assert_eq!(v["error"], "network timeout");
        assert_eq!(v["entryCount"], 0);
    }

    #[test]
    fn load_merged_db_always_has_baseline() {
        // Without a cache file or env override, the merged DB must still
        // contain the bundled baseline (Xbox 360 is the canonical test entry).
        std::env::remove_var("GAME_SHELL_GAMECONTROLLERDB");
        let (db, _source) = load_merged_db();
        // Xbox 360: vendor=0x045e, product=0x028e
        assert!(
            db.is_known(0x045e, 0x028e),
            "bundled baseline must always be present"
        );
    }
}
