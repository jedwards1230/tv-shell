//! Session-environment self-discovery (#165).
//!
//! The daemon is often launched before the Wayland compositor starts (the
//! session wrapper runs `exec game-shell-input &; exec Hyprland`), so
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
//!    `$GAME_SHELL_DIR`; `/opt/game-shell` is only a last-ditch fallback).
//!
//! Per-machine daemon options (HTTP/MCP/CEC/Plex/Steam) no longer live here as a
//! `daemon.env` env file — they are a typed `config.toml` read by
//! [`crate::daemon_config`]. This module is now only about session/runtime
//! discovery, not config.
//!
//! All functions are intentionally side-effect-free except `install_root`
//! (which calls `std::env::current_exe`).

use std::path::PathBuf;

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
/// Returns `HYPRLAND_INSTANCE_SIGNATURE` from the environment when it is set
/// and non-empty.  Otherwise, reads the entries of `$XDG_RUNTIME_DIR/hypr/`
/// and picks the most-recently-modified subdirectory that contains either
/// `.socket.sock` or `.socket2.sock` — that subdirectory name IS the instance
/// signature.
pub fn resolve_hypr_signature() -> Option<String> {
    if let Ok(sig) = std::env::var("HYPRLAND_INSTANCE_SIGNATURE") {
        if !sig.is_empty() {
            return Some(sig);
        }
    }

    let runtime_dir = PathBuf::from(std::env::var_os("XDG_RUNTIME_DIR")?);
    let hypr_dir = runtime_dir.join("hypr");

    let mut best: Option<(std::time::SystemTime, String)> = None;

    let entries = std::fs::read_dir(&hypr_dir).ok()?;
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
/// 2. `GAME_SHELL_DIR` environment variable.
/// 3. `/opt/game-shell` is only a last-ditch fallback.
pub fn install_root() -> PathBuf {
    if let Ok(exe) = std::env::current_exe() {
        if let Some(root) = exe.parent().and_then(|p| p.parent()) {
            return root.to_path_buf();
        }
    }
    if let Ok(dir) = std::env::var("GAME_SHELL_DIR") {
        if !dir.is_empty() {
            return PathBuf::from(dir);
        }
    }
    PathBuf::from("/opt/game-shell")
}

/// Return the path to the `game-shell-input` daemon binary.
///
/// Resolution order:
/// 1. `$GAME_SHELL_INPUT_BIN` when set and non-empty (lets a packaged
///    dev-override point the re-exec target at an arbitrary build).
/// 2. [`install_root`]`.join("bin/game-shell-input")` (the canonical
///    install-path binary; `/opt/game-shell` is only a last-ditch fallback via
///    `install_root`).
pub fn input_bin() -> PathBuf {
    if let Ok(p) = std::env::var("GAME_SHELL_INPUT_BIN") {
        if !p.is_empty() {
            return PathBuf::from(p);
        }
    }
    install_root().join("bin/game-shell-input")
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // ── install_root fallback ────────────────────────────────────────────────

    #[test]
    fn install_root_returns_pathbuf() {
        // We can't assert the exact value in a unit test, but we can assert it's
        // non-empty and that the function does not panic.
        let root = install_root();
        assert!(!root.as_os_str().is_empty());
    }

    // ── input_bin resolution ─────────────────────────────────────────────────

    // GAME_SHELL_INPUT_BIN is process-global, so the three cases are exercised in
    // one test to avoid a set/remove race between parallel test threads.
    #[test]
    fn input_bin_resolution() {
        // 1. Override set + non-empty → wins outright.
        std::env::set_var(
            "GAME_SHELL_INPUT_BIN",
            "/custom/prefix/bin/game-shell-input",
        );
        assert_eq!(
            input_bin(),
            PathBuf::from("/custom/prefix/bin/game-shell-input")
        );

        // 2. Empty override → treated as unset → install_root fallback.
        std::env::set_var("GAME_SHELL_INPUT_BIN", "");
        assert_eq!(input_bin(), install_root().join("bin/game-shell-input"));

        // 3. Unset → install_root fallback.
        std::env::remove_var("GAME_SHELL_INPUT_BIN");
        let bin = input_bin();
        assert_eq!(bin, install_root().join("bin/game-shell-input"));
        assert!(bin.ends_with("bin/game-shell-input"));
    }
}
