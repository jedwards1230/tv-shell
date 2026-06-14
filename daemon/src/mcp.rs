//! MCP server for the game-shell daemon, built on the official `rmcp` 1.7.0 crate.
//!
//! **Opt-in**: the server only starts when the `GAME_SHELL_MCP_BIND` environment
//! variable is set to a `host:port` address. When unset, no socket is opened.
//!
//! **Auth**: bearer-token auth at parity with the HTTP bridge. Uses the same
//! `GAME_SHELL_HTTP_TOKEN` and `GAME_SHELL_HTTP_AUTH_ENABLED` env vars so
//! operators only need one token. Applied via an axum middleware layer wrapping
//! the `/mcp` route; constant-time comparison via `bridge_core::ct_eq_str`.
//! If auth is enabled but no token is configured, all requests are rejected (fail
//! closed). `GAME_SHELL_MCP_ALLOWED_HOSTS` (comma-separated host[:port]) overrides
//! the rmcp host allowlist.
//!
//! **Dev tools**: `dev_deploy`, `dev_build`, and `dev_restart_daemon` are only
//! registered when `GAME_SHELL_MCP_DEV` is set (any non-empty value). This keeps
//! the production tool surface minimal.
//!
//! **Transport**: StreamableHttpService over axum, served at `/mcp`. The MCP
//! endpoint is at `http://<bind>/mcp`.
//!
//! **Feature-gated (cross-platform)**: this module is gated on
//! `#[cfg(feature = "mcp")]` only — `rmcp`, `axum`, and `tokio` are all
//! cross-platform, so the module compiles and typechecks on macOS dev boxes
//! just as well as on Linux. It is NOT `#[cfg(target_os = "linux")]`-gated.
//! Only the binary (`main.rs`) and the Linux-specific modules (evdev, uinput,
//! zbus, Hyprland IPC) carry an OS guard; the MCP server itself runs anywhere
//! tokio does.

use std::sync::Arc;

use axum::{
    extract::Request,
    http::{HeaderMap, StatusCode},
    middleware::{self, Next},
    response::Response,
};
use base64::Engine as _;
use rmcp::{
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{CallToolResult, Content, Implementation, ServerCapabilities, ServerInfo},
    schemars::{self, JsonSchema},
    tool, tool_handler, tool_router, Json, ServerHandler,
};
use serde::Deserialize;
use tokio::sync::{broadcast, mpsc};
use tokio_util::sync::CancellationToken;

use crate::bridge_core;
use crate::protocol::Event;
use crate::state::Control;

// ─── Parameter types ─────────────────────────────────────────────────────────

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SendIntentParams {
    /// The intent name. Bare names: home, home-tap, home-hold, menu, settings,
    /// power. Deep-link families (colon-delimited): settings:<page-slug>,
    /// overlay:<target (volume|network|session)>, app:<StartupWMClass>.
    /// To open a specific settings page, overlay, or app, prefer
    /// open_settings / open_overlay / launch_app.
    pub name: String,
}

/// Direction or action key for D-pad / keyboard navigation.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum NavKey {
    /// Move focus upward.
    Up,
    /// Move focus downward.
    Down,
    /// Move focus left.
    Left,
    /// Move focus right.
    Right,
    /// Activate / confirm the currently focused element (A button / Enter).
    Select,
    /// Go up one level / dismiss (B button / Escape).
    Back,
}

impl NavKey {
    fn as_str(&self) -> &'static str {
        match self {
            NavKey::Up => "up",
            NavKey::Down => "down",
            NavKey::Left => "left",
            NavKey::Right => "right",
            NavKey::Select => "select",
            NavKey::Back => "back",
        }
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct NavigateParams {
    /// Direction or action key. `select` = activate/confirm the focused element
    /// (A button / Enter); `back` = go up one level / dismiss (B button / Escape).
    pub key: NavKey,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct OpenSettingsParams {
    /// Settings page slug. Known slugs: audio, bluetooth, network, display,
    /// controllers, keybindings, avcontrol, accessibility, power, system.
    /// Unknown slugs are a graceful no-op in QML.
    pub page: String,
}

/// Overlay popover target.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum OverlayTarget {
    /// Audio volume slider popover.
    Volume,
    /// Wi-Fi / network connection popover.
    Network,
    /// Power / session drawer (sleep, restart, end session).
    Session,
}

impl OverlayTarget {
    fn as_str(&self) -> &'static str {
        match self {
            OverlayTarget::Volume => "volume",
            OverlayTarget::Network => "network",
            OverlayTarget::Session => "session",
        }
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct OpenOverlayParams {
    /// Overlay popover to open. `volume` = audio volume slider; `network` = Wi-Fi /
    /// connection popover; `session` = power / session drawer (sleep, restart,
    /// end session).
    pub target: OverlayTarget,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct LaunchAppParams {
    /// The StartupWMClass of the .desktop application to launch.
    pub wm_class: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct TakeScreenshotParams {
    /// When true, broadcast a flash event on the IPC bus after capture so the
    /// QML shell can paint a brief white vignette as visual feedback.
    #[serde(default)]
    pub flash: bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct GetLogsParams {
    /// Number of log lines to return (default 100, max 1000).
    #[serde(default = "default_log_lines")]
    pub lines: u32,
    /// Optional substring filter (case-insensitive).
    pub filter: Option<String>,
}

fn default_log_lines() -> u32 {
    100
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct DevDeployParams {
    /// Git ref to deploy (branch name, tag, or commit SHA). Defaults to "main".
    /// Only alphanumeric characters, `.`, `_`, `/`, and `-` are accepted.
    pub git_ref: Option<String>,
}

// ─── Server handle ───────────────────────────────────────────────────────────

/// The shared runtime handles cloned into each MCP session.
#[derive(Clone)]
struct Handles {
    control_tx: mpsc::Sender<Control>,
    events_tx: broadcast::Sender<Event>,
    shutdown: CancellationToken,
    reexec_flag: Arc<std::sync::atomic::AtomicBool>,
    dev_enabled: bool,
}

// ─── MCP handler ─────────────────────────────────────────────────────────────

#[derive(Clone)]
pub struct GameShellMcp {
    handles: Handles,
    tool_router: ToolRouter<Self>,
}

impl GameShellMcp {
    fn new(handles: Handles) -> Self {
        Self {
            tool_router: Self::tool_router(),
            handles,
        }
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for GameShellMcp {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new(
                "game-shell-mcp",
                env!("CARGO_PKG_VERSION"),
            ))
            .with_instructions(
                "Controls the game-shell Quickshell UI on the game client. \
                 Use send_intent / navigate for UI navigation; prefer the typed sugar \
                 tools (open_settings, open_overlay, launch_app) for specific targets. \
                 Use take_screenshot to capture the current display — call it after \
                 every action to confirm the UI reached the expected state. \
                 Use get_status / get_logs for diagnostics and restart_shell to recover \
                 from a crashed UI.",
            )
    }
}

#[tool_router(router = tool_router)]
impl GameShellMcp {
    // ── Navigation / intents ──────────────────────────────────────────────────

    #[tool(
        description = "Send a named intent to the game-shell UI. \
            Bare names: home, home-tap, home-hold, menu, settings, power. \
            Deep-link families (colon-delimited): settings:<page-slug>, \
            overlay:<target (volume|network|session)>, app:<StartupWMClass>. \
            To open a specific settings page, overlay, or app, prefer \
            open_settings / open_overlay / launch_app.",
        annotations(read_only_hint = false, destructive_hint = false)
    )]
    async fn send_intent(
        &self,
        Parameters(SendIntentParams { name }): Parameters<SendIntentParams>,
    ) -> CallToolResult {
        if !bridge_core::is_valid_intent(&name) {
            return CallToolResult::error(vec![Content::text(format!(
                "unknown intent '{name}'. Valid bare intents: home, home-tap, home-hold, \
                 menu, settings, power. Deep-links: settings:<page>, \
                 overlay:<volume|network|session>, app:<wmClass>."
            ))]);
        }
        match bridge_core::dispatch_intent(&self.handles.control_tx, name).await {
            None => CallToolResult::error(vec![Content::text("daemon unavailable")]),
            Some(r) if r.starts_with("error:") => {
                let msg = r.trim_start_matches("error:").trim().to_owned();
                CallToolResult::error(vec![Content::text(msg)])
            }
            Some(_) => CallToolResult::success(vec![Content::text("ok")]),
        }
    }

    #[tool(
        description = "Synthesize a directional or action keypress on the \
            game-shell virtual keyboard. `select` activates/confirms the focused \
            element (A button / Enter); `back` goes up one level / dismisses \
            (B button / Escape).",
        annotations(read_only_hint = false, destructive_hint = false)
    )]
    async fn navigate(
        &self,
        Parameters(NavigateParams { key }): Parameters<NavigateParams>,
    ) -> CallToolResult {
        let key_str = key.as_str().to_owned();
        match bridge_core::dispatch_key(&self.handles.control_tx, key_str).await {
            None => CallToolResult::error(vec![Content::text("daemon unavailable")]),
            Some(r) if r.starts_with("error:") => {
                let msg = r.trim_start_matches("error:").trim().to_owned();
                CallToolResult::error(vec![Content::text(msg)])
            }
            Some(_) => CallToolResult::success(vec![Content::text("ok")]),
        }
    }

    #[tool(
        description = "Open a specific settings page directly. \
            Known page slugs: audio, bluetooth, network, display, controllers, \
            keybindings, avcontrol, accessibility, power, system. \
            Unknown slugs are a graceful no-op in QML.",
        annotations(read_only_hint = false, destructive_hint = false)
    )]
    async fn open_settings(
        &self,
        Parameters(OpenSettingsParams { page }): Parameters<OpenSettingsParams>,
    ) -> CallToolResult {
        let intent_name = bridge_core::settings_intent(&page);
        match bridge_core::dispatch_intent(&self.handles.control_tx, intent_name).await {
            None => CallToolResult::error(vec![Content::text("daemon unavailable")]),
            Some(r) if r.starts_with("error:") => {
                let msg = r.trim_start_matches("error:").trim().to_owned();
                CallToolResult::error(vec![Content::text(msg)])
            }
            Some(_) => CallToolResult::success(vec![Content::text("ok")]),
        }
    }

    #[tool(
        description = "Open an overlay popover. `volume` = audio volume slider; \
            `network` = Wi-Fi / connection popover; `session` = power / session \
            drawer (sleep, restart, end session).",
        annotations(read_only_hint = false, destructive_hint = false)
    )]
    async fn open_overlay(
        &self,
        Parameters(OpenOverlayParams { target }): Parameters<OpenOverlayParams>,
    ) -> CallToolResult {
        let intent_name = bridge_core::overlay_intent(target.as_str())
            .expect("OverlayTarget enum guarantees a valid target");
        match bridge_core::dispatch_intent(&self.handles.control_tx, intent_name).await {
            None => CallToolResult::error(vec![Content::text("daemon unavailable")]),
            Some(r) if r.starts_with("error:") => {
                let msg = r.trim_start_matches("error:").trim().to_owned();
                CallToolResult::error(vec![Content::text(msg)])
            }
            Some(_) => CallToolResult::success(vec![Content::text("ok")]),
        }
    }

    #[tool(
        description = "Launch a local .desktop application by its StartupWMClass.",
        annotations(read_only_hint = false, destructive_hint = false)
    )]
    async fn launch_app(
        &self,
        Parameters(LaunchAppParams { wm_class }): Parameters<LaunchAppParams>,
    ) -> CallToolResult {
        let intent_name = bridge_core::app_intent(&wm_class);
        match bridge_core::dispatch_intent(&self.handles.control_tx, intent_name).await {
            None => CallToolResult::error(vec![Content::text("daemon unavailable")]),
            Some(r) if r.starts_with("error:") => {
                let msg = r.trim_start_matches("error:").trim().to_owned();
                CallToolResult::error(vec![Content::text(msg)])
            }
            Some(_) => CallToolResult::success(vec![Content::text("ok")]),
        }
    }

    // ── Screenshot ────────────────────────────────────────────────────────────

    #[tool(
        description = "Capture the current Wayland display as a PNG image. \
            Set flash=true to trigger a brief white vignette on the game-shell UI \
            after capture (visual feedback for the user at the TV). \
            Call this after every UI action to confirm the expected state was reached.",
        annotations(read_only_hint = true)
    )]
    async fn take_screenshot(
        &self,
        Parameters(TakeScreenshotParams { flash }): Parameters<TakeScreenshotParams>,
    ) -> CallToolResult {
        match bridge_core::capture_screenshot(&self.handles.events_tx, flash).await {
            Ok(png) => {
                let b64 = base64::engine::general_purpose::STANDARD.encode(&png);
                CallToolResult::success(vec![Content::image(b64, "image/png")])
            }
            Err(msg) => CallToolResult::error(vec![Content::text(msg)]),
        }
    }

    // ── Status / diagnostics ──────────────────────────────────────────────────

    #[tool(
        description = "Return structured status: git SHA, daemon PID, daemon version, \
            whether quickshell is running, Wayland display name, and whether \
            HYPRLAND_INSTANCE_SIGNATURE is resolvable. Returns a typed JSON object \
            with an advertised output schema. \
            After an action, call take_screenshot to confirm the UI reached the \
            expected state. The typed sugar tools (open_settings, open_overlay, \
            launch_app) are preferred for specific navigation targets.",
        annotations(read_only_hint = true)
    )]
    async fn get_status(&self) -> Result<Json<bridge_core::StatusInfo>, String> {
        Ok(Json(bridge_core::get_status().await))
    }

    #[tool(
        description = "Return the last N lines of /tmp/qs-log.txt \
            (the quickshell log file). Optionally filter by a substring. \
            If the log file does not exist yet, returns an explanatory hint.",
        annotations(read_only_hint = true)
    )]
    async fn get_logs(
        &self,
        Parameters(GetLogsParams { lines, filter }): Parameters<GetLogsParams>,
    ) -> CallToolResult {
        let lines_usize = (lines as usize).min(1000);
        match bridge_core::get_logs(lines_usize, filter.as_deref()) {
            Ok(content) => CallToolResult::success(vec![Content::text(content)]),
            Err(msg) => CallToolResult::error(vec![Content::text(msg)]),
        }
    }

    // ── Shell management ──────────────────────────────────────────────────────

    #[tool(
        description = "Kill and restart the quickshell process. \
            Waits 3 seconds for startup and returns the first WARN/ERROR log lines \
            (or a 'no errors' confirmation). Use after deploying a new game-shell \
            build to pick up QML changes without rebooting.",
        annotations(read_only_hint = false, destructive_hint = true)
    )]
    async fn restart_shell(&self) -> CallToolResult {
        match bridge_core::dev_restart_shell().await {
            Ok(body) => CallToolResult::success(vec![Content::text(body)]),
            Err(msg) => CallToolResult::error(vec![Content::text(msg)]),
        }
    }

    // ── Dev operations (gated on GAME_SHELL_MCP_DEV) ─────────────────────────
    // These are only *registered* when dev_enabled is true; the tool_router
    // includes them unconditionally at compile time but they return a clear
    // error when the dev flag is absent — this is the safest approach with the
    // current rmcp macro model (conditional registration is not yet supported).

    #[tool(
        description = "DEV ONLY (requires GAME_SHELL_MCP_DEV env var). \
            git fetch + checkout + reset to remote. Defaults to 'main'. \
            Use to pull a branch onto the device without a full re-deploy.",
        annotations(read_only_hint = false, destructive_hint = true)
    )]
    async fn dev_deploy(
        &self,
        Parameters(DevDeployParams { git_ref }): Parameters<DevDeployParams>,
    ) -> CallToolResult {
        if !self.handles.dev_enabled {
            return CallToolResult::error(vec![Content::text(
                "dev tools disabled — set GAME_SHELL_MCP_DEV to enable",
            )]);
        }
        match bridge_core::dev_deploy(git_ref.as_deref()).await {
            Ok(body) => CallToolResult::success(vec![Content::text(body)]),
            Err(msg) => CallToolResult::error(vec![Content::text(msg)]),
        }
    }

    #[tool(
        description = "DEV ONLY (requires GAME_SHELL_MCP_DEV env var). \
            Run scripts/build-daemon.sh and install the resulting binary. \
            This is a long-running operation (~15-60 seconds depending on cache).",
        annotations(read_only_hint = false, destructive_hint = true)
    )]
    async fn dev_build(&self) -> CallToolResult {
        if !self.handles.dev_enabled {
            return CallToolResult::error(vec![Content::text(
                "dev tools disabled — set GAME_SHELL_MCP_DEV to enable",
            )]);
        }
        match bridge_core::dev_build().await {
            Ok(body) => CallToolResult::success(vec![Content::text(body)]),
            Err(msg) => CallToolResult::error(vec![Content::text(msg)]),
        }
    }

    #[tool(
        description = "DEV ONLY (requires GAME_SHELL_MCP_DEV env var). \
            Re-exec the daemon process (picks up a newly built binary). \
            The MCP connection will drop immediately after the response.",
        annotations(read_only_hint = false, destructive_hint = true)
    )]
    async fn dev_restart_daemon(&self) -> CallToolResult {
        if !self.handles.dev_enabled {
            return CallToolResult::error(vec![Content::text(
                "dev tools disabled — set GAME_SHELL_MCP_DEV to enable",
            )]);
        }
        bridge_core::request_reexec(&self.handles.reexec_flag, &self.handles.shutdown);
        CallToolResult::success(vec![Content::text("ok, re-execing\n")])
    }
}

// ─── Auth state (shared by the middleware closure) ────────────────────────────

#[derive(Clone)]
struct AuthState {
    /// `Some(token)` when auth is enabled and a token is configured.
    /// `None` when auth is disabled.
    expected_bearer: Option<String>,
    /// True when auth is enabled but no token was provided — fail closed.
    fail_closed: bool,
}

/// axum middleware that enforces bearer-token auth on all `/mcp` requests.
///
/// Mirrors the HTTP bridge's auth logic (`http.rs`) at parity:
/// - Auth enabled + token set: constant-time comparison of `Authorization: Bearer <token>`.
/// - Auth enabled + no token:  reject all (fail closed, 401).
/// - Auth disabled:            pass through unconditionally.
async fn auth_middleware(
    axum::extract::State(state): axum::extract::State<AuthState>,
    headers: HeaderMap,
    request: Request,
    next: Next,
) -> Result<Response, StatusCode> {
    if state.fail_closed {
        // Auth enabled but no token configured — reject all.
        return Err(StatusCode::UNAUTHORIZED);
    }
    if let Some(expected) = &state.expected_bearer {
        let provided = headers
            .get(axum::http::header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        if !bridge_core::ct_eq_str(provided, expected) {
            return Err(StatusCode::UNAUTHORIZED);
        }
    }
    // Auth disabled or token matched.
    Ok(next.run(request).await)
}

// ─── Public serve entry point ─────────────────────────────────────────────────

/// Bind an axum listener to `addr` and serve the MCP Streamable HTTP server
/// on `/mcp` until the shared `shutdown` token is cancelled.
///
/// Called from `main.rs` when `GAME_SHELL_MCP_BIND` is set. The shared
/// CancellationToken is cancelled on SIGTERM/SIGINT or when a re-exec is
/// requested — both paths call `bridge_core::request_reexec` which cancels
/// the token. The MCP server then shuts down cleanly and the main loop
/// proceeds to re-exec (if flagged).
///
/// `token` and `auth_enabled` mirror the HTTP bridge's env vars
/// (`GAME_SHELL_HTTP_TOKEN`, `GAME_SHELL_HTTP_AUTH_ENABLED`) so operators
/// only need one token for both bridges.
pub async fn serve(
    addr: std::net::SocketAddr,
    token: Option<String>,
    auth_enabled: bool,
    control_tx: mpsc::Sender<Control>,
    events_tx: broadcast::Sender<Event>,
    shutdown: CancellationToken,
    reexec_flag: Arc<std::sync::atomic::AtomicBool>,
) {
    use rmcp::transport::streamable_http_server::{
        session::local::LocalSessionManager, StreamableHttpServerConfig, StreamableHttpService,
    };

    let dev_enabled = std::env::var("GAME_SHELL_MCP_DEV")
        .map(|v| !v.is_empty())
        .unwrap_or(false);

    if dev_enabled {
        tracing::warn!(
            "mcp: GAME_SHELL_MCP_DEV is set — dev tools enabled (deploy/build/restart-daemon)"
        );
    }

    // Fix 5: Refuse to start the MCP server when the dangerous combo is present:
    // non-loopback bind + dev mode + auth effectively disabled (no token / auth off).
    if dev_enabled && !addr.ip().is_loopback() {
        let auth_effectively_disabled = !auth_enabled || token.is_none();
        if auth_effectively_disabled {
            tracing::error!(
                "mcp: REFUSING to start MCP server on non-loopback address {addr} \
                 with GAME_SHELL_MCP_DEV enabled and no authentication. \
                 This would expose an unauthenticated RCE surface on the LAN. \
                 Set GAME_SHELL_HTTP_TOKEN + ensure GAME_SHELL_HTTP_AUTH_ENABLED != 0, \
                 or bind to 127.0.0.1 for local-only dev access."
            );
            return;
        }
    }

    // Build auth state for the middleware.
    let auth_state = if !auth_enabled {
        tracing::warn!(
            "mcp: AUTH DISABLED (GAME_SHELL_HTTP_AUTH_ENABLED=0) — \
             any host on the network can send MCP commands without authentication"
        );
        AuthState {
            expected_bearer: None,
            fail_closed: false,
        }
    } else {
        match &token {
            None => {
                tracing::warn!(
                    "mcp: auth is ENABLED but GAME_SHELL_HTTP_TOKEN is not set — \
                     all MCP requests will be rejected with 401 (set the token or \
                     disable auth with GAME_SHELL_HTTP_AUTH_ENABLED=0)"
                );
                AuthState {
                    expected_bearer: None,
                    fail_closed: true,
                }
            }
            Some(t) => {
                if addr.ip().is_unspecified() {
                    tracing::warn!(
                        "mcp: binding to {} with bearer auth — \
                         any host on the network can attempt authentication",
                        addr
                    );
                }
                AuthState {
                    expected_bearer: Some(format!("Bearer {t}")),
                    fail_closed: false,
                }
            }
        }
    };

    let handles = Handles {
        control_tx,
        events_tx,
        shutdown: shutdown.clone(),
        reexec_flag,
        dev_enabled,
    };

    // StreamableHttpServerConfig is #[non_exhaustive] — cannot use struct
    // literal syntax with ..Default::default(). Mutate the default instead.
    let mut config = StreamableHttpServerConfig::default();
    config.cancellation_token = shutdown.child_token();

    // Fix 3: Configure the host allowlist correctly.
    //
    // rmcp 1.7.0 does NOT support a "*" wildcard — it does literal string
    // matching. An empty allowed_hosts list means "allow all hosts" (see
    // rmcp source: host_is_allowed returns true when the list is empty).
    //
    // We build the allowlist from:
    //   1. Always: localhost, 127.0.0.1, ::1 (loopback)
    //   2. GAME_SHELL_MCP_ALLOWED_HOSTS env var (comma-separated host[:port])
    //   3. If the bind address is a concrete IP (not 0.0.0.0/::), include it.
    //   4. If the bind is a wildcard AND no env override, clear the list
    //      (allow-all) and warn — acceptable because Fix 2's bearer token is
    //      the real gate.
    let allowed_hosts_env = std::env::var("GAME_SHELL_MCP_ALLOWED_HOSTS").ok();
    let bind_is_wildcard = addr.ip().is_unspecified();

    if bind_is_wildcard && allowed_hosts_env.is_none() {
        // Allow all — host header matching disabled. Token is the gate.
        tracing::warn!(
            "mcp: host allowlisting is DISABLED (bind is wildcard, \
             GAME_SHELL_MCP_ALLOWED_HOSTS not set). \
             DNS-rebinding protection relies on the bearer token. \
             Set GAME_SHELL_MCP_ALLOWED_HOSTS=<host> or bind to a concrete IP \
             to restrict by Host header."
        );
        config.allowed_hosts = vec![];
    } else {
        let mut hosts: Vec<String> = vec!["localhost".into(), "127.0.0.1".into(), "::1".into()];
        if !bind_is_wildcard {
            // Include the concrete bind address so LAN clients connecting to
            // game-client-1's real IP get through.
            hosts.push(addr.ip().to_string());
            hosts.push(addr.to_string()); // also include host:port form
        }
        if let Some(env_hosts) = &allowed_hosts_env {
            for h in env_hosts.split(',') {
                let h = h.trim();
                if !h.is_empty() {
                    hosts.push(h.to_owned());
                }
            }
        }
        config.allowed_hosts = hosts;
    }

    let handles_clone = handles.clone();
    let service: StreamableHttpService<GameShellMcp, LocalSessionManager> =
        StreamableHttpService::new(
            move || Ok(GameShellMcp::new(handles_clone.clone())),
            Default::default(),
            config,
        );

    // Wrap the MCP service with bearer-token auth middleware (Fix 2).
    let router = axum::Router::new()
        .nest_service("/mcp", service)
        .layer(middleware::from_fn_with_state(auth_state, auth_middleware));

    let listener = match tokio::net::TcpListener::bind(addr).await {
        Ok(l) => l,
        Err(e) => {
            tracing::error!("mcp: failed to bind {addr}: {e}");
            return;
        }
    };
    tracing::info!("MCP server listening on http://{addr}/mcp");

    let _ = axum::serve(listener, router)
        .with_graceful_shutdown(async move { shutdown.cancelled().await })
        .await;

    tracing::info!("mcp: server stopped");
}

// ─── Unit tests ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── NavKey enum serialisation ─────────────────────────────────────────────

    #[test]
    fn nav_key_as_str_matches_expected() {
        assert_eq!(NavKey::Up.as_str(), "up");
        assert_eq!(NavKey::Down.as_str(), "down");
        assert_eq!(NavKey::Left.as_str(), "left");
        assert_eq!(NavKey::Right.as_str(), "right");
        assert_eq!(NavKey::Select.as_str(), "select");
        assert_eq!(NavKey::Back.as_str(), "back");
    }

    #[test]
    fn nav_key_deserialises_lowercase() {
        // serde rename_all = "lowercase" → the JSON value must be the lowercase variant
        let up: NavKey = serde_json::from_str("\"up\"").expect("deserialise up");
        assert_eq!(up.as_str(), "up");
        let select: NavKey = serde_json::from_str("\"select\"").expect("deserialise select");
        assert_eq!(select.as_str(), "select");
        let back: NavKey = serde_json::from_str("\"back\"").expect("deserialise back");
        assert_eq!(back.as_str(), "back");
    }

    #[test]
    fn nav_key_deserialise_all_variants() {
        for (json, expected) in &[
            ("\"up\"", "up"),
            ("\"down\"", "down"),
            ("\"left\"", "left"),
            ("\"right\"", "right"),
            ("\"select\"", "select"),
            ("\"back\"", "back"),
        ] {
            let k: NavKey = serde_json::from_str(json)
                .unwrap_or_else(|e| panic!("failed to deserialise {json}: {e}"));
            assert_eq!(k.as_str(), *expected);
        }
    }

    // ── OverlayTarget enum serialisation ──────────────────────────────────────

    #[test]
    fn overlay_target_as_str_matches_expected() {
        assert_eq!(OverlayTarget::Volume.as_str(), "volume");
        assert_eq!(OverlayTarget::Network.as_str(), "network");
        assert_eq!(OverlayTarget::Session.as_str(), "session");
    }

    #[test]
    fn overlay_target_deserialises_lowercase() {
        let v: OverlayTarget = serde_json::from_str("\"volume\"").expect("deserialise volume");
        assert_eq!(v.as_str(), "volume");
        let n: OverlayTarget = serde_json::from_str("\"network\"").expect("deserialise network");
        assert_eq!(n.as_str(), "network");
        let s: OverlayTarget = serde_json::from_str("\"session\"").expect("deserialise session");
        assert_eq!(s.as_str(), "session");
    }

    #[test]
    fn overlay_target_maps_to_valid_intent() {
        // Every OverlayTarget variant must produce a valid overlay intent.
        for target in &[
            OverlayTarget::Volume,
            OverlayTarget::Network,
            OverlayTarget::Session,
        ] {
            let result = bridge_core::overlay_intent(target.as_str());
            assert!(
                result.is_ok(),
                "expected valid intent for {:?}, got {:?}",
                target.as_str(),
                result
            );
        }
    }
}
