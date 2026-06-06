//! Session-environment self-discovery and daemon.env loading (#165).
//!
//! The daemon is often launched before the Wayland compositor starts (the
//! session wrapper runs `exec game-shell-input &; exec Hyprland`), so
//! `WAYLAND_DISPLAY`, `HYPRLAND_INSTANCE_SIGNATURE`, and friends are not
//! inherited. This module provides helpers to:
//!
//! 1. Load a `daemon.env` file from `${XDG_CONFIG_HOME:-$HOME/.config}/game-shell/`
//!    into the process environment (only for variables that are not already set).
//! 2. Resolve `WAYLAND_DISPLAY` from the live compositor socket in
//!    `$XDG_RUNTIME_DIR`.
//! 3. Resolve `HYPRLAND_INSTANCE_SIGNATURE` from either the env or the
//!    `$XDG_RUNTIME_DIR/hypr/` socket directory.
//! 4. Collect the session variables that should be injected into child
//!    subprocesses (grim, quickshell, …).
//! 5. Resolve the installation root (`/opt/game-shell` by default).
//!
//! All functions are intentionally side-effect-free except `load_daemon_env`
//! (which calls `std::env::set_var`) and `install_root` (which calls
//! `std::env::current_exe`).

use std::path::PathBuf;

// ---------------------------------------------------------------------------
// daemon.env loading
// ---------------------------------------------------------------------------

/// Parse `KEY=VALUE` pairs from `content`.
///
/// Rules:
/// - Blank lines and lines whose first non-whitespace character is `#` are
///   skipped.
/// - Surrounding single or double quotes on the VALUE are stripped (but not
///   required).
/// - Inline `#` comments after the value are NOT stripped (uncommon in env
///   files and avoid ambiguity with values that contain `#`).
/// - Lines without `=` are skipped.
///
/// Returns a `Vec<(key, value)>` in file order.  The caller decides whether to
/// apply them to the process environment.
pub fn parse_env_lines(content: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let Some(eq) = trimmed.find('=') else {
            continue;
        };
        let key = trimmed[..eq].trim().to_owned();
        if key.is_empty() {
            continue;
        }
        let raw_val = trimmed[eq + 1..].trim();
        let value = strip_quotes(raw_val).to_owned();
        out.push((key, value));
    }
    out
}

/// Strip a single layer of matching surrounding single or double quotes.
fn strip_quotes(s: &str) -> &str {
    if (s.starts_with('"') && s.ends_with('"')) || (s.starts_with('\'') && s.ends_with('\'')) {
        if s.len() >= 2 {
            return &s[1..s.len() - 1];
        }
    }
    s
}

/// Load `${XDG_CONFIG_HOME:-$HOME/.config}/game-shell/daemon.env` into the
/// process environment.
///
/// For each `KEY=VALUE` pair produced by [`parse_env_lines`], if
/// `std::env::var_os(KEY)` is `None` (variable not already set), the variable
/// is set via `std::env::set_var`.  Variables that are already present in the
/// environment (e.g. passed explicitly by a wrapper script) are left untouched,
/// so the env file acts as a fallback rather than an override.
///
/// Missing or unreadable files are silently ignored — the function degrades
/// gracefully when the file does not exist yet.
pub fn load_daemon_env() {
    let path = daemon_env_path();
    let content = match std::fs::read_to_string(&path) {
        Ok(c) => c,
        Err(_) => return, // file absent or unreadable — normal during first run
    };
    for (key, value) in parse_env_lines(&content) {
        if std::env::var_os(&key).is_none() {
            // SAFETY: single-threaded at the point this is called (before any
            // tokio runtime or spawned thread is created).
            #[allow(deprecated)]
            // set_var is deprecated in Rust 2024 edition for multi-thread safety
            std::env::set_var(&key, &value);
        }
    }
}

/// Returns the path to the daemon.env file:
/// `${XDG_CONFIG_HOME:-$HOME/.config}/game-shell/daemon.env`
fn daemon_env_path() -> PathBuf {
    let config_home = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            let home = std::env::var_os("HOME")
                .map(PathBuf::from)
                .unwrap_or_default();
            home.join(".config")
        });
    config_home.join("game-shell").join("daemon.env")
}

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
/// 1. `std::env::current_exe()` → `parent()` (bin/) → `parent()` (install root,
///    e.g. `/opt/game-shell`).
/// 2. `GAME_SHELL_DIR` environment variable.
/// 3. Hard-coded fallback `/opt/game-shell`.
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

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // ── parse_env_lines ──────────────────────────────────────────────────────

    #[test]
    fn parse_simple_pair() {
        let pairs = parse_env_lines("FOO=bar\n");
        assert_eq!(pairs, vec![("FOO".to_owned(), "bar".to_owned())]);
    }

    #[test]
    fn parse_double_quoted_value() {
        let pairs = parse_env_lines("FOO=\"bar baz\"\n");
        assert_eq!(pairs, vec![("FOO".to_owned(), "bar baz".to_owned())]);
    }

    #[test]
    fn parse_single_quoted_value() {
        let pairs = parse_env_lines("FOO='bar baz'\n");
        assert_eq!(pairs, vec![("FOO".to_owned(), "bar baz".to_owned())]);
    }

    #[test]
    fn parse_skips_blank_lines() {
        let pairs = parse_env_lines("\n   \nFOO=bar\n\n");
        assert_eq!(pairs, vec![("FOO".to_owned(), "bar".to_owned())]);
    }

    #[test]
    fn parse_skips_comments() {
        let pairs = parse_env_lines("# this is a comment\nFOO=bar\n");
        assert_eq!(pairs, vec![("FOO".to_owned(), "bar".to_owned())]);
    }

    #[test]
    fn parse_skips_line_without_equals() {
        let pairs = parse_env_lines("NOTAPAIR\nFOO=bar\n");
        assert_eq!(pairs, vec![("FOO".to_owned(), "bar".to_owned())]);
    }

    #[test]
    fn parse_multiple_pairs() {
        let content = "GAME_SHELL_HTTP_BIND=0.0.0.0:7070\nGAME_SHELL_HTTP_TOKEN=secret\n";
        let pairs = parse_env_lines(content);
        assert_eq!(
            pairs,
            vec![
                ("GAME_SHELL_HTTP_BIND".to_owned(), "0.0.0.0:7070".to_owned()),
                ("GAME_SHELL_HTTP_TOKEN".to_owned(), "secret".to_owned()),
            ]
        );
    }

    #[test]
    fn parse_value_with_equals_sign() {
        // VALUE itself contains '=' — only the first '=' is the separator.
        let pairs = parse_env_lines("FOO=bar=baz\n");
        assert_eq!(pairs, vec![("FOO".to_owned(), "bar=baz".to_owned())]);
    }

    #[test]
    fn parse_empty_value() {
        let pairs = parse_env_lines("FOO=\n");
        assert_eq!(pairs, vec![("FOO".to_owned(), "".to_owned())]);
    }

    #[test]
    fn parse_empty_quoted_value() {
        let pairs = parse_env_lines("FOO=\"\"\n");
        // Two-char quoted string "" → strip both quotes → ""
        assert_eq!(pairs, vec![("FOO".to_owned(), "".to_owned())]);
    }

    #[test]
    fn strip_quotes_double() {
        assert_eq!(strip_quotes("\"hello\""), "hello");
    }

    #[test]
    fn strip_quotes_single() {
        assert_eq!(strip_quotes("'hello'"), "hello");
    }

    #[test]
    fn strip_quotes_no_quotes() {
        assert_eq!(strip_quotes("hello"), "hello");
    }

    #[test]
    fn strip_quotes_mismatched_leaves_intact() {
        // Single open, double close — not matching, pass through.
        assert_eq!(strip_quotes("'hello\""), "'hello\"");
    }

    // ── install_root fallback ────────────────────────────────────────────────

    #[test]
    fn install_root_returns_pathbuf() {
        // We can't assert the exact value in a unit test, but we can assert it's
        // non-empty and that the function does not panic.
        let root = install_root();
        assert!(!root.as_os_str().is_empty());
    }
}
