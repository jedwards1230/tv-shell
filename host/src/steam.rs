//! Steam library enumeration — pure Rust, cross-platform.
//!
//! Locates the Steam install root per-OS, reads `steamapps/libraryfolders.vdf`
//! to discover every library folder, then parses each `appmanifest_*.acf`
//! (Valve KeyValues) into a [`LibraryEntry`]. Runtime junk (Proton, the Steam
//! Linux Runtime, Steamworks redistributables) is filtered out so only real
//! games surface.
//!
//! Everything here is pure (no network, no daemon state), so the parser is
//! unit-tested on every platform against a checked-in `appmanifest` fixture.

use game_shell_protocol::LibraryEntry;
use keyvalues_parser::Vdf;
use std::path::{Path, PathBuf};

/// `StateFlags` bit indicating a fully-installed app (Steam's
/// `k_EAppStateFullyInstalled`). A manifest can exist for a game that's only
/// partially downloaded; we still report it but flag `installed = false`.
const STATE_FLAG_FULLY_INSTALLED: u64 = 4;

/// Known non-game appids: Proton builds, Steam Linux Runtime, Steamworks Common
/// Redistributables. These ship `appmanifest`s but are tooling, not games.
const RUNTIME_APPIDS: &[u32] = &[
    228980,  // Steamworks Common Redistributables
    1070560, // Steam Linux Runtime 1.0 (scout)
    1391110, // Steam Linux Runtime 2.0 (soldier)
    1493710, // Proton Experimental
    1628350, // Steam Linux Runtime 3.0 (sniper)
    2180100, // Proton Hotfix
    4183110, // (historical Proton/runtime appid)
    4628710, // (historical Proton/runtime appid)
];

/// Name prefixes that mark a manifest as runtime tooling rather than a game.
/// Lower-cased prefix match against the manifest `name`.
const RUNTIME_NAME_PREFIXES: &[&str] = &[
    "proton",
    "steam linux runtime",
    "steamworks common redistributables",
];

/// Return the candidate Steam install roots for the current OS, in priority
/// order. Only existing directories are kept by the caller.
fn steam_roots() -> Vec<PathBuf> {
    let home = home_dir();
    let mut roots = Vec::new();

    #[cfg(target_os = "linux")]
    {
        if let Some(h) = &home {
            roots.push(h.join(".steam/steam"));
            roots.push(h.join(".local/share/Steam"));
            // Flatpak Steam.
            roots.push(h.join(".var/app/com.valvesoftware.Steam/.local/share/Steam"));
        }
    }

    #[cfg(target_os = "macos")]
    {
        if let Some(h) = &home {
            roots.push(h.join("Library/Application Support/Steam"));
        }
    }

    #[cfg(target_os = "windows")]
    {
        // Registry would be more precise (HKCU\Software\Valve\Steam\SteamPath),
        // but the default install path covers the overwhelming majority and
        // keeps this dependency-free. A `STEAM_PATH` override is honored first.
        if let Ok(p) = std::env::var("STEAM_PATH") {
            roots.push(PathBuf::from(p));
        }
        roots.push(PathBuf::from(r"C:\Program Files (x86)\Steam"));
        roots.push(PathBuf::from(r"C:\Program Files\Steam"));
    }

    // Allow an explicit override on any OS (used by tests and unusual installs).
    if let Ok(p) = std::env::var("GAME_SHELL_STEAM_ROOT") {
        roots.insert(0, PathBuf::from(p));
    }

    let _ = &home; // silence unused warning on OSes with no home-based root.
    roots
}

/// Best-effort home directory without pulling in a crate (`$HOME` on
/// Unix, `%USERPROFILE%` on Windows).
fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
}

/// Enumerate all installed games across every Steam library folder.
///
/// Returns an empty list (not an error) when Steam isn't installed or no
/// manifests are found — the widget renders "no games" the same as an empty
/// library. Hard I/O errors on individual manifests are skipped, not fatal.
pub fn enumerate() -> Vec<LibraryEntry> {
    let Some(steam_root) = steam_roots().into_iter().find(|r| r.is_dir()) else {
        tracing::debug!("steam: no install root found");
        return Vec::new();
    };

    let mut entries = Vec::new();
    for lib in library_folders(&steam_root) {
        let steamapps = lib.join("steamapps");
        let Ok(read) = std::fs::read_dir(&steamapps) else {
            continue;
        };
        for dirent in read.flatten() {
            let path = dirent.path();
            if !is_appmanifest(&path) {
                continue;
            }
            match std::fs::read_to_string(&path) {
                Ok(text) => {
                    if let Some(entry) = parse_appmanifest(&text) {
                        if !is_runtime(&entry) {
                            entries.push(entry);
                        }
                    }
                }
                Err(e) => tracing::debug!("steam: read {} failed: {e}", path.display()),
            }
        }
    }

    // Stable order: name (case-insensitive) so the daemon's name-sorted rail is
    // deterministic even before it re-sorts.
    entries.sort_by_key(|e| e.name.to_lowercase());
    entries.dedup_by_key(|e| e.appid);
    entries
}

/// Is this a `appmanifest_<appid>.acf` file?
fn is_appmanifest(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
        return false;
    };
    name.starts_with("appmanifest_") && name.ends_with(".acf")
}

/// Resolve every library folder from `<steam_root>/steamapps/libraryfolders.vdf`.
/// The Steam root itself is always included (it's library 0). Parse failures
/// degrade to just the root.
fn library_folders(steam_root: &Path) -> Vec<PathBuf> {
    let mut libs = vec![steam_root.to_path_buf()];
    let vdf_path = steam_root.join("steamapps/libraryfolders.vdf");
    if let Ok(text) = std::fs::read_to_string(&vdf_path) {
        for p in parse_library_folders(&text) {
            let p = PathBuf::from(p);
            if !libs.contains(&p) {
                libs.push(p);
            }
        }
    }
    libs
}

/// Parse `libraryfolders.vdf` → the `"path"` of every library entry. Pure
/// function (unit-tested). The file shape is:
/// `"libraryfolders" { "0" { "path" "/a" ... } "1" { "path" "/b" ... } }`.
pub fn parse_library_folders(text: &str) -> Vec<String> {
    let Ok(vdf) = keyvalues_parser::parse(text).map(Vdf::from) else {
        return Vec::new();
    };
    let Some(root) = vdf.value.get_obj() else {
        return Vec::new();
    };
    let mut paths = Vec::new();
    // Each numeric-keyed child is a library; pull its "path".
    for values in root.values() {
        for v in values {
            if let Some(obj) = v.get_obj() {
                if let Some(path) = obj
                    .get("path")
                    .and_then(|vs| vs.first())
                    .and_then(|v| v.get_str())
                {
                    paths.push(path.to_string());
                }
            }
        }
    }
    paths
}

/// Parse one `appmanifest_*.acf` into a [`LibraryEntry`]. Returns `None` when
/// the manifest lacks an `appid` (the one field we can't synthesize). Pure
/// function — unit-tested against a fixture.
pub fn parse_appmanifest(text: &str) -> Option<LibraryEntry> {
    let vdf = keyvalues_parser::parse(text).map(Vdf::from).ok()?;
    // The root key is "AppState"; its value is the object with our fields.
    let obj = vdf.value.get_obj()?;

    let field = |key: &str| -> Option<String> {
        obj.get(key)
            .and_then(|vs| vs.first())
            .and_then(|v| v.get_str())
            .map(|s| s.to_string())
    };

    let appid = field("appid")?.trim().parse::<u32>().ok()?;
    let name = field("name").unwrap_or_else(|| format!("App {appid}"));
    let last_played = field("LastPlayed")
        .and_then(|s| s.trim().parse::<u64>().ok())
        .filter(|&t| t > 0);
    let size_on_disk = field("SizeOnDisk").and_then(|s| s.trim().parse::<u64>().ok());
    let state_flags = field("StateFlags")
        .and_then(|s| s.trim().parse::<u64>().ok())
        .unwrap_or(0);
    let installed = state_flags & STATE_FLAG_FULLY_INSTALLED != 0;

    Some(LibraryEntry {
        appid,
        name,
        last_played,
        size_on_disk,
        installed,
    })
}

/// Is this entry Steam runtime tooling (Proton, Linux Runtime, redistributables)
/// rather than a real game? Matches known appids and lower-cased name prefixes.
fn is_runtime(entry: &LibraryEntry) -> bool {
    if RUNTIME_APPIDS.contains(&entry.appid) {
        return true;
    }
    let name = entry.name.to_lowercase();
    RUNTIME_NAME_PREFIXES.iter().any(|p| name.starts_with(p))
}

#[cfg(test)]
mod tests {
    use super::*;

    // A trimmed but realistic appmanifest_*.acf (Valve KeyValues).
    const FIXTURE_ACF: &str = include_str!("../tests/fixtures/appmanifest_730.acf");

    #[test]
    fn parses_fixture_manifest() {
        let entry = parse_appmanifest(FIXTURE_ACF).expect("fixture parses");
        assert_eq!(entry.appid, 730);
        assert_eq!(entry.name, "Counter-Strike 2");
        assert_eq!(entry.last_played, Some(1_700_000_000));
        assert_eq!(entry.size_on_disk, Some(35_000_000_000));
        assert!(entry.installed); // StateFlags 4 → fully installed.
    }

    #[test]
    fn partially_installed_flagged() {
        let acf = r#""AppState"
{
    "appid"  "12345"
    "name"   "Half-Downloaded"
    "StateFlags"  "1026"
    "SizeOnDisk"  "100"
}"#;
        let entry = parse_appmanifest(acf).unwrap();
        assert_eq!(entry.appid, 12345);
        // 1026 & 4 == 0 → not fully installed.
        assert!(!entry.installed);
    }

    #[test]
    fn missing_appid_is_none() {
        let acf = r#""AppState" { "name" "No Appid" }"#;
        assert!(parse_appmanifest(acf).is_none());
    }

    #[test]
    fn zero_last_played_is_none() {
        let acf = r#""AppState"
{
    "appid"  "55"
    "name"   "Never Played"
    "LastPlayed"  "0"
    "StateFlags"  "4"
}"#;
        let entry = parse_appmanifest(acf).unwrap();
        assert_eq!(entry.last_played, None);
    }

    #[test]
    fn parses_library_folders_paths() {
        let vdf = r#""libraryfolders"
{
    "0"
    {
        "path"   "/home/user/.local/share/Steam"
        "label"  ""
    }
    "1"
    {
        "path"   "/mnt/games/SteamLibrary"
        "label"  ""
    }
}"#;
        let paths = parse_library_folders(vdf);
        assert_eq!(
            paths,
            vec![
                "/home/user/.local/share/Steam".to_string(),
                "/mnt/games/SteamLibrary".to_string(),
            ]
        );
    }

    #[test]
    fn runtime_junk_is_detected() {
        let proton = LibraryEntry {
            appid: 999999,
            name: "Proton 9.0".to_string(),
            last_played: None,
            size_on_disk: None,
            installed: true,
        };
        assert!(is_runtime(&proton));

        let redist = LibraryEntry {
            appid: 228980,
            name: "Steamworks Common Redistributables".to_string(),
            last_played: None,
            size_on_disk: None,
            installed: true,
        };
        assert!(is_runtime(&redist));

        let game = LibraryEntry {
            appid: 730,
            name: "Counter-Strike 2".to_string(),
            last_played: None,
            size_on_disk: None,
            installed: true,
        };
        assert!(!is_runtime(&game));
    }
}
