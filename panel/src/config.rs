//! Panel configuration: reads the daemon's `~/.config/tv-shell/config.toml`
//! for the `[panel]` and `[http]` sections only.
//!
//! The panel cannot depend on the `tv-shell-input` daemon crate (it pulls a
//! Linux-only evdev/zbus/bluer/cec graph), so it parses the shared
//! `config.toml` itself with a PERMISSIVE deserializer: only the two sections
//! it needs are declared, every other section (`[mcp]`, `[cec]`, `[plex]`,
//! `[steam]`, `[observability]`, `[input]`, `[dev]`, ...) is silently ignored
//! by serde's default "unknown fields are OK" behavior (no
//! `deny_unknown_fields` anywhere in this module).
//!
//! ```toml
//! [panel]
//! enabled = true
//! bind = "127.0.0.1:8091"
//! token_file = "~/.config/tv-shell/panel-token"  # parsed, unused in v1 (no auth)
//!
//! [http]
//! bind = "127.0.0.1:8089"
//! token_file = "~/.config/tv-shell/http-token"
//! ```
//!
//! Loading never panics and never blocks boot: a missing file yields all
//! defaults, a malformed file logs a warning and falls back to defaults too —
//! the panel must always come up so an operator can reach the Dev recovery
//! page even when config.toml is broken.

use std::net::SocketAddr;
use std::path::PathBuf;

use serde::Deserialize;

/// Default panel bind address (loopback-only; LAN exposure is the operator's
/// choice via `[panel].bind` in config.toml).
const DEFAULT_PANEL_BIND: &str = "127.0.0.1:8091";

/// `[panel]` section of `config.toml`.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct PanelConfig {
    pub enabled: bool,
    pub bind: String,
    pub token_file: Option<String>,
}

impl Default for PanelConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            bind: DEFAULT_PANEL_BIND.to_string(),
            token_file: None,
        }
    }
}

/// `[http]` section of `config.toml` (the daemon's opt-in LAN HTTP bridge).
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct HttpSection {
    pub bind: Option<String>,
    pub token_file: Option<String>,
}

/// Top-level shape captured from `config.toml`. Deliberately does NOT declare
/// the daemon's other sections (`mcp`, `cec`, `plex`, `steam`, `observability`,
/// `input`, `dev`) — serde ignores unknown top-level keys by default (no
/// `deny_unknown_fields`), so this struct tolerates the full daemon config
/// document unchanged.
#[derive(Debug, Clone, Default, Deserialize)]
struct RawConfig {
    #[serde(default)]
    panel: PanelConfig,
    #[serde(default)]
    http: HttpSection,
}

/// Resolved, ready-to-use panel configuration.
#[derive(Debug, Clone)]
pub struct AppConfig {
    /// Whether the panel should serve at all (`[panel].enabled`).
    pub enabled: bool,
    /// Resolved bind address for the panel's own HTTP listener.
    pub panel_bind: SocketAddr,
    /// Raw `[panel].bind` string, kept for diagnostics/logging.
    pub panel_bind_raw: String,
    /// `[panel].token_file`, parsed but unused for the panel's own auth (v1
    /// has no auth — LAN-only). Kept for a future milestone.
    pub panel_token_file: Option<String>,
    /// `Some("http://<http.bind>")` when the daemon's HTTP bridge is
    /// configured; `None` when `[http].bind` is absent (bridge off).
    pub http_bridge_base: Option<String>,
    /// The daemon HTTP bridge's bearer token, read from `[http].token_file`
    /// (tilde-expanded, trimmed). `None` on any error (missing/unreadable
    /// file, no `token_file` configured).
    pub http_token: Option<String>,
}

impl Default for AppConfig {
    fn default() -> Self {
        let panel = PanelConfig::default();
        let panel_bind = panel
            .bind
            .parse()
            .unwrap_or_else(|_| DEFAULT_PANEL_BIND.parse().expect("default bind is valid"));
        Self {
            enabled: panel.enabled,
            panel_bind,
            panel_bind_raw: panel.bind,
            panel_token_file: panel.token_file,
            http_bridge_base: None,
            http_token: None,
        }
    }
}

/// Path to the daemon's `config.toml` (`tv_shell_protocol::brand::config_dir()`).
fn config_path() -> PathBuf {
    tv_shell_protocol::brand::config_dir().join("config.toml")
}

/// Load and resolve the panel configuration. Never panics: a missing file
/// yields all defaults; a malformed file logs a warning and falls back to
/// defaults so the panel can still boot (and an operator can reach the Dev
/// recovery page) even with a broken config.toml.
pub fn load() -> AppConfig {
    let path = config_path();
    let raw = match std::fs::read_to_string(&path) {
        Ok(text) => match toml::from_str::<RawConfig>(&text) {
            Ok(cfg) => cfg,
            Err(e) => {
                tracing::warn!(
                    "panel: failed to parse {} — falling back to defaults: {e}",
                    path.display()
                );
                RawConfig::default()
            }
        },
        Err(_) => {
            // Missing file (or unreadable) ⇒ all defaults. Not worth a
            // warning: an absent config.toml is a normal fresh install.
            RawConfig::default()
        }
    };
    resolve(raw)
}

/// Resolve a parsed [`RawConfig`] into a ready-to-use [`AppConfig`].
fn resolve(raw: RawConfig) -> AppConfig {
    let panel_bind = raw.panel.bind.parse().unwrap_or_else(|e| {
        tracing::warn!(
            "panel: invalid [panel].bind {:?} ({e}) — falling back to {DEFAULT_PANEL_BIND}",
            raw.panel.bind
        );
        DEFAULT_PANEL_BIND.parse().expect("default bind is valid")
    });
    let http_bridge_base = raw
        .http
        .bind
        .as_deref()
        .filter(|s| !s.is_empty())
        .map(|bind| format!("http://{bind}"));
    let http_token = raw.http.token_file.as_deref().and_then(read_token_file);

    AppConfig {
        enabled: raw.panel.enabled,
        panel_bind,
        panel_bind_raw: raw.panel.bind,
        panel_token_file: raw.panel.token_file,
        http_bridge_base,
        http_token,
    }
}

/// Read a bearer token from a file path, tilde-expanding a leading `~/`.
/// Returns `None` on any error (missing file, unreadable, ...) or when the
/// trimmed content is empty.
fn read_token_file(path: &str) -> Option<String> {
    let expanded = expand_tilde(path);
    let content = std::fs::read_to_string(expanded).ok()?;
    let trimmed = content.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

/// Expand a leading `~/` to `$HOME/`. Paths without a leading `~/` pass
/// through unchanged.
fn expand_tilde(path: &str) -> PathBuf {
    if let Some(rest) = path.strip_prefix("~/") {
        if let Some(home) = std::env::var_os("HOME") {
            return PathBuf::from(home).join(rest);
        }
    }
    PathBuf::from(path)
}

/// Resolve the daemon's IPC Unix-socket path.
///
/// Preference order: `TV_SHELL_SOCK` (via [`tv_shell_protocol::brand::env`],
/// legacy `GAME_SHELL_SOCK` honored) → `$XDG_RUNTIME_DIR/<socket_name>` →
/// `/run/user/<uid>/<socket_name>` (uid from `libc::getuid()`).
pub fn socket_path() -> PathBuf {
    let name = tv_shell_protocol::brand::socket_name();
    if let Some(sock) = tv_shell_protocol::brand::env("SOCK") {
        return PathBuf::from(sock);
    }
    if let Some(runtime_dir) = std::env::var_os("XDG_RUNTIME_DIR") {
        if !runtime_dir.is_empty() {
            return PathBuf::from(runtime_dir).join(name);
        }
    }
    // SAFETY: libc::getuid() is always safe to call — POSIX defines it as
    // infallible (no error return, no invalid states), it takes no arguments,
    // and it only reads the caller's real UID.
    let uid = unsafe { libc::getuid() };
    PathBuf::from(format!("/run/user/{uid}/{name}"))
}

/// systemd unit name for the input daemon (`tv-shell-input.service`).
pub fn daemon_unit() -> String {
    format!("{}-input.service", tv_shell_protocol::brand::SLUG)
}

/// systemd unit name for the Quickshell shell (`tv-shell-quickshell.service`).
pub fn shell_unit() -> String {
    format!("{}-quickshell.service", tv_shell_protocol::brand::SLUG)
}

/// systemd unit name for the panel itself (`tv-shell-panel.service`).
pub fn panel_unit() -> String {
    format!("{}-panel.service", tv_shell_protocol::brand::SLUG)
}

/// `journalctl --user -u <unit>` target for the input daemon
/// (`tv-shell-input`, no `.service` suffix — matches unit-name-as-journal-tag
/// convention).
pub fn daemon_journal_unit() -> String {
    format!("{}-input", tv_shell_protocol::brand::SLUG)
}

/// `journalctl --user -t <tag>` target for the Quickshell shell — the
/// `SyslogIdentifier` the quickshell unit sets (`tv-shell-quickshell`).
///
/// Not yet wired into the M1 Logs page (which sources the shell log via the
/// HTTP bridge only, per spec, and degrades to an inline message rather than
/// falling back to the journal when the bridge is down). Reserved for a
/// future milestone (e.g. a direct-exec shell-log fallback).
#[allow(dead_code)]
pub fn shell_journal_tag() -> String {
    format!("{}-quickshell", tv_shell_protocol::brand::SLUG)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_panel_config_is_enabled_on_loopback() {
        let cfg = AppConfig::default();
        assert!(cfg.enabled);
        assert_eq!(cfg.panel_bind_raw, DEFAULT_PANEL_BIND);
        assert_eq!(cfg.panel_bind, DEFAULT_PANEL_BIND.parse().unwrap());
        assert!(cfg.http_bridge_base.is_none());
        assert!(cfg.http_token.is_none());
    }

    #[test]
    fn resolve_missing_sections_yields_defaults() {
        let raw = RawConfig::default();
        let cfg = resolve(raw);
        assert!(cfg.enabled);
        assert_eq!(cfg.panel_bind_raw, DEFAULT_PANEL_BIND);
    }

    #[test]
    fn resolve_parses_http_bind_into_bridge_base() {
        let mut raw = RawConfig::default();
        raw.http.bind = Some("127.0.0.1:8089".to_string());
        let cfg = resolve(raw);
        assert_eq!(
            cfg.http_bridge_base.as_deref(),
            Some("http://127.0.0.1:8089")
        );
    }

    #[test]
    fn resolve_empty_http_bind_string_is_treated_as_off() {
        let mut raw = RawConfig::default();
        raw.http.bind = Some(String::new());
        let cfg = resolve(raw);
        assert!(cfg.http_bridge_base.is_none());
    }

    #[test]
    fn resolve_falls_back_on_invalid_panel_bind() {
        let mut raw = RawConfig::default();
        raw.panel.bind = "not-an-addr".to_string();
        let cfg = resolve(raw);
        assert_eq!(cfg.panel_bind, DEFAULT_PANEL_BIND.parse().unwrap());
    }

    #[test]
    fn permissive_parse_ignores_unrelated_sections() {
        let toml_text = r#"
            [panel]
            enabled = false
            bind = "127.0.0.1:9000"

            [http]
            bind = "127.0.0.1:8089"

            [mcp]
            bind = "127.0.0.1:8090"
            dev = true

            [cec]
            lifecycle = true

            [plex]
            url = "http://plex:32400"

            [steam]
            url = "http://gaming-pc:47995"

            [observability]
            enabled = true

            [input]
            some_key = "some_value"

            [dev]
            allow_insecure_lan = true
        "#;
        let raw: RawConfig = toml::from_str(toml_text).expect("permissive parse should succeed");
        assert!(!raw.panel.enabled);
        assert_eq!(raw.panel.bind, "127.0.0.1:9000");
        assert_eq!(raw.http.bind.as_deref(), Some("127.0.0.1:8089"));
    }

    #[test]
    fn read_token_file_expands_tilde_and_trims() {
        let dir = std::env::temp_dir().join(format!("tv-shell-panel-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let token_path = dir.join("token");
        std::fs::write(&token_path, "  sekret-token\n").unwrap();

        let read = read_token_file(token_path.to_str().unwrap());
        assert_eq!(read.as_deref(), Some("sekret-token"));

        // Empty file ⇒ None.
        std::fs::write(&token_path, "   \n").unwrap();
        assert_eq!(read_token_file(token_path.to_str().unwrap()), None);

        // Missing file ⇒ None.
        std::fs::remove_file(&token_path).unwrap();
        assert_eq!(read_token_file(token_path.to_str().unwrap()), None);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn expand_tilde_prefixes_home() {
        let prev = std::env::var_os("HOME");
        std::env::set_var("HOME", "/home/testuser");
        assert_eq!(
            expand_tilde("~/config.toml"),
            PathBuf::from("/home/testuser/config.toml")
        );
        assert_eq!(
            expand_tilde("/absolute/path"),
            PathBuf::from("/absolute/path")
        );
        match prev {
            Some(v) => std::env::set_var("HOME", v),
            None => std::env::remove_var("HOME"),
        }
    }

    #[test]
    fn unit_and_journal_names_use_slug() {
        assert_eq!(daemon_unit(), "tv-shell-input.service");
        assert_eq!(shell_unit(), "tv-shell-quickshell.service");
        assert_eq!(panel_unit(), "tv-shell-panel.service");
        assert_eq!(daemon_journal_unit(), "tv-shell-input");
        assert_eq!(shell_journal_tag(), "tv-shell-quickshell");
    }

    #[test]
    fn socket_path_prefers_env_override() {
        std::env::set_var("TV_SHELL_SOCK", "/tmp/custom.sock");
        assert_eq!(socket_path(), PathBuf::from("/tmp/custom.sock"));
        std::env::remove_var("TV_SHELL_SOCK");
    }
}
