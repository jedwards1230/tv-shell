//! Typed daemon configuration: `~/.config/game-shell/config.toml`.
//!
//! Replaces the old `daemon.env` `KEY=VALUE` file that `game-shell-session.sh`
//! sourced into the environment. The daemon now reads a typed TOML document
//! directly, so the config surface is parsed once, validated at startup, and
//! never leaks the bearer token into the process environment (where any child
//! subprocess — grim, quickshell — would inherit it).
//!
//! ## Layout
//!
//! ```toml
//! [http]                      # LAN HTTP control bridge (opt-in)
//! bind = "127.0.0.1:8089"     #   absent ⇒ bridge off
//! auth_enabled = true         #   default true; only disable on a trusted LAN
//! token_file = "~/.config/game-shell/http-token"  # 0600; the shared bearer token
//!
//! [mcp]                       # MCP server (opt-in; needs --features mcp)
//! bind = "127.0.0.1:8090"     #   absent ⇒ off; shares [http].token_file + auth
//! dev = false                 #   dev tools (build/restart/deploy) — keep off in prod
//! allowed_hosts = ["my-host.local"]   # Host-header allowlist (DNS-rebind guard)
//!
//! [cec]                       # HDMI-CEC lifecycle (needs --features cec)
//! lifecycle = false
//!
//! [plex]                      # Plex home-screen widget (optional)
//! url = "http://plex:32400"
//! token_file = "~/.config/game-shell/plex-token"   # or: token = "…"
//!
//! [steam]                     # Steam library row (optional)
//! url = "http://gaming-pc:47995"
//! token_file = "~/.config/game-shell/steam-token"  # or: token = "…"
//!
//! [dev]                       # operator escape hatch
//! allow_insecure_lan = false  # see validate(): permit LAN + dev + no-auth on purpose
//! ```
//!
//! ## Security
//!
//! The shared bearer token is **by reference only** (`[http].token_file`), never
//! inline, and the referenced file must be private (mode `0600`). A
//! missing/empty/world-readable token file means "no token", which — with auth
//! enabled — fails closed (all requests 401).
//!
//! [`DaemonConfig::validate`] REFUSES to run with the dangerous combination of a
//! non-loopback bind + dev tools + effectively-disabled auth (for BOTH the HTTP
//! bridge and the MCP server), unless the operator has explicitly opted in with
//! `[dev].allow_insecure_lan = true`.
//!
//! Cross-platform: pure parsing/validation, unit-tested on every host.

use serde::Deserialize;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// Process-global parsed config, populated once at startup by [`init_global`].
/// The standalone command handlers (plex/steam/cec/http) read their settings
/// from here instead of `std::env::var`, so the typed config is the single
/// source of truth without threading a `&DaemonConfig` through every call site.
static GLOBAL: OnceLock<DaemonConfig> = OnceLock::new();

/// Install the process-global config (call once, at startup, after `validate`).
/// A second call is ignored (the first wins) — tests that don't init see the
/// default via [`global`].
pub fn init_global(config: DaemonConfig) {
    let _ = GLOBAL.set(config);
}

/// Borrow the process-global config. Before [`init_global`] runs (e.g. in unit
/// tests of the standalone modules) this returns a shared all-default config, so
/// callers never panic and behave as "everything off / not configured".
pub fn global() -> &'static DaemonConfig {
    GLOBAL.get_or_init(DaemonConfig::default)
}

/// The full typed daemon configuration. Every section is optional; an empty or
/// missing `config.toml` yields all-default (everything off), matching the old
/// "no daemon.env ⇒ shell still boots, no control surface" behavior.
#[derive(Debug, Default, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct DaemonConfig {
    pub http: HttpConfig,
    pub mcp: McpConfig,
    pub cec: CecConfig,
    pub plex: PlexConfig,
    pub steam: SteamConfig,
    pub observability: ObservabilityConfig,
    pub dev: DevConfig,
}

/// `[http]` — the LAN HTTP control bridge.
#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct HttpConfig {
    /// Bind address, e.g. `127.0.0.1:8089`. `None` ⇒ bridge off.
    pub bind: Option<String>,
    /// Require a bearer token. Default `true` (secure by default).
    pub auth_enabled: bool,
    /// Path to a 0600 file holding the shared bearer token. Never inline.
    pub token_file: Option<String>,
}

impl Default for HttpConfig {
    fn default() -> Self {
        Self {
            bind: None,
            auth_enabled: true,
            token_file: None,
        }
    }
}

/// `[mcp]` — the MCP server (shares the HTTP bridge's token + auth).
#[derive(Debug, Default, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct McpConfig {
    /// Bind address, e.g. `127.0.0.1:8090`. `None` ⇒ off.
    pub bind: Option<String>,
    /// Enable the dev tool surface (build/restart/deploy). Default `false`.
    pub dev: bool,
    /// Host-header allowlist (DNS-rebinding guard). Empty ⇒ allow-all (token-gated).
    pub allowed_hosts: Vec<String>,
}

/// `[cec]` — HDMI-CEC lifecycle.
#[derive(Debug, Default, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct CecConfig {
    /// Wake the AV chain on start/resume and standby on suspend. Default `false`.
    pub lifecycle: bool,
}

/// `[plex]` — Plex home-screen widget. Both a URL and a token are required for
/// the widget to function (the QML collapses it otherwise).
#[derive(Debug, Default, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct PlexConfig {
    pub url: Option<String>,
    /// Inline token (convenient for an optional widget) …
    pub token: Option<String>,
    /// … or a path to a file holding the token (preferred; keep it 0600).
    pub token_file: Option<String>,
}

/// `[steam]` — Steam library row, pointing at a `game-shell-host` sidecar.
#[derive(Debug, Default, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SteamConfig {
    pub url: Option<String>,
    pub token: Option<String>,
    pub token_file: Option<String>,
}

/// `[observability]` — logs + metrics emission (#268).
///
/// `RUST_LOG` is deliberately NOT modelled here: it's the standard
/// `tracing-subscriber` EnvFilter variable, read directly at logging init, and
/// kept as an env var so the usual `RUST_LOG=debug game-shell-input` workflow
/// still works. Everything else that used to be a `GAME_SHELL_*` env var is
/// typed config now.
#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ObservabilityConfig {
    /// Logging backend: `Some(true)` forces the systemd journal
    /// (tracing-journald), `Some(false)` forces plain stdout, `None` = auto
    /// (journald when `JOURNAL_STREAM` indicates a systemd-spawned service).
    /// Was `GAME_SHELL_LOG_JOURNAL`.
    pub log_journal: Option<bool>,
    /// node_exporter textfile-collector output path (the PRIMARY metrics path).
    /// `None` ⇒ the textfile writer is disabled (the `/metrics` HTTP route, when
    /// the bridge is bound, is unaffected). Was `GAME_SHELL_METRICS_TEXTFILE`.
    pub metrics_textfile: Option<String>,
    /// Textfile render/write interval in seconds. Was `GAME_SHELL_METRICS_INTERVAL`.
    pub metrics_interval: u64,
}

impl Default for ObservabilityConfig {
    fn default() -> Self {
        Self {
            log_journal: None,
            metrics_textfile: None,
            // Mirrors metrics.rs DEFAULT_INTERVAL_SECS (15).
            metrics_interval: 15,
        }
    }
}

/// `[dev]` — operator escape hatches.
#[derive(Debug, Default, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct DevConfig {
    /// Explicitly permit the otherwise-refused LAN + dev-tools + no-auth combo.
    /// This is how game-client-1 keeps its intentional insecure dev loop.
    pub allow_insecure_lan: bool,
}

/// Default config path: `~/.config/game-shell/config.toml`.
pub fn config_path() -> PathBuf {
    config_dir().join("config.toml")
}

/// `${XDG_CONFIG_HOME:-$HOME/.config}/game-shell`.
fn config_dir() -> PathBuf {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            std::env::var_os("HOME")
                .map(PathBuf::from)
                .unwrap_or_default()
                .join(".config")
        });
    base.join("game-shell")
}

/// Expand a leading `~/` (or bare `~`) in a config path to `$HOME`. Other paths
/// pass through unchanged. Token/config files are commonly written with `~`.
fn expand_tilde(p: &str) -> PathBuf {
    if let Some(rest) = p.strip_prefix("~/") {
        if let Some(home) = std::env::var_os("HOME") {
            return PathBuf::from(home).join(rest);
        }
    }
    if p == "~" {
        if let Some(home) = std::env::var_os("HOME") {
            return PathBuf::from(home);
        }
    }
    PathBuf::from(p)
}

impl DaemonConfig {
    /// Load and parse `config.toml`. A missing file is not an error — it yields
    /// the all-default config (everything off), so a fresh install still boots.
    /// A present-but-malformed file IS an error (the operator should know their
    /// config was ignored rather than silently running with defaults).
    pub fn load() -> anyhow::Result<Self> {
        Self::load_from(&config_path())
    }

    /// Load from an explicit path (testable; no env/global state).
    pub fn load_from(path: &Path) -> anyhow::Result<Self> {
        match std::fs::read_to_string(path) {
            Ok(text) => Self::parse(&text),
            // Absent ⇒ defaults. Any other read error (perms, etc.) surfaces.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(anyhow::anyhow!("reading {}: {e}", path.display())),
        }
    }

    /// Parse a TOML document (no I/O).
    pub fn parse(text: &str) -> anyhow::Result<Self> {
        toml::from_str(text).map_err(|e| anyhow::anyhow!("parsing config.toml: {e}"))
    }

    /// Parse the HTTP bind address, if any. `Err` on a malformed address so the
    /// operator gets a clear startup failure rather than a silently-off bridge.
    pub fn http_bind(&self) -> anyhow::Result<Option<SocketAddr>> {
        parse_bind("http.bind", self.http.bind.as_deref())
    }

    /// Parse the MCP bind address, if any.
    pub fn mcp_bind(&self) -> anyhow::Result<Option<SocketAddr>> {
        parse_bind("mcp.bind", self.mcp.bind.as_deref())
    }

    /// Resolve the shared bearer token from `[http].token_file`. Returns `None`
    /// when no token file is configured, the file is missing, or it is empty —
    /// all of which mean "no token" (fail-closed when auth is enabled).
    ///
    /// Reads (and warns) if the token file is group/other-accessible: a bearer
    /// token in a world-readable file is a leak, but we still load it (the
    /// operator's intent is clear) and log loudly so it's noticed.
    pub fn http_token(&self) -> Option<String> {
        let path = self.http.token_file.as_deref()?;
        read_token_file(&expand_tilde(path), "http.token_file")
    }

    /// Resolve the Plex token: inline `token` wins, else `token_file`.
    pub fn plex_token(&self) -> Option<String> {
        resolve_service_token(
            self.plex.token.as_deref(),
            self.plex.token_file.as_deref(),
            "plex.token_file",
        )
    }

    /// Resolve the Steam token: inline `token` wins, else `token_file`.
    pub fn steam_token(&self) -> Option<String> {
        resolve_service_token(
            self.steam.token.as_deref(),
            self.steam.token_file.as_deref(),
            "steam.token_file",
        )
    }

    /// Metrics textfile write interval in seconds, clamped to ≥1 so a `0` (which
    /// would busy-loop the writer) falls back to the 15s default — mirroring the
    /// old `interval_secs()` env parser's `filter(|&n| n > 0)`.
    pub fn metrics_interval_secs(&self) -> u64 {
        let n = self.observability.metrics_interval;
        if n == 0 {
            15
        } else {
            n
        }
    }

    /// Validate cross-field invariants, refusing to run in a configuration that
    /// would expose an unauthenticated remote-control / RCE surface on the LAN.
    ///
    /// For BOTH the HTTP bridge and the MCP server, the dangerous combination is:
    /// a **non-loopback** bind + **dev tools** enabled + auth **effectively
    /// disabled** (auth off, or no token resolvable). The HTTP bridge's dev tools
    /// are its `/dev/*` endpoints (build/deploy/restart); the MCP server's are
    /// gated by `[mcp].dev`. Returning `Err` here aborts startup.
    ///
    /// `[dev].allow_insecure_lan = true` is an explicit operator opt-in that
    /// downgrades the refusal to a loud warning — this is how game-client-1 keeps
    /// its intentional LAN + dev + no-auth dev loop.
    pub fn validate(&self) -> anyhow::Result<()> {
        let token = self.http_token();
        let auth_effectively_disabled = !self.http.auth_enabled || token.is_none();

        // The HTTP bridge always exposes its /dev/* tools, so a non-loopback
        // bridge with no auth is an unauthenticated RCE surface regardless of MCP.
        if let Some(addr) = self.http_bind()? {
            if !addr.ip().is_loopback() && auth_effectively_disabled {
                self.refuse_or_warn(
                    "HTTP control bridge",
                    addr,
                    "its /dev/* endpoints (build/deploy/restart) are an unauthenticated RCE surface",
                )?;
            }
        }

        // The MCP server only exposes dev tools when [mcp].dev is set; without
        // dev tools an unauthenticated MCP surface is still a remote-control leak
        // but not RCE — match the existing mcp.rs refusal which gated on dev.
        if let Some(addr) = self.mcp_bind()? {
            if self.mcp.dev && !addr.ip().is_loopback() && auth_effectively_disabled {
                self.refuse_or_warn(
                    "MCP server",
                    addr,
                    "GAME_SHELL dev tools over MCP are an unauthenticated RCE surface",
                )?;
            }
        }

        Ok(())
    }

    /// Either return an error (refuse to start) or, when the operator has opted
    /// into `[dev].allow_insecure_lan`, log a loud warning and continue.
    fn refuse_or_warn(&self, surface: &str, addr: SocketAddr, why: &str) -> anyhow::Result<()> {
        if self.dev.allow_insecure_lan {
            tracing::warn!(
                "config: {surface} bound to non-loopback {addr} with auth effectively \
                 disabled — {why}. Permitted only because [dev].allow_insecure_lan = true."
            );
            Ok(())
        } else {
            Err(anyhow::anyhow!(
                "refusing to start: {surface} is bound to non-loopback {addr} with auth \
                 effectively disabled (no token / auth off) — {why}. Set [http].token_file \
                 (0600) and [http].auth_enabled = true, bind to 127.0.0.1, or explicitly \
                 opt in with [dev].allow_insecure_lan = true."
            ))
        }
    }
}

/// Parse an optional `host:port` bind string into a `SocketAddr`.
fn parse_bind(field: &str, value: Option<&str>) -> anyhow::Result<Option<SocketAddr>> {
    match value {
        None => Ok(None),
        Some(s) => s
            .parse::<SocketAddr>()
            .map(Some)
            .map_err(|e| anyhow::anyhow!("{field} = {s:?} is not a valid host:port address: {e}")),
    }
}

/// Resolve a service token: a non-empty inline `token` wins; otherwise read the
/// `token_file`. `None` when neither yields a non-empty value.
fn resolve_service_token(
    inline: Option<&str>,
    token_file: Option<&str>,
    field: &str,
) -> Option<String> {
    if let Some(t) = inline.filter(|t| !t.is_empty()) {
        return Some(t.to_string());
    }
    read_token_file(&expand_tilde(token_file?), field)
}

/// Read a bearer/API token from a file: trim trailing whitespace/newline, treat
/// empty as absent. Warns if the file is group/other-accessible (mode & 0o077).
fn read_token_file(path: &Path, field: &str) -> Option<String> {
    let raw = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!("config: {field} {} unreadable: {e}", path.display());
            return None;
        }
    };
    warn_if_world_accessible(path, field);
    let token = raw.trim();
    if token.is_empty() {
        tracing::warn!(
            "config: {field} {} is empty — treating as no token",
            path.display()
        );
        None
    } else {
        Some(token.to_string())
    }
}

/// Log a warning if a token file is readable by group/other (mode & 0o077 != 0).
/// Unix-only check; a no-op elsewhere.
#[cfg(unix)]
fn warn_if_world_accessible(path: &Path, field: &str) {
    use std::os::unix::fs::PermissionsExt;
    if let Ok(meta) = std::fs::metadata(path) {
        let mode = meta.permissions().mode();
        if mode & 0o077 != 0 {
            tracing::warn!(
                "config: {field} {} is group/other-accessible (mode {:o}); a bearer token \
                 should be 0600 (chmod 600 it)",
                path.display(),
                mode & 0o7777
            );
        }
    }
}

#[cfg(not(unix))]
fn warn_if_world_accessible(_path: &Path, _field: &str) {}

#[cfg(test)]
mod tests {
    use super::*;

    fn loopback_http(dev_allow: bool, bind: &str, auth: bool) -> DaemonConfig {
        let mut c = DaemonConfig::default();
        c.http.bind = Some(bind.to_string());
        c.http.auth_enabled = auth;
        c.dev.allow_insecure_lan = dev_allow;
        c
    }

    #[test]
    fn empty_config_is_all_default_off() {
        let c = DaemonConfig::parse("").unwrap();
        assert!(c.http.bind.is_none());
        assert!(c.http.auth_enabled); // secure by default
        assert!(c.mcp.bind.is_none());
        assert!(!c.mcp.dev);
        assert!(!c.cec.lifecycle);
        assert!(!c.dev.allow_insecure_lan);
        // Observability defaults: auto log backend, no textfile, 15s interval.
        assert_eq!(c.observability.log_journal, None);
        assert!(c.observability.metrics_textfile.is_none());
        assert_eq!(c.metrics_interval_secs(), 15);
        c.validate().unwrap(); // nothing bound ⇒ trivially valid
    }

    #[test]
    fn observability_section_parses_and_interval_clamps() {
        let c = DaemonConfig::parse(
            r#"
            [observability]
            log_journal = true
            metrics_textfile = "/var/lib/node_exporter/textfile/game-shell.prom"
            metrics_interval = 30
        "#,
        )
        .unwrap();
        assert_eq!(c.observability.log_journal, Some(true));
        assert_eq!(
            c.observability.metrics_textfile.as_deref(),
            Some("/var/lib/node_exporter/textfile/game-shell.prom")
        );
        assert_eq!(c.metrics_interval_secs(), 30);

        // A zero interval (busy-loop) clamps back to the 15s default.
        let z = DaemonConfig::parse("[observability]\nmetrics_interval = 0\n").unwrap();
        assert_eq!(z.metrics_interval_secs(), 15);

        // log_journal = false forces stdout.
        let f = DaemonConfig::parse("[observability]\nlog_journal = false\n").unwrap();
        assert_eq!(f.observability.log_journal, Some(false));
    }

    #[test]
    fn full_config_parses() {
        let toml = r#"
            [http]
            bind = "127.0.0.1:8089"
            auth_enabled = true
            token_file = "/run/secrets/http-token"

            [mcp]
            bind = "127.0.0.1:8090"
            dev = true
            allowed_hosts = ["localhost", "my-host.local"]

            [cec]
            lifecycle = true

            [plex]
            url = "http://plex:32400"
            token = "plex-tok"

            [steam]
            url = "http://gaming-pc:47995"
            token_file = "/run/secrets/steam-token"

            [dev]
            allow_insecure_lan = false
        "#;
        let c = DaemonConfig::parse(toml).unwrap();
        assert_eq!(c.http.bind.as_deref(), Some("127.0.0.1:8089"));
        assert_eq!(c.mcp.allowed_hosts, vec!["localhost", "my-host.local"]);
        assert!(c.mcp.dev);
        assert!(c.cec.lifecycle);
        assert_eq!(c.plex.url.as_deref(), Some("http://plex:32400"));
        assert_eq!(c.plex_token().as_deref(), Some("plex-tok")); // inline wins
        assert_eq!(c.http_bind().unwrap().unwrap().port(), 8089);
    }

    #[test]
    fn unknown_field_is_rejected() {
        // deny_unknown_fields catches typos (e.g. a stale daemon.env-era key).
        assert!(DaemonConfig::parse("[http]\nbnid = \"x\"\n").is_err());
        assert!(DaemonConfig::parse("[bogus]\nx = 1\n").is_err());
    }

    #[test]
    fn malformed_bind_is_an_error_not_silently_off() {
        let mut c = DaemonConfig::default();
        c.http.bind = Some("not-an-addr".to_string());
        assert!(c.http_bind().is_err());
    }

    #[test]
    fn validate_refuses_lan_http_without_auth() {
        // Non-loopback HTTP bridge + auth off + no opt-in ⇒ refuse.
        let c = loopback_http(false, "0.0.0.0:8089", false);
        let err = c.validate().unwrap_err().to_string();
        assert!(err.contains("refusing to start"), "got: {err}");
        assert!(err.contains("HTTP control bridge"), "got: {err}");
    }

    #[test]
    fn validate_allows_lan_http_loopback() {
        // Loopback bind is always fine even with auth off.
        let c = loopback_http(false, "127.0.0.1:8089", false);
        c.validate().unwrap();
    }

    #[test]
    fn validate_escape_hatch_downgrades_to_warning() {
        // allow_insecure_lan = true ⇒ the same dangerous combo is permitted.
        let c = loopback_http(true, "0.0.0.0:8089", false);
        c.validate().unwrap();
    }

    #[test]
    fn validate_mcp_dev_lan_no_auth_refused() {
        let mut c = DaemonConfig::default();
        c.mcp.bind = Some("0.0.0.0:8090".to_string());
        c.mcp.dev = true;
        c.http.auth_enabled = false; // shared auth disabled
        let err = c.validate().unwrap_err().to_string();
        assert!(err.contains("MCP server"), "got: {err}");
    }

    #[test]
    fn validate_mcp_lan_no_dev_no_auth_is_allowed() {
        // Without dev tools, an unauthenticated MCP bind is a remote-control leak
        // but not RCE; matches the prior mcp.rs refusal which gated on dev.
        let mut c = DaemonConfig::default();
        c.mcp.bind = Some("0.0.0.0:8090".to_string());
        c.mcp.dev = false;
        c.http.auth_enabled = false;
        c.validate().unwrap();
    }

    #[test]
    fn validate_lan_http_with_token_ok() {
        // A resolvable token + auth on ⇒ not "effectively disabled" ⇒ allowed.
        // (Use an inline-equivalent by pointing token_file at a temp file.)
        let dir = std::env::temp_dir().join(format!(
            "gs-cfgtok-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::create_dir_all(&dir);
        let tok = dir.join("http-token");
        std::fs::write(&tok, "a-long-secret\n").unwrap();
        let mut c = DaemonConfig::default();
        c.http.bind = Some("0.0.0.0:8089".to_string());
        c.http.auth_enabled = true;
        c.http.token_file = Some(tok.to_string_lossy().into_owned());
        assert_eq!(c.http_token().as_deref(), Some("a-long-secret"));
        c.validate().unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }
}
