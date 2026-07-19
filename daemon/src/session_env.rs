//! Session-environment self-discovery (#165).
//!
//! The daemon is often launched before the Wayland compositor starts (the
//! session wrapper runs `exec tv-shell-input &; exec Hyprland`), so
//! `WAYLAND_DISPLAY`, `HYPRLAND_INSTANCE_SIGNATURE`, and friends are not
//! inherited. This module provides helpers to:
//!
//! 1. Resolve `WAYLAND_DISPLAY` from the live compositor socket in
//!    `$XDG_RUNTIME_DIR`.
//! 2. Resolve `HYPRLAND_INSTANCE_SIGNATURE` from either the env or the
//!    `$XDG_RUNTIME_DIR/hypr/` socket directory.
//! 3. Collect the session variables that should be injected into child
//!    subprocesses (grim, quickshell, …).
//! 4. Resolve the installation root (resolved from `current_exe` /
//!    `$TV_SHELL_DIR`; `/opt/tv-shell` is only a last-ditch fallback).
//!
//! Per-machine daemon options (HTTP/MCP/CEC/Plex/Steam) no longer live here as a
//! `daemon.env` env file — they are a typed `config.toml` read by
//! [`crate::daemon_config`]. This module is now only about session/runtime
//! discovery, not config.
//!
//! All functions are intentionally side-effect-free except `install_root`
//! (which calls `std::env::current_exe`).

use std::path::{Path, PathBuf};

// ---------------------------------------------------------------------------
// Wayland display resolution
// ---------------------------------------------------------------------------

/// Resolve the Wayland display socket name for subprocesses (grim, etc.).
///
/// The session wrapper launches the daemon *before* `exec Hyprland`, so the
/// daemon may not inherit `WAYLAND_DISPLAY`.  Prefer the inherited value when
/// present, otherwise discover the compositor socket in `$XDG_RUNTIME_DIR`
/// (`wayland-N`, preferring one with a `wayland-N.lock` sibling, which marks
/// a live server).
pub fn resolve_wayland_display() -> Option<String> {
    if let Some(d) = std::env::var_os("WAYLAND_DISPLAY") {
        let s = d.to_string_lossy().into_owned();
        if !s.is_empty() {
            return Some(s);
        }
    }
    let dir = PathBuf::from(std::env::var_os("XDG_RUNTIME_DIR")?);
    let mut fallback: Option<String> = None;
    for entry in std::fs::read_dir(&dir).ok()?.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let Some(rest) = name.strip_prefix("wayland-") else {
            continue;
        };
        if rest.is_empty() || !rest.chars().all(|c| c.is_ascii_digit()) {
            continue;
        }
        if dir.join(format!("{name}.lock")).exists() {
            return Some(name);
        }
        fallback.get_or_insert(name);
    }
    fallback
}

// ---------------------------------------------------------------------------
// Hyprland instance signature resolution
// ---------------------------------------------------------------------------

/// Resolve the Hyprland instance signature.
///
/// Prefers the **live** socket directory on disk over the inherited
/// `HYPRLAND_INSTANCE_SIGNATURE` env var, and only falls back to the env var
/// when no live directory is present yet. This ordering is deliberate and load-
/// bearing:
///
/// The daemon is a long-lived `systemd --user` unit. If a Hyprland instance
/// imported `HYPRLAND_INSTANCE_SIGNATURE` into the user manager's environment
/// (`systemctl --user import-environment` / `dbus-update-activation-environment`),
/// the daemon can inherit it — and a process's environment is frozen for its
/// lifetime. When that Hyprland is later killed and restarted (e.g. render-loop
/// hang recovery after an HDMI/CEC flap), it comes up under a NEW signature, but
/// the daemon's inherited env var still names the DEAD instance. Trusting the env
/// var first (the previous behavior) then pinned every socket path to the dead
/// instance forever: queries and the event stream failed with "Connection
/// refused", the daemon went silently deaf to the live compositor, and no amount
/// of reconnect backoff could recover because each retry re-resolved to the same
/// stale signature. Scanning `$XDG_RUNTIME_DIR/hypr/` first reflects the CURRENT
/// compositor, so the reconnect loop self-heals onto the new instance.
///
/// The scan picks the most-recently-modified `$XDG_RUNTIME_DIR/hypr/<sig>`
/// subdirectory that still contains a `.socket.sock`/`.socket2.sock` — that
/// subdirectory name IS the instance signature. On a single-compositor kiosk the
/// newest such directory is the live instance; a killed instance's directory has
/// an older mtime, so the freshly-created live one wins. (Residual edge: a socket
/// FILE left behind by a `SIGKILL`ed instance can linger and pass the existence
/// check; the reconnect loop's connect attempt then fails and retries, and once
/// the live instance's newer directory appears the scan prefers it.)
pub fn resolve_hypr_signature() -> Option<String> {
    if let Some(rt) = std::env::var_os("XDG_RUNTIME_DIR") {
        if let Some(sig) = newest_live_signature_in(&PathBuf::from(rt).join("hypr")) {
            return Some(sig);
        }
    }

    // No live socket dir yet (e.g. the daemon started before Hyprland): fall back
    // to the inherited env var if it names anything.
    std::env::var("HYPRLAND_INSTANCE_SIGNATURE")
        .ok()
        .filter(|s| !s.is_empty())
}

/// Scan a `.../hypr/` directory for the most-recently-modified subdirectory that
/// still holds a Hyprland IPC socket, returning its name (the instance
/// signature). Pure over the passed path — takes no environment — so it is unit-
/// testable without mutating process-global env vars. Returns `None` when the
/// directory is absent or holds no socket-bearing subdirectory.
fn newest_live_signature_in(hypr_dir: &Path) -> Option<String> {
    let mut best: Option<(std::time::SystemTime, String)> = None;

    let entries = std::fs::read_dir(hypr_dir).ok()?;
    for entry in entries.flatten() {
        let ft = match entry.file_type() {
            Ok(t) => t,
            Err(_) => continue,
        };
        if !ft.is_dir() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        let sub = hypr_dir.join(&name);

        // Must contain a socket file to be considered a live instance.
        let has_socket = sub.join(".socket.sock").exists() || sub.join(".socket2.sock").exists();
        if !has_socket {
            continue;
        }

        // Use the directory's mtime as the "most recent" heuristic.
        let mtime = entry
            .metadata()
            .ok()
            .and_then(|m| m.modified().ok())
            .unwrap_or(std::time::UNIX_EPOCH);

        let is_newer = best.as_ref().map(|(t, _)| mtime > *t).unwrap_or(true);
        if is_newer {
            best = Some((mtime, name));
        }
    }

    best.map(|(_, name)| name)
}

// ---------------------------------------------------------------------------
// Session env pairs for subprocess injection
// ---------------------------------------------------------------------------

/// Collect the session environment variables that should be forwarded to
/// child subprocesses (grim, quickshell, …).
///
/// Returns only the pairs whose values are actually resolved (non-`None`).
/// Callers iterate the result and apply it via `Command::env`.
pub fn session_env_pairs() -> Vec<(String, String)> {
    let mut pairs = Vec::new();

    if let Some(d) = resolve_wayland_display() {
        pairs.push(("WAYLAND_DISPLAY".to_owned(), d));
    }
    if let Some(sig) = resolve_hypr_signature() {
        pairs.push(("HYPRLAND_INSTANCE_SIGNATURE".to_owned(), sig));
    }
    if let Ok(rdir) = std::env::var("XDG_RUNTIME_DIR") {
        if !rdir.is_empty() {
            pairs.push(("XDG_RUNTIME_DIR".to_owned(), rdir));
        }
    }

    pairs
}

// ---------------------------------------------------------------------------
// Installation root
// ---------------------------------------------------------------------------

/// Return the installation root directory.
///
/// Resolution order:
/// 1. `std::env::current_exe()` → `parent()` (bin/) → `parent()` (install root).
/// 2. `TV_SHELL_DIR` environment variable (legacy `GAME_SHELL_DIR` honored).
/// 3. `/opt/tv-shell` is only a last-ditch fallback (`brand::install_root_default`).
pub fn install_root() -> PathBuf {
    if let Ok(exe) = std::env::current_exe() {
        if let Some(root) = exe.parent().and_then(|p| p.parent()) {
            return root.to_path_buf();
        }
    }
    if let Some(dir) = tv_shell_protocol::brand::env("DIR") {
        return PathBuf::from(dir);
    }
    tv_shell_protocol::brand::install_root_default()
}

/// Return the path to the `tv-shell-input` daemon binary.
///
/// Resolution order:
/// 1. `$TV_SHELL_INPUT_BIN` (legacy `$GAME_SHELL_INPUT_BIN`) when set and
///    non-empty (lets a packaged dev-override point the re-exec target at an
///    arbitrary build).
/// 2. [`install_root`]`.join("bin/tv-shell-input")` (the canonical install-path
///    binary; `/opt/tv-shell` is only a last-ditch fallback via `install_root`).
pub fn input_bin() -> PathBuf {
    if let Some(p) = tv_shell_protocol::brand::env("INPUT_BIN") {
        return PathBuf::from(p);
    }
    install_root().join("bin/tv-shell-input")
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// Create a unique scratch directory for a test. Delegates to the
    /// shared [`crate::testutil::scratch_dir`] helper — see its doc comment
    /// for why it's based on `current_exe()` rather than the system temp dir,
    /// and for the residual fresh-worktree limitation.
    fn scratch(tag: &str) -> PathBuf {
        crate::testutil::scratch_dir(tag)
    }

    /// Materialize a fake `hypr/<sig>/` instance dir; when `with_socket`, drop a
    /// `.socket2.sock` marker file so it counts as a live instance.
    fn make_instance(hypr_dir: &Path, sig: &str, with_socket: bool) {
        let sub = hypr_dir.join(sig);
        std::fs::create_dir_all(&sub).unwrap();
        if with_socket {
            std::fs::write(sub.join(".socket2.sock"), b"").unwrap();
        }
    }

    // ── newest_live_signature_in (pure scan) ─────────────────────────────────

    #[test]
    fn scan_returns_none_for_absent_or_empty_dir() {
        let root = scratch("scan-empty");
        // Absent hypr/ dir.
        assert_eq!(newest_live_signature_in(&root.join("hypr")), None);
        // Present but empty hypr/ dir.
        let hypr = root.join("hypr");
        std::fs::create_dir_all(&hypr).unwrap();
        assert_eq!(newest_live_signature_in(&hypr), None);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn scan_skips_dirs_without_a_socket() {
        // A socketless instance dir is never a candidate, so even when it is the
        // only entry the scan yields None — and when a live one coexists, the
        // live one wins regardless of mtime ordering.
        let root = scratch("scan-skip");
        let hypr = root.join("hypr");
        make_instance(&hypr, "dead_no_socket", false);
        assert_eq!(newest_live_signature_in(&hypr), None);

        make_instance(&hypr, "live_abc123", true);
        assert_eq!(
            newest_live_signature_in(&hypr).as_deref(),
            Some("live_abc123")
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    // ── resolve_hypr_signature (scan-first, env fallback) ─────────────────────

    // Exercises the load-bearing ordering in one test: a live socket dir must win
    // over a stale inherited HYPRLAND_INSTANCE_SIGNATURE (the deaf-daemon fix),
    // and the env var is used only when no live dir exists. Both env vars are
    // process-global, so the cases share one test (mirrors input_bin_resolution)
    // and the env is restored at the end.
    #[test]
    fn resolve_prefers_live_socket_dir_over_stale_env() {
        let prev_xdg = std::env::var_os("XDG_RUNTIME_DIR");
        let prev_sig = std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE");

        let root = scratch("resolve");
        make_instance(&root.join("hypr"), "live_sig_9999", true);

        std::env::set_var("XDG_RUNTIME_DIR", &root);
        // Stale env: names a DEAD instance with no socket dir on disk.
        std::env::set_var("HYPRLAND_INSTANCE_SIGNATURE", "dead_stale_env_sig");
        assert_eq!(
            resolve_hypr_signature().as_deref(),
            Some("live_sig_9999"),
            "live socket dir must override a stale inherited signature"
        );

        // No live dir anywhere → fall back to whatever the env names.
        let empty = scratch("resolve-empty");
        std::env::set_var("XDG_RUNTIME_DIR", &empty);
        assert_eq!(
            resolve_hypr_signature().as_deref(),
            Some("dead_stale_env_sig"),
            "with no live dir the inherited env signature is the only lead"
        );

        // Restore prior environment for other tests.
        match prev_xdg {
            Some(v) => std::env::set_var("XDG_RUNTIME_DIR", v),
            None => std::env::remove_var("XDG_RUNTIME_DIR"),
        }
        match prev_sig {
            Some(v) => std::env::set_var("HYPRLAND_INSTANCE_SIGNATURE", v),
            None => std::env::remove_var("HYPRLAND_INSTANCE_SIGNATURE"),
        }
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&empty);
    }

    // ── install_root fallback ────────────────────────────────────────────────

    #[test]
    fn install_root_returns_pathbuf() {
        // We can't assert the exact value in a unit test, but we can assert it's
        // non-empty and that the function does not panic.
        let root = install_root();
        assert!(!root.as_os_str().is_empty());
    }

    // ── input_bin resolution ─────────────────────────────────────────────────

    // TV_SHELL_INPUT_BIN is process-global, so the three cases are exercised in
    // one test to avoid a set/remove race between parallel test threads. The
    // legacy GAME_SHELL_INPUT_BIN is removed throughout so the brand::env
    // fallback can't leak an ambient value into the empty/unset cases.
    #[test]
    fn input_bin_resolution() {
        std::env::remove_var("GAME_SHELL_INPUT_BIN");

        // 1. Override set + non-empty → wins outright.
        std::env::set_var("TV_SHELL_INPUT_BIN", "/custom/prefix/bin/tv-shell-input");
        assert_eq!(
            input_bin(),
            PathBuf::from("/custom/prefix/bin/tv-shell-input")
        );

        // 2. Empty override → treated as unset → install_root fallback.
        std::env::set_var("TV_SHELL_INPUT_BIN", "");
        assert_eq!(input_bin(), install_root().join("bin/tv-shell-input"));

        // 3. Unset → install_root fallback.
        std::env::remove_var("TV_SHELL_INPUT_BIN");
        let bin = input_bin();
        assert_eq!(bin, install_root().join("bin/tv-shell-input"));
        assert!(bin.ends_with("bin/tv-shell-input"));
    }
}
