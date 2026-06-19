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

/// The currently-running Steam appid, or `None` when nothing is running.
///
/// The detection source is OS-specific because Steam exposes the running game
/// differently per platform:
///
/// - **Linux**: scan `/proc/*/cmdline` for Steam's `reaper` launcher process,
///   whose argv is `reaper SteamLaunch AppId=<appid> -- <proton/...> <game.exe>`.
///   Linux Steam does NOT write `RunningAppID` to `registry.vdf` (that's a
///   Windows-only field), so the process scan is the only reliable signal.
/// - **Windows**: read `RunningAppID` from `registry.vdf` (where it IS correct).
/// - **macOS**: not wired yet — returns `None` (out of scope).
///
/// Returns `None` whenever nothing matches — the "Playing" badge then shows no
/// running game.
pub fn running_appid() -> Option<u32> {
    #[cfg(target_os = "linux")]
    {
        running_appid_linux()
    }

    #[cfg(target_os = "windows")]
    {
        running_appid_registry()
    }

    #[cfg(not(any(target_os = "linux", target_os = "windows")))]
    {
        // macOS (and any other OS): no running-game detection wired yet. Steam
        // on mac keeps a `registry.vdf`, but without a Linux-style `reaper`
        // process or a Windows registry key it's out of scope for now.
        None
    }
}

/// Linux running-game detector: scan `/proc/*/cmdline` for Steam's `reaper`
/// launcher and return its `AppId=<digits>`. `/proc` is read with pure std (no
/// new crates); each `cmdline` is NUL-separated argv. Unreadable entries
/// (permissions, races where the pid exits mid-scan) are skipped.
#[cfg(target_os = "linux")]
fn running_appid_linux() -> Option<u32> {
    let proc = std::fs::read_dir("/proc").ok()?;
    for dirent in proc.flatten() {
        // Only numeric (pid) directories carry a cmdline worth reading.
        let name = dirent.file_name();
        let Some(name) = name.to_str() else { continue };
        if !name.bytes().all(|b| b.is_ascii_digit()) {
            continue;
        }
        let Ok(raw) = std::fs::read(dirent.path().join("cmdline")) else {
            continue;
        };
        // cmdline is NUL-separated; split, drop the trailing empty field, and
        // keep only valid UTF-8 args (Steam's launcher args are plain ASCII).
        let args: Vec<&str> = raw
            .split(|&b| b == 0)
            .filter(|s| !s.is_empty())
            .filter_map(|s| std::str::from_utf8(s).ok())
            .collect();
        if let Some(appid) = steam_appid_from_argv(&args) {
            return Some(appid);
        }
    }
    None
}

/// Pure argv → appid extractor for the Linux `reaper` launcher. A running Steam
/// game has a process whose args contain the token `SteamLaunch` followed by an
/// `AppId=<digits>` arg, e.g.
/// `reaper SteamLaunch AppId=2215200 -- <proton/...> <game.exe>`.
/// Returns the parsed appid, or `None` when the marker/appid is absent or
/// unparseable. Unit-tested without touching `/proc`.
#[cfg(any(target_os = "linux", test))]
fn steam_appid_from_argv(args: &[&str]) -> Option<u32> {
    if !args.contains(&"SteamLaunch") {
        return None;
    }
    args.iter()
        .find_map(|a| a.strip_prefix("AppId="))
        .and_then(|id| id.parse::<u32>().ok())
}

/// Windows running-game detector: read `RunningAppID` from `registry.vdf`.
///
/// On Linux this field is never written, so this path is Windows-only (and the
/// `GAME_SHELL_STEAM_REGISTRY` override, used by tests). The registry-DWORD case
/// (`HKCU\Software\Valve\Steam\RunningAppID`) is left as a later refinement.
// TODO(windows): the authoritative source is the actual Windows registry DWORD
// `HKCU\Software\Valve\Steam\RunningAppID`; reading it needs the `winreg` crate
// (or a `reg query` shell-out). For now we parse the on-disk `registry.vdf`,
// which Steam also maintains. (The pure parser `parse_running_appid` is exercised
// by tests on every OS; this Windows-only wiring is not.)
#[cfg(target_os = "windows")]
fn running_appid_registry() -> Option<u32> {
    let path = registry_vdf_path()?;
    let text = std::fs::read_to_string(&path).ok()?;
    parse_running_appid(&text)
}

/// Resolve the path to Steam's `registry.vdf` (Windows running-game path only).
#[cfg(target_os = "windows")]
fn registry_vdf_path() -> Option<PathBuf> {
    // Test/override hook: point at a fixture.
    if let Ok(p) = std::env::var("GAME_SHELL_STEAM_REGISTRY") {
        return Some(PathBuf::from(p));
    }
    // Default install; a STEAM_PATH override refines this. registry.vdf sits at
    // the Steam root.
    let root = std::env::var("STEAM_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(r"C:\Program Files (x86)\Steam"));
    Some(root.join("registry.vdf"))
}

/// Parse the `RunningAppID` out of a `registry.vdf` body (Windows source only).
/// Pure function (unit-tested). The file is Valve KeyValues:
/// `"Registry" { "HKCU" { "Software" { "Valve" { "Steam" { "RunningAppID" "<n>" … } } } } }`.
/// Returns `None` for a missing key or a `0`/non-numeric value.
#[cfg(any(target_os = "windows", test))]
pub fn parse_running_appid(text: &str) -> Option<u32> {
    let vdf = keyvalues_parser::parse(text).map(Vdf::from).ok()?;
    // Walk the fixed nesting Registry > HKCU > Software > Valve > Steam.
    let mut obj = vdf.value.get_obj()?;
    for key in ["HKCU", "Software", "Valve", "Steam"] {
        obj = obj.get(key).and_then(|vs| vs.first())?.get_obj()?;
    }
    let raw = obj
        .get("RunningAppID")
        .and_then(|vs| vs.first())
        .and_then(|v| v.get_str())?;
    raw.trim().parse::<u32>().ok().filter(|&id| id != 0)
}

/// Whether a Moonlight/Sunshine session is currently live on this host —
/// **active OR resumable** (suspended).
///
/// The signal is Sunshine's own GameStream `serverinfo` state, the same thing the
/// Moonlight client reads: `<state>SUNSHINE_SERVER_BUSY</state>` means an app
/// session is running and streamable — whether a client is actively connected
/// (encoding) or the session is paused/resumable after a client disconnected.
/// `…_FREE` means idle.
///
/// This deliberately replaces the earlier NVENC encode-session probe, which only
/// saw an *actively encoding* stream and reported `false` for a resumable session
/// the user can still reconnect to — the mismatch the session indicator showed
/// ("No session" while the laptop's Moonlight still listed BPM as resumable). It
/// is also GPU-agnostic (no `nvidia-smi`), so it works on AMD/Intel hosts.
///
/// Best-effort: a minimal sync HTTP GET to Sunshine's HTTP port (default 47989,
/// override via `GAME_SHELL_SUNSHINE_PORT`) on loopback — the *unpaired*
/// `serverinfo` returns basic state (incl. `<state>`) with no client cert.
/// Sunshine down / unreachable / any error ⇒ `false` (never fail the endpoint).
pub fn streaming() -> bool {
    sunshine_server_busy()
}

/// Sunshine GameStream HTTP port. Default 47989 (Sunshine's GameStream base);
/// override with `GAME_SHELL_SUNSHINE_PORT` for a non-default install.
fn sunshine_http_port() -> u16 {
    std::env::var("GAME_SHELL_SUNSHINE_PORT")
        .ok()
        .and_then(|p| p.trim().parse::<u16>().ok())
        .unwrap_or(47989)
}

/// Query Sunshine's unpaired `serverinfo` on loopback and report whether a
/// session is live (active or resumable). Dependency-free sync HTTP/1.0 GET (this
/// runs inside `spawn_blocking`), so the host crate stays HTTP-client-free.
/// Any connect/read failure ⇒ `false`.
fn sunshine_server_busy() -> bool {
    use std::io::{Read, Write};
    use std::net::TcpStream;
    use std::time::Duration;

    let addr = format!("127.0.0.1:{}", sunshine_http_port());
    let Ok(mut stream) = TcpStream::connect(&addr) else {
        return false;
    };
    let _ = stream.set_read_timeout(Some(Duration::from_secs(3)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(3)));

    // HTTP/1.0 + `Connection: close` so the server closes after the response and
    // `read_to_string` returns on EOF. The `uniqueid` is required by the
    // GameStream contract but any value works for the unpaired basic info.
    let req = "GET /serverinfo?uniqueid=0123456789ABCDEF HTTP/1.0\r\n\
               Host: localhost\r\nConnection: close\r\n\r\n";
    if stream.write_all(req.as_bytes()).is_err() {
        return false;
    }
    // Ignore a read error: a partial body that already carries `<state>` is still
    // conclusive, and a timeout leaves whatever arrived in `body`.
    let mut body = String::new();
    let _ = stream.read_to_string(&mut body);
    serverinfo_is_busy(&body)
}

/// True when a Sunshine `serverinfo` response reports a live (active or
/// resumable) session: `<state>SUNSHINE_SERVER_BUSY</state>`. Idle is
/// `…_SERVER_FREE`. Pure function (unit-tested).
fn serverinfo_is_busy(body: &str) -> bool {
    body.contains("SUNSHINE_SERVER_BUSY")
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

    // A realistic registry.vdf with RunningAppID set (Valve KeyValues).
    const FIXTURE_REGISTRY: &str = include_str!("../tests/fixtures/registry.vdf");

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
    fn parses_running_appid_from_fixture() {
        // The fixture has RunningAppID 730 nested under Registry/HKCU/Software/Valve/Steam.
        assert_eq!(parse_running_appid(FIXTURE_REGISTRY), Some(730));
    }

    #[test]
    fn running_appid_zero_is_none() {
        let vdf = r#""Registry"
{
    "HKCU"
    {
        "Software"
        {
            "Valve"
            {
                "Steam"
                {
                    "RunningAppID"  "0"
                }
            }
        }
    }
}"#;
        assert_eq!(parse_running_appid(vdf), None);
    }

    #[test]
    fn running_appid_missing_key_is_none() {
        let vdf = r#""Registry"
{
    "HKCU"
    {
        "Software"
        {
            "Valve"
            {
                "Steam"
                {
                    "SomethingElse"  "1"
                }
            }
        }
    }
}"#;
        assert_eq!(parse_running_appid(vdf), None);
    }

    #[test]
    fn running_appid_unparseable_is_none() {
        assert_eq!(parse_running_appid("not vdf at all {{{"), None);
    }

    #[test]
    fn steam_appid_from_reaper_argv() {
        // The canonical Linux launcher process for a running game.
        let argv = [
            "reaper",
            "SteamLaunch",
            "AppId=2215200",
            "--",
            "/home/user/.steam/steam/steamapps/common/Proton/proton",
            "waitforexitandrun",
            "/path/to/game.exe",
        ];
        assert_eq!(steam_appid_from_argv(&argv), Some(2215200));
    }

    #[test]
    fn steam_appid_from_unrelated_argv_is_none() {
        let argv = ["/usr/bin/firefox", "--new-window", "https://example.com"];
        assert_eq!(steam_appid_from_argv(&argv), None);
    }

    #[test]
    fn steam_appid_missing_appid_is_none() {
        // SteamLaunch present but no AppId= arg.
        let argv = ["reaper", "SteamLaunch", "--", "/some/proton"];
        assert_eq!(steam_appid_from_argv(&argv), None);
    }

    #[test]
    fn steam_appid_garbled_appid_is_none() {
        // SteamLaunch present, AppId= present but non-numeric.
        let argv = ["reaper", "SteamLaunch", "AppId=notanumber", "--"];
        assert_eq!(steam_appid_from_argv(&argv), None);
    }

    #[test]
    fn serverinfo_busy_detected() {
        // A live (active or resumable) session: state BUSY.
        let busy = "<root><state>SUNSHINE_SERVER_BUSY</state><currentgame>1093255277</currentgame></root>";
        assert!(serverinfo_is_busy(busy));
    }

    #[test]
    fn serverinfo_free_is_not_busy() {
        // Idle: state FREE, no current game.
        let free = "<root><state>SUNSHINE_SERVER_FREE</state><currentgame>0</currentgame></root>";
        assert!(!serverinfo_is_busy(free));
        // Empty / failed fetch ⇒ not busy.
        assert!(!serverinfo_is_busy(""));
        assert!(!serverinfo_is_busy("HTTP/1.0 404 Not Found"));
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
