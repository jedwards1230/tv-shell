//! Game launch — fire a `steam://rungameid/<appid>` URL at the local Steam
//! client, per-OS. Steam picks up the protocol URL and starts (or focuses) the
//! game; we keep Moonlight as the stream engine, so this only needs to get the
//! game running on the host.

use std::process::Command;

/// Build the `steam://rungameid/<appid>` URL.
fn steam_url(appid: u32) -> String {
    format!("steam://rungameid/{appid}")
}

/// Launch the given Steam app on this host. Returns `Ok(())` once the launch
/// command has been spawned (we don't wait for Steam to finish starting the
/// game — that can take a while and Moonlight handles the stream regardless).
pub fn launch(appid: u32) -> anyhow::Result<()> {
    let url = steam_url(appid);
    tracing::info!("launch: {url}");

    let mut cmd = launch_command(&url);
    let status = cmd
        .spawn()
        .map_err(|e| anyhow::anyhow!("failed to spawn launcher for {url}: {e}"))?;
    // Detach: we don't `.wait()` — the URL handler returns immediately and Steam
    // keeps running independently. Dropping the child is fine (no zombie: the
    // launcher process is short-lived).
    drop(status);
    Ok(())
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
        assert_eq!(steam_url(730), "steam://rungameid/730");
        assert_eq!(steam_url(0), "steam://rungameid/0");
    }

    #[test]
    fn launch_command_targets_a_program() {
        // The command's program is set (per-OS) and includes the URL somewhere
        // in its argv. We don't spawn it in the test — just assert it's built.
        let cmd = launch_command("steam://rungameid/220");
        let prog = cmd.get_program().to_string_lossy().to_string();
        assert!(!prog.is_empty());
        let args: Vec<String> = cmd
            .get_args()
            .map(|a| a.to_string_lossy().to_string())
            .collect();
        assert!(
            args.iter().any(|a| a.contains("rungameid/220")),
            "argv {args:?} should carry the steam url"
        );
    }
}
