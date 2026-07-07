//! Central brand identity for tv-shell (formerly game-shell).
//!
//! Single source of truth for the product slug, env-var prefix, metric prefix,
//! HTTP header prefix, and the filesystem/naming defaults that were renamed in
//! the game-shell → tv-shell rebrand. Every runtime read of a `TV_SHELL_*` env
//! var, the per-user config dir, and the install-root default routes through
//! here so the **backward-compat shims** (legacy `GAME_SHELL_*` env fallback,
//! read-fallback to `~/.config/game-shell`) live in exactly one place and a box
//! that reboots mid-migration keeps working.
//!
//! Pure std — no I/O beyond env reads and `Path::is_dir` existence probes, so
//! both the daemon and the host sidecar depend on it without pulling in either
//! one's heavier graph.

use std::path::{Path, PathBuf};

/// Current product slug (config dir, quickshell config, session/unit basenames).
pub const SLUG: &str = "tv-shell";
/// Pre-rename slug, kept for read-fallback + compat symlinks.
pub const LEGACY_SLUG: &str = "game-shell";

/// Current env-var prefix (`TV_SHELL_SOCK`, `TV_SHELL_HTTP_BIND`, …).
pub const ENV_PREFIX: &str = "TV_SHELL";
/// Pre-rename env-var prefix, honored as a fallback by [`env`].
pub const LEGACY_ENV: &str = "GAME_SHELL";

/// Prometheus metric-name prefix (`tv_shell_build_info`, …).
pub const METRIC_PREFIX: &str = "tv_shell";
/// HTTP response-header prefix (`X-TvShell-Sha`, …).
pub const HEADER_PREFIX: &str = "X-TvShell";

/// Read an env var by its suffix, honoring the legacy prefix as a fallback.
///
/// `env("SOCK")` reads `TV_SHELL_SOCK`, and only if that is unset (or empty)
/// falls back to `GAME_SHELL_SOCK`. An empty value is treated as unset so a
/// stray `TV_SHELL_SOCK=` in the environment does not mask a real legacy value.
pub fn env(suffix: &str) -> Option<String> {
    for prefix in [ENV_PREFIX, LEGACY_ENV] {
        if let Ok(val) = std::env::var(format!("{prefix}_{suffix}")) {
            if !val.is_empty() {
                return Some(val);
            }
        }
    }
    None
}

/// The base config directory (`$XDG_CONFIG_HOME`, else `$HOME/.config`).
fn config_base() -> PathBuf {
    if let Some(xdg) = std::env::var_os("XDG_CONFIG_HOME") {
        if !xdg.is_empty() {
            return PathBuf::from(xdg);
        }
    }
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default();
    home.join(".config")
}

/// Resolve the per-user config dir against a base, with legacy read-fallback.
///
/// Returns `<base>/tv-shell` — UNLESS that directory does not exist and the
/// legacy `<base>/game-shell` does, in which case the legacy dir is returned so
/// a box that has not been re-provisioned keeps reading its existing config.
fn resolve_config_dir(base: &Path) -> PathBuf {
    let current = base.join(SLUG);
    if current.is_dir() {
        return current;
    }
    let legacy = base.join(LEGACY_SLUG);
    if legacy.is_dir() {
        return legacy;
    }
    current
}

/// The per-user config directory, with legacy read-fallback.
///
/// `${XDG_CONFIG_HOME:-$HOME/.config}/tv-shell`, but if that directory does not
/// exist and `…/game-shell` does, the legacy path is returned (read-fallback so
/// a mid-migration reboot still finds `config.toml`/`settings.json`).
pub fn config_dir() -> PathBuf {
    resolve_config_dir(&config_base())
}

/// The base data directory (`$HOME/.local/share`).
fn data_base() -> PathBuf {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default();
    home.join(".local/share")
}

/// The per-user data directory, with legacy read-fallback.
///
/// `$HOME/.local/share/tv-shell`, but if that directory does not exist and
/// `…/game-shell` does, the legacy path is returned so a mid-migration reboot
/// keeps its `recents.json` / `notifications.json` / cached controller DB.
pub fn data_dir() -> PathBuf {
    resolve_config_dir(&data_base())
}

/// Default install root (`--prefix`) for a from-source install.
pub fn install_root_default() -> PathBuf {
    PathBuf::from("/opt/tv-shell")
}

/// MCP server implementation name reported in the MCP handshake.
pub fn mcp_server_name() -> &'static str {
    "tv-shell-mcp"
}

/// Default IPC socket basename (`tv-shell-input.sock`), under `$XDG_RUNTIME_DIR`.
pub fn socket_name() -> &'static str {
    "tv-shell-input.sock"
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    // Unique temp dir per call (std-only, no tempfile dep). Racy names are
    // avoided via pid + a process-local counter.
    fn unique_tmp(tag: &str) -> PathBuf {
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("tv-shell-brand-{tag}-{}-{n}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn env_prefers_new_prefix_then_falls_back_to_legacy() {
        // Unique suffix keeps this test independent of any real env + other tests.
        let suffix = "BRAND_TEST_FALLBACK";
        let new_key = format!("{ENV_PREFIX}_{suffix}");
        let legacy_key = format!("{LEGACY_ENV}_{suffix}");

        std::env::remove_var(&new_key);
        std::env::remove_var(&legacy_key);
        assert_eq!(env(suffix), None, "unset on both prefixes ⇒ None");

        // Legacy-only ⇒ falls back.
        std::env::set_var(&legacy_key, "legacy-value");
        assert_eq!(env(suffix).as_deref(), Some("legacy-value"));

        // New prefix wins over legacy when both are set.
        std::env::set_var(&new_key, "new-value");
        assert_eq!(env(suffix).as_deref(), Some("new-value"));

        // Empty new value is treated as unset ⇒ legacy still wins.
        std::env::set_var(&new_key, "");
        assert_eq!(env(suffix).as_deref(), Some("legacy-value"));

        std::env::remove_var(&new_key);
        std::env::remove_var(&legacy_key);
    }

    #[test]
    fn config_dir_prefers_current_slug() {
        let base = unique_tmp("current");
        std::fs::create_dir_all(base.join(SLUG)).unwrap();
        std::fs::create_dir_all(base.join(LEGACY_SLUG)).unwrap();
        // Both exist ⇒ current wins.
        assert_eq!(resolve_config_dir(&base), base.join(SLUG));
        std::fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn config_dir_falls_back_to_legacy_when_current_absent() {
        let base = unique_tmp("legacy");
        std::fs::create_dir_all(base.join(LEGACY_SLUG)).unwrap();
        // Only legacy exists ⇒ read-fallback returns it.
        assert_eq!(resolve_config_dir(&base), base.join(LEGACY_SLUG));
        std::fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn config_dir_defaults_to_current_when_neither_exists() {
        let base = unique_tmp("neither");
        // Neither present ⇒ the (to-be-created) current path.
        assert_eq!(resolve_config_dir(&base), base.join(SLUG));
        std::fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn install_root_default_is_tv_shell() {
        assert_eq!(install_root_default(), PathBuf::from("/opt/tv-shell"));
    }
}
