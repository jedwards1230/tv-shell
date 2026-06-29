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
    /// Path to a `0600` file holding the Plex token. Inline tokens are NOT
    /// supported (matching the HTTP bearer-token policy) — a secret pasted into
    /// config.toml leaks via backups/CI/config-management/shared-host reads.
    pub token_file: Option<String>,
}

/// `[steam]` — Steam library row, pointing at a `game-shell-host` sidecar.
#[derive(Debug, Default, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SteamConfig {
    pub url: Option<String>,
    /// Path to a `0600` file holding the Steam/host token. Inline tokens are NOT
    /// supported (same rationale as Plex / the HTTP bearer token).
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
    /// This is how htpc-1 keeps its intentional insecure dev loop.
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
/// pass through unchanged. Used for non-secret output paths (e.g.
/// `metrics_textfile`) where the operator may legitimately point outside the
/// config dir; secret token files go through [`resolve_token_path`] instead.
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

/// Resolve a **token file** path and confine it to the config dir (CWE-22 guard).
///
/// A token file holds a secret the daemon reads with its own privileges, so a
/// config writer must not be able to point it at arbitrary paths
/// (`../../../etc/shadow`, `/tmp/attacker`). After tilde-expansion the path is
/// canonicalized and required to live within `~/.config/game-shell/`; anything
/// escaping the config dir is a hard error (refuse startup). Canonicalizing also
/// resolves `..`/symlinks, so a symlink inside the config dir pointing out is
/// caught too.
///
/// NOTE: this is intentionally NOT applied to `metrics_textfile` — that is an
/// OUTPUT the operator legitimately points outside the config dir (e.g.
/// node_exporter's `/var/lib/node_exporter/textfile/`), not a secret the daemon
/// reads. Output-path safety there is bounded by filesystem permissions, not by
/// confinement to the config dir.
fn resolve_token_path(p: &str, field: &str) -> anyhow::Result<PathBuf> {
    let expanded = expand_tilde(p);
    let canonical = expanded.canonicalize().map_err(|e| {
        anyhow::anyhow!(
            "{field} {}: cannot resolve token file path: {e}",
            expanded.display()
        )
    })?;
    let config_dir = config_dir()
        .canonicalize()
        .map_err(|e| anyhow::anyhow!("cannot resolve config dir for {field} validation: {e}"))?;
    if !canonical.starts_with(&config_dir) {
        return Err(anyhow::anyhow!(
            "{field} {} escapes the config directory {} — a token file must live \
             under ~/.config/game-shell/ (refusing to read a secret from an \
             arbitrary path)",
            canonical.display(),
            config_dir.display()
        ));
    }
    Ok(canonical)
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

    /// Resolve the shared bearer token from `[http].token_file`. `Ok(None)` when
    /// no token file is configured or it is empty (both mean "no token" →
    /// fail-closed when auth is enabled). `Err` when the path escapes the config
    /// dir (CWE-22) or the file is group/other-accessible (fail-closed: a leaked
    /// token must abort startup, not run with a compromised secret).
    pub fn http_token(&self) -> anyhow::Result<Option<String>> {
        match self.http.token_file.as_deref() {
            Some(p) => read_token_file(
                &resolve_token_path(p, "http.token_file")?,
                "http.token_file",
            ),
            None => Ok(None),
        }
    }

    /// Resolve the Plex token from `[plex].token_file` (token-file only; inline
    /// tokens are not supported — see PlexConfig). Same fail-closed semantics as
    /// [`http_token`].
    pub fn plex_token(&self) -> anyhow::Result<Option<String>> {
        match self.plex.token_file.as_deref() {
            Some(p) => read_token_file(
                &resolve_token_path(p, "plex.token_file")?,
                "plex.token_file",
            ),
            None => Ok(None),
        }
    }

    /// Resolve the Steam token from `[steam].token_file` (token-file only).
    pub fn steam_token(&self) -> anyhow::Result<Option<String>> {
        match self.steam.token_file.as_deref() {
            Some(p) => read_token_file(
                &resolve_token_path(p, "steam.token_file")?,
                "steam.token_file",
            ),
            None => Ok(None),
        }
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
    /// downgrades the refusal to a loud warning — this is how htpc-1 keeps
    /// its intentional LAN + dev + no-auth dev loop.
    pub fn validate(&self) -> anyhow::Result<()> {
        // Resolve the token eagerly so a path-traversal / world-readable token
        // file aborts startup here (fail-closed), not silently as "no token".
        let token = self.http_token()?;
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
            // error!, not warn!: the escape hatch is a deliberate hole, and a
            // forgotten `allow_insecure_lan = true` (e.g. a copy-pasted dev
            // config) silently opens an unauthenticated RCE surface to the LAN.
            // Logging at error level makes that impossible to miss at startup.
            tracing::error!(
                "config: {surface} bound to non-loopback {addr} with auth effectively \
                 disabled — {why}. PERMITTED ONLY because [dev].allow_insecure_lan = true; \
                 remove it unless this box intentionally runs an unauthenticated LAN dev loop."
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

/// Read a bearer/API token from a (config-dir-confined) file: trim trailing
/// whitespace/newline, treat empty as `Ok(None)` ("no token" → fail-closed when
/// auth is on). A group/other-accessible file is a hard `Err` (fail-closed): a
/// world-readable secret lets any local user / co-hosted service assume daemon
/// privileges, so the daemon refuses to start rather than run with a leaked token.
fn read_token_file(path: &Path, field: &str) -> anyhow::Result<Option<String>> {
    ensure_owner_only(path, field)?;
    let raw = std::fs::read_to_string(path)
        .map_err(|e| anyhow::anyhow!("config: {field} {} unreadable: {e}", path.display()))?;
    let token = raw.trim();
    if token.is_empty() {
        tracing::warn!(
            "config: {field} {} is empty — treating as no token",
            path.display()
        );
        Ok(None)
    } else {
        Ok(Some(token.to_string()))
    }
}

/// Fail-closed if a token file is readable by group/other (mode & 0o077 != 0).
/// Unix-only check; a no-op elsewhere (non-Unix has no POSIX mode bits).
#[cfg(unix)]
fn ensure_owner_only(path: &Path, field: &str) -> anyhow::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let meta = std::fs::metadata(path)
        .map_err(|e| anyhow::anyhow!("config: {field} {} stat failed: {e}", path.display()))?;
    let mode = meta.permissions().mode();
    if mode & 0o077 != 0 {
        return Err(anyhow::anyhow!(
            "config: {field} {} is group/other-accessible (mode {:o}); refusing to \
             start — a bearer/API token must be private. Fix: chmod 600 {}",
            path.display(),
            mode & 0o7777,
            path.display()
        ));
    }
    Ok(())
}

#[cfg(not(unix))]
fn ensure_owner_only(_path: &Path, _field: &str) -> anyhow::Result<()> {
    Ok(())
}

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
            token_file = "/run/secrets/plex-token"

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
        // Plex/Steam are token-file only now (inline `token` is a rejected
        // unknown field — verified in inline_token_is_rejected below).
        assert_eq!(
            c.plex.token_file.as_deref(),
            Some("/run/secrets/plex-token")
        );
        assert_eq!(c.http_bind().unwrap().unwrap().port(), 8089);
    }

    #[test]
    fn inline_plex_steam_token_is_rejected() {
        // #5: inline tokens are not supported; deny_unknown_fields rejects them so
        // an operator can't paste a raw secret into config.toml.
        assert!(DaemonConfig::parse("[plex]\ntoken = \"x\"\n").is_err());
        assert!(DaemonConfig::parse("[steam]\ntoken = \"x\"\n").is_err());
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

    // Token-file tests mutate XDG_CONFIG_HOME (process-global, since config_dir()
    // reads it), so they serialize on this guard to stay parallel-safe.
    static ENV_GUARD: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// Run `f` with XDG_CONFIG_HOME pointed at a fresh temp dir whose
    /// `game-shell/` subdir exists; cleans up after. Serialized via ENV_GUARD.
    #[cfg(unix)]
    fn with_temp_config_dir(f: impl FnOnce(&std::path::Path)) {
        let _g = ENV_GUARD.lock().unwrap_or_else(|p| p.into_inner());
        let base = std::env::temp_dir().join(format!(
            "gs-cfgdir-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let gs = base.join("game-shell");
        std::fs::create_dir_all(&gs).unwrap();
        let prev = std::env::var_os("XDG_CONFIG_HOME");
        // SAFETY: serialized by ENV_GUARD; restored before returning.
        unsafe { std::env::set_var("XDG_CONFIG_HOME", &base) };
        f(&gs);
        match prev {
            Some(v) => unsafe { std::env::set_var("XDG_CONFIG_HOME", v) },
            None => unsafe { std::env::remove_var("XDG_CONFIG_HOME") },
        }
        let _ = std::fs::remove_dir_all(&base);
    }

    /// Write a token file at `<config-dir>/<name>` with mode `mode`.
    #[cfg(unix)]
    fn write_token(dir: &std::path::Path, name: &str, body: &str, mode: u32) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let p = dir.join(name);
        std::fs::write(&p, body).unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(mode)).unwrap();
        p
    }

    #[cfg(unix)]
    #[test]
    fn validate_lan_http_with_0600_token_ok() {
        // A resolvable 0600 token inside the config dir + auth on ⇒ not
        // "effectively disabled" ⇒ allowed.
        with_temp_config_dir(|gs| {
            let tok = write_token(gs, "http-token", "a-long-secret\n", 0o600);
            let mut c = DaemonConfig::default();
            c.http.bind = Some("0.0.0.0:8089".to_string());
            c.http.auth_enabled = true;
            c.http.token_file = Some(tok.to_string_lossy().into_owned());
            assert_eq!(c.http_token().unwrap().as_deref(), Some("a-long-secret"));
            c.validate().unwrap();
        });
    }

    #[cfg(unix)]
    #[test]
    fn token_file_world_readable_is_rejected() {
        // #4: a group/other-accessible token file fails closed (hard error).
        with_temp_config_dir(|gs| {
            let tok = write_token(gs, "http-token", "secret\n", 0o644);
            let mut c = DaemonConfig::default();
            c.http.token_file = Some(tok.to_string_lossy().into_owned());
            let err = c.http_token().unwrap_err().to_string();
            assert!(err.contains("group/other-accessible"), "got: {err}");
            // And validate() refuses to start because of it.
            c.http.bind = Some("0.0.0.0:8089".to_string());
            assert!(c.validate().is_err());
        });
    }

    #[cfg(unix)]
    #[test]
    fn token_file_outside_config_dir_is_rejected() {
        // #1 (CWE-22): a token path escaping the config dir is a hard error even
        // when the target exists and is 0600.
        with_temp_config_dir(|_gs| {
            // /etc/hostname exists on Linux CI; any readable file outside the
            // config dir works to prove the confinement check fires.
            let mut c = DaemonConfig::default();
            c.http.token_file = Some("/etc/hostname".to_string());
            let err = c.http_token().unwrap_err().to_string();
            assert!(
                err.contains("escapes the config directory") || err.contains("cannot resolve"),
                "got: {err}"
            );
        });
    }
}
