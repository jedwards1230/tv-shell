//! Shared wire types for the game-shell Steam-library feature.
//!
//! These types are the single source of truth for the JSON shape exchanged
//! between `game-shell-host` (the cross-platform sidecar that enumerates and
//! launches Steam games on the gaming PC) and `game-shell-input` (the daemon on
//! the TV client that proxies `GET /library` / `POST /launch` for the QML shell).
//!
//! Pure serde, no I/O — so both crates depend on it without dragging in either
//! one's heavier graph (axum on the host, evdev/cec on the daemon).

use serde::{Deserialize, Serialize};

/// One installed Steam game, derived from an `appmanifest_*.acf` file.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct LibraryEntry {
    /// Steam application id (the number in `appmanifest_<appid>.acf` and the
    /// `steam://rungameid/<appid>` launch URL).
    pub appid: u32,
    /// Display name (`name` field in the manifest).
    pub name: String,
    /// Last-played unix timestamp (`LastPlayed`), or `None` when never played /
    /// absent. Used to build the "Recently Played" rail.
    pub last_played: Option<u64>,
    /// On-disk size in bytes (`SizeOnDisk`), or `None` when absent.
    pub size_on_disk: Option<u64>,
    /// Fully-installed bit (`StateFlags & 4`). Only fully-installed games are
    /// launchable; partially-downloaded ones are reported but flagged.
    pub installed: bool,
}

/// Response body for `GET /library`.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
pub struct LibraryResponse {
    /// Installed games, real titles only (runtime junk filtered out host-side).
    pub games: Vec<LibraryEntry>,
}

/// Request body for `POST /launch`.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct LaunchRequest {
    /// The Steam appid to launch via `steam://rungameid/<appid>`.
    pub appid: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn library_response_roundtrips() {
        let resp = LibraryResponse {
            games: vec![LibraryEntry {
                appid: 730,
                name: "Counter-Strike 2".to_string(),
                last_played: Some(1_700_000_000),
                size_on_disk: Some(35_000_000_000),
                installed: true,
            }],
        };
        let json = serde_json::to_string(&resp).unwrap();
        let back: LibraryResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(resp, back);
    }

    #[test]
    fn launch_request_roundtrips() {
        let req = LaunchRequest { appid: 220 };
        let json = serde_json::to_string(&req).unwrap();
        assert_eq!(json, r#"{"appid":220}"#);
        let back: LaunchRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(req, back);
    }

    #[test]
    fn empty_library_default() {
        assert!(LibraryResponse::default().games.is_empty());
    }
}
