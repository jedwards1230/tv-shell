//! Game navigation — fire a `steam://nav/games/details/<appid>` URL at the local
//! Steam client, per-OS. Steam picks up the protocol URL and **navigates Big
//! Picture to the game's library page** (it does NOT auto-start the game — the
//! user presses Play). Moonlight remains the stream engine, so this only needs to
//! get Steam pointed at the right page on the host.
//!
//! Robust timing: before firing the nav we wait (on Linux) for Big Picture mode
//! to be up, by scanning `/proc` for the `steamwebhelper … -bigpicture` process.
//! This makes the nav land whether BPM is already running (warm) or a stream is
//! just bringing it up (cold). If BPM never appears within the timeout we fire the
//! nav anyway.

use std::process::Command;
#[cfg(target_os = "linux")]
use std::time::{Duration, Instant};

/// Max time to wait for Big Picture mode to come up before firing the nav anyway.
#[cfg(target_os = "linux")]
const BIGPICTURE_WAIT: Duration = Duration::from_secs(12);

/// Poll interval while waiting for Big Picture mode.
#[cfg(target_os = "linux")]
const BIGPICTURE_POLL: Duration = Duration::from_millis(300);

/// Build the `steam://nav/games/details/<appid>` URL. This navigates Big Picture
/// to the game's details page rather than launching it (`steam://rungameid`).
fn steam_url(appid: u32) -> String {
    format!("steam://nav/games/details/{appid}")
}

/// Navigate the local Steam client's Big Picture to the given app's page. Returns
/// `Ok(())` once the nav command has been spawned (we don't wait for Steam to
/// finish rendering — the URL handler returns immediately and Steam keeps running
/// independently).
///
/// On Linux we first wait (up to [`BIGPICTURE_WAIT`]) for Big Picture mode to be
/// ready so the nav lands on a live BPM window; on other OSes we skip the wait and
/// fire immediately.
pub fn launch(appid: u32) -> anyhow::Result<()> {
    fire_steam_url(&steam_url(appid))
}

/// Open Steam Big Picture's HOME screen (no game pre-selected) by firing
/// `steam://open/bigpicture`. Unlike [`launch`] this does NOT navigate to a game's
/// page — it just resets Steam to the Big Picture home. Same BPM-up wait + fire-and-
/// detach choreography as [`launch`].
pub fn open_bigpicture() -> anyhow::Result<()> {
    fire_steam_url(BIGPICTURE_URL)
}

/// `steam://open/bigpicture` — open Big Picture's home screen (no game selected).
const BIGPICTURE_URL: &str = "steam://open/bigpicture";

/// Wait for Big Picture mode to be up (Linux), then fire a `steam://` URL at the
/// local Steam client and detach. Shared by [`launch`] (game nav) and
/// [`open_bigpicture`] (BPM home). Returns `Ok(())` once the launcher is spawned;
/// the URL handler returns immediately and Steam keeps running independently.
fn fire_steam_url(url: &str) -> anyhow::Result<()> {
    wait_for_bigpicture();

    tracing::info!("steam url: {url}");
    let mut cmd = launch_command(url);
    let status = cmd
        .spawn()
        .map_err(|e| anyhow::anyhow!("failed to spawn launcher for {url}: {e}"))?;
    // Detach: we don't `.wait()` — the URL handler returns immediately and Steam
    // keeps running independently. Dropping the child is fine (no zombie: the
    // launcher process is short-lived).
    drop(status);
    Ok(())
}

/// Wait for Big Picture mode to be ready before firing the nav.
///
/// - **Linux**: poll `/proc` for a `steamwebhelper … -bigpicture` process, up to
///   [`BIGPICTURE_WAIT`]. If it's already up (warm) we return immediately; if it
///   appears mid-wait (cold start by a stream) we return as soon as it does; if
///   the timeout elapses we return anyway and the caller fires the nav regardless.
/// - **Other OSes**: no-op (we just fire the nav).
fn wait_for_bigpicture() {
    #[cfg(target_os = "linux")]
    {
        let deadline = Instant::now() + BIGPICTURE_WAIT;
        loop {
            if bigpicture_running() {
                tracing::debug!("nav: Big Picture is up");
                return;
            }
            if Instant::now() >= deadline {
                tracing::debug!("nav: Big Picture wait timed out; firing nav anyway");
                return;
            }
            std::thread::sleep(BIGPICTURE_POLL);
        }
    }
    // Other OSes: no BPM-process check — `launch` just fires the nav.
}

/// Is Steam's Big Picture mode running? Scans `/proc/*/cmdline` for a process
/// whose argv contains a `bigpicture` token (the `steamwebhelper … -bigpicture`
/// process). Pure `/proc` scan with std (no new crates); unreadable entries
/// (permissions, pid-exit races) are skipped. Linux-only.
#[cfg(target_os = "linux")]
fn bigpicture_running() -> bool {
    let Ok(proc) = std::fs::read_dir("/proc") else {
        return false;
    };
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
        // cmdline is NUL-separated; keep only valid UTF-8 args.
        let args: Vec<&str> = raw
            .split(|&b| b == 0)
            .filter(|s| !s.is_empty())
            .filter_map(|s| std::str::from_utf8(s).ok())
            .collect();
        if cmdline_is_bigpicture(&args) {
            return true;
        }
    }
    false
}

/// Pure predicate: does this argv belong to a Steam Big Picture process? Matches
/// any arg containing the `bigpicture` token (e.g. `-bigpicture`). Unit-tested
/// without touching `/proc`.
#[cfg(any(target_os = "linux", test))]
fn cmdline_is_bigpicture(args: &[&str]) -> bool {
    args.iter().any(|a| a.contains("bigpicture"))
}

/// Construct the per-OS command that opens a `steam://` URL.
///
/// - Linux: `steam <url>` (Steam registers the `steam://` handler; if `steam`
///   isn't on PATH, `xdg-open` is the documented fallback — but `steam` is the
///   direct path and avoids a desktop-portal round-trip).
/// - Windows: `cmd /C start "" <url>` (`start` resolves the registered URL
///   protocol handler; the empty `""` is the required title arg).
/// - macOS: `open <url>`.
fn launch_command(url: &str) -> Command {
    #[cfg(target_os = "linux")]
    {
        let mut c = Command::new("steam");
        c.arg(url);
        c
    }
    #[cfg(target_os = "windows")]
    {
        let mut c = Command::new("cmd");
        c.args(["/C", "start", "", url]);
        c
    }
    #[cfg(target_os = "macos")]
    {
        let mut c = Command::new("open");
        c.arg(url);
        c
    }
    #[cfg(not(any(target_os = "linux", target_os = "windows", target_os = "macos")))]
    {
        // Last-ditch: try xdg-open semantics on unknown unixes.
        let mut c = Command::new("xdg-open");
        c.arg(url);
        c
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_shape() {
        assert_eq!(steam_url(730), "steam://nav/games/details/730");
        assert_eq!(steam_url(0), "steam://nav/games/details/0");
    }

    #[test]
    fn launch_command_targets_a_program() {
        // The command's program is set (per-OS) and includes the URL somewhere
        // in its argv. We don't spawn it in the test — just assert it's built.
        let cmd = launch_command("steam://nav/games/details/220");
        let prog = cmd.get_program().to_string_lossy().to_string();
        assert!(!prog.is_empty());
        let args: Vec<String> = cmd
            .get_args()
            .map(|a| a.to_string_lossy().to_string())
            .collect();
        assert!(
            args.iter().any(|a| a.contains("nav/games/details/220")),
            "argv {args:?} should carry the steam nav url"
        );
    }

    #[test]
    fn bigpicture_detected_from_steamwebhelper_argv() {
        // The canonical Big Picture process argv carries `-bigpicture`.
        let argv = [
            "/home/user/.steam/steam/ubuntu12_64/steamwebhelper",
            "-lang=en_US",
            "-bigpicture",
            "-cef-sandbox-allow-network",
        ];
        assert!(cmdline_is_bigpicture(&argv));
    }

    #[test]
    fn bigpicture_not_detected_from_plain_steam_argv() {
        // A regular (non-BPM) Steam/desktop process must not match.
        let argv = ["/usr/bin/steam", "steam://nav/games/details/220"];
        assert!(!cmdline_is_bigpicture(&argv));

        let unrelated = ["/usr/bin/firefox", "--new-window"];
        assert!(!cmdline_is_bigpicture(&unrelated));
    }
}
