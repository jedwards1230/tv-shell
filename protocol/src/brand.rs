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

/// Per-file backfill core: copy each of `names` from `legacy` into `current`
/// when it's missing there. Injectable (both dirs passed in) so it's
/// deterministic and unit-testable with tempdirs — it does its own I/O but reads
/// no env and logs nothing (the protocol crate has no tracing dep).
///
/// For each `name`: `dst = current.join(name)`, `src = legacy.join(name)`.
/// - Skip if `dst` already exists (NEVER overwrite an existing target) or if
///   `src` is not a regular file (nothing to migrate).
/// - Ensure `current` exists (`create_dir_all`); on error, skip this name.
/// - `std::fs::copy(src, dst)`. On Unix `std::fs::copy` also copies the source's
///   permission bits to the destination, so the mode is preserved for free. On
///   copy error, skip this name.
///
/// Returns the filenames actually copied, in `names` order.
fn migrate_files_between(current: &Path, legacy: &Path, names: &[&str]) -> Vec<String> {
    let mut migrated = Vec::new();
    for name in names {
        let dst = current.join(name);
        let src = legacy.join(name);
        // Never clobber an existing target; nothing to do if the source is absent.
        if dst.exists() || !src.is_file() {
            continue;
        }
        // The current dir may not exist yet (config_dir() returns a not-yet-
        // created path when neither slug's dir is present — though the public
        // wrapper short-circuits that case, this keeps the core self-contained).
        if std::fs::create_dir_all(current).is_err() {
            continue;
        }
        // std::fs::copy preserves the source's Unix permission bits on the
        // destination, so no explicit set_permissions is needed.
        if std::fs::copy(&src, &dst).is_ok() {
            migrated.push(name.to_string());
        }
    }
    migrated
}

/// One-time, per-file backfill of legacy `game-shell` state into the current
/// `tv-shell` config dir. Returns the filenames actually copied so the caller
/// can log each (the protocol crate itself logs nothing).
///
/// **Idempotent and non-destructive:** a file is copied only when it is MISSING
/// in the current dir and PRESENT (as a regular file) in the legacy dir — an
/// existing target is never overwritten, so re-running this is a no-op once the
/// backfill has happened (or once the user has written fresh state).
///
/// **Why this exists on top of [`config_dir`]'s dir-level fallback:** that
/// fallback only fires when the whole new `tv-shell` dir is ABSENT. When a
/// deployment (Ansible) pre-creates `~/.config/tv-shell/` to drop in
/// deployment-owned files (`config.toml`/`targets.json`/tokens), the dir-level
/// fallback stops helping and the shell would write fresh defaults, silently
/// discarding real user state (`settings.json`, `host-macs.json`) still living
/// in `~/.config/game-shell/`. This copies those specific files across once.
///
/// **The `current == legacy` short-circuit:** [`config_dir`] returns the LEGACY
/// dir (via its dir-level fallback) when the new dir doesn't exist yet — in that
/// case we're already reading legacy directly, so there is nothing to migrate
/// and we return empty. Only when the new dir actually exists does `config_dir`
/// return it (differing from `legacy`), triggering the per-file backfill. The
/// legacy dir is derived from [`LEGACY_SLUG`] (not a hardcoded path) so it stays
/// in lockstep with [`config_dir`]'s own base + slug resolution.
pub fn migrate_legacy_config_files(names: &[&str]) -> Vec<String> {
    let current = config_dir();
    let legacy = config_base().join(LEGACY_SLUG);
    if current == legacy {
        // config_dir() fell back to the legacy dir (new dir absent) — we're
        // already reading legacy directly, so there is nothing to backfill.
        return Vec::new();
    }
    migrate_files_between(&current, &legacy, names)
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

    #[test]
    fn migrate_backfills_legacy_only_file_preserving_mode() {
        let base = unique_tmp("migrate-legacy-only");
        let current = base.join(SLUG);
        let legacy = base.join(LEGACY_SLUG);
        std::fs::create_dir_all(&current).unwrap();
        std::fs::create_dir_all(&legacy).unwrap();
        let src = legacy.join("settings.json");
        std::fs::write(&src, r#"{"themeMode":"dark"}"#).unwrap();
        // Set a non-default mode on the source so we can assert it survives the copy.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&src, std::fs::Permissions::from_mode(0o600)).unwrap();
        }

        let migrated = migrate_files_between(&current, &legacy, &["settings.json"]);
        assert_eq!(migrated, vec!["settings.json".to_string()]);

        let dst = current.join("settings.json");
        assert_eq!(
            std::fs::read_to_string(&dst).unwrap(),
            r#"{"themeMode":"dark"}"#
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let src_mode = std::fs::metadata(&src).unwrap().permissions().mode() & 0o777;
            let dst_mode = std::fs::metadata(&dst).unwrap().permissions().mode() & 0o777;
            assert_eq!(
                dst_mode, src_mode,
                "std::fs::copy preserves the source mode"
            );
        }

        std::fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn migrate_never_overwrites_existing_target() {
        let base = unique_tmp("migrate-both");
        let current = base.join(SLUG);
        let legacy = base.join(LEGACY_SLUG);
        std::fs::create_dir_all(&current).unwrap();
        std::fs::create_dir_all(&legacy).unwrap();
        // Both dirs have the file, with DIFFERENT contents.
        std::fs::write(current.join("settings.json"), "current").unwrap();
        std::fs::write(legacy.join("settings.json"), "legacy").unwrap();

        let migrated = migrate_files_between(&current, &legacy, &["settings.json"]);
        assert!(migrated.is_empty(), "existing target must not migrate");
        assert_eq!(
            std::fs::read_to_string(current.join("settings.json")).unwrap(),
            "current",
            "existing target must be left untouched"
        );

        std::fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn migrate_noop_when_neither_has_the_file() {
        let base = unique_tmp("migrate-neither");
        let current = base.join(SLUG);
        let legacy = base.join(LEGACY_SLUG);
        std::fs::create_dir_all(&current).unwrap();
        std::fs::create_dir_all(&legacy).unwrap();

        let migrated = migrate_files_between(&current, &legacy, &["settings.json"]);
        assert!(migrated.is_empty());
        assert!(
            !current.join("settings.json").exists(),
            "no file should be created when the source is absent"
        );

        std::fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn migrate_multi_file_only_present_ones() {
        let base = unique_tmp("migrate-multi");
        let current = base.join(SLUG);
        let legacy = base.join(LEGACY_SLUG);
        std::fs::create_dir_all(&current).unwrap();
        std::fs::create_dir_all(&legacy).unwrap();
        // Only host-macs.json exists in legacy; settings.json does not.
        std::fs::write(legacy.join("host-macs.json"), "{}").unwrap();

        let migrated =
            migrate_files_between(&current, &legacy, &["settings.json", "host-macs.json"]);
        assert_eq!(migrated, vec!["host-macs.json".to_string()]);
        assert!(current.join("host-macs.json").exists());
        assert!(!current.join("settings.json").exists());

        std::fs::remove_dir_all(&base).ok();
    }
}
