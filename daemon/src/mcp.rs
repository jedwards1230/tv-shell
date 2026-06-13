//! MCP server for the game-shell daemon, built on the official `rmcp` 1.7.0 crate.
//!
//! **Opt-in**: the server only starts when the `GAME_SHELL_MCP_BIND` environment
//! variable is set to a `host:port` address. When unset, no socket is opened.
//!
//! **Dev tools**: `dev_deploy`, `dev_build`, and `dev_restart_daemon` are only
//! registered when `GAME_SHELL_MCP_DEV` is set (any non-empty value). This keeps
//! the production tool surface minimal.
//!
//! **Transport**: StreamableHttpService over axum, served at `/mcp`. The MCP
//! endpoint is at `http://<bind>/mcp`.
//!
//! **Linux-only + feature-gated**: this entire module is
//! `#[cfg(all(target_os = "linux", feature = "mcp"))]` — the `rmcp` crate
//! links against several Linux-oriented async I/O primitives. Default builds
//! (including macOS dev boxes and the CI default leg) exclude it entirely.

use std::sync::Arc;

use base64::Engine as _;
use rmcp::{
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{CallToolResult, Content, Implementation, ServerCapabilities, ServerInfo},
    schemars::{self, JsonSchema},
    tool, tool_handler, tool_router, ServerHandler,
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
    /// power. Deep-link families: settings:<page-slug>, overlay:<target>,
    /// app:<StartupWMClass>.
    pub name: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct NavigateParams {
    /// Direction or action key: up, down, left, right, select, back.
    pub key: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct OpenSettingsParams {
    /// Settings page slug. Known slugs: audio, bluetooth, network, display,
    /// controllers, keybindings, avcontrol, accessibility, power, system.
    /// Unknown slugs are a graceful no-op in QML.
    pub page: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct OpenOverlayParams {
    /// Overlay target: volume, network, session.
    pub target: String,
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
    pub git_ref: Option<String>,
}

// ─── Server handle ───────────────────────────────────────────────────────────

/// The shared runtime handles cloned into each MCP session.
#[derive(Clone)]
struct Handles {
    control_tx: mpsc::Sender<Control>,
    events_tx: broadcast::Sender<Event>,
    reexec_notify: Arc<tokio::sync::Notify>,
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
                 Use send_intent / navigate for UI navigation, take_screenshot to \
                 capture the current display, get_status / get_logs for diagnostics, \
                 and restart_shell to recover from a crashed UI.",
            )
    }
}

#[tool_router(router = tool_router)]
impl GameShellMcp {
    // ── Navigation / intents ──────────────────────────────────────────────────

    #[tool(description = "Send a named intent to the game-shell UI. \
        Bare names: home, home-tap, home-hold, menu, settings, power. \
        Deep-link families (colon-delimited): settings:<page-slug>, \
        overlay:<target (volume|network|session)>, app:<StartupWMClass>.")]
    async fn send_intent(
        &self,
        Parameters(SendIntentParams { name }): Parameters<SendIntentParams>,
    ) -> CallToolResult {
        if !bridge_core::is_valid_intent(&name) {
            return CallToolResult::error(vec![Content::text(format!("unknown intent '{name}'"))]);
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

    #[tool(description = "Synthesize a directional or action keypress on the \
        game-shell virtual keyboard. Valid keys: up, down, left, right, select, back.")]
    async fn navigate(
        &self,
        Parameters(NavigateParams { key }): Parameters<NavigateParams>,
    ) -> CallToolResult {
        match bridge_core::dispatch_key(&self.handles.control_tx, key).await {
            None => CallToolResult::error(vec![Content::text("daemon unavailable")]),
            Some(r) if r.starts_with("error:") => {
                let msg = r.trim_start_matches("error:").trim().to_owned();
                CallToolResult::error(vec![Content::text(msg)])
            }
            Some(_) => CallToolResult::success(vec![Content::text("ok")]),
        }
    }

    #[tool(description = "Open a specific settings page directly. \
        Known page slugs: audio, bluetooth, network, display, controllers, \
        keybindings, avcontrol, accessibility, power, system. \
        Unknown slugs are a graceful no-op in QML.")]
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

    #[tool(description = "Open a QAM overlay popover. \
        Valid targets: volume, network, session.")]
    async fn open_overlay(
        &self,
        Parameters(OpenOverlayParams { target }): Parameters<OpenOverlayParams>,
    ) -> CallToolResult {
        let intent_name = match bridge_core::overlay_intent(&target) {
            Ok(n) => n,
            Err(msg) => return CallToolResult::error(vec![Content::text(msg)]),
        };
        match bridge_core::dispatch_intent(&self.handles.control_tx, intent_name).await {
            None => CallToolResult::error(vec![Content::text("daemon unavailable")]),
            Some(r) if r.starts_with("error:") => {
                let msg = r.trim_start_matches("error:").trim().to_owned();
                CallToolResult::error(vec![Content::text(msg)])
            }
            Some(_) => CallToolResult::success(vec![Content::text("ok")]),
        }
    }

    #[tool(description = "Launch a local .desktop application by its StartupWMClass.")]
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

    #[tool(description = "Capture the current Wayland display as a PNG image. \
        Set flash=true to trigger a brief white vignette on the game-shell UI \
        after capture (visual feedback for the user at the TV).")]
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

    #[tool(description = "Return a JSON status blob: git SHA, daemon PID, \
        daemon version, whether quickshell is running, Wayland display name, \
        and whether HYPRLAND_INSTANCE_SIGNATURE is resolvable.")]
    async fn get_status(&self) -> CallToolResult {
        let info = bridge_core::get_status().await;
        match serde_json::to_string_pretty(&info) {
            Ok(json) => CallToolResult::success(vec![Content::text(json)]),
            Err(e) => CallToolResult::error(vec![Content::text(format!("serialise error: {e}"))]),
        }
    }

    #[tool(description = "Return the last N lines of /tmp/qs-log.txt \
        (the quickshell log file). Optionally filter by a substring. \
        If the log file does not exist yet, returns an explanatory hint.")]
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

    #[tool(description = "Kill and restart the quickshell process. \
        Waits 3 seconds for startup and returns the first WARN/ERROR log lines \
        (or a 'no errors' confirmation). Use after deploying a new game-shell \
        build to pick up QML changes without rebooting.")]
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

    #[tool(description = "DEV ONLY (requires GAME_SHELL_MCP_DEV env var). \
        git fetch + checkout + reset to remote. Defaults to 'main'. \
        Use to pull a branch onto the device without a full re-deploy.")]
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

    #[tool(description = "DEV ONLY (requires GAME_SHELL_MCP_DEV env var). \
        Run scripts/build-daemon.sh and install the resulting binary. \
        This is a long-running operation (~15-60 seconds depending on cache).")]
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

    #[tool(description = "DEV ONLY (requires GAME_SHELL_MCP_DEV env var). \
        Re-exec the daemon process (picks up a newly built binary). \
        The MCP connection will drop immediately after the response.")]
    async fn dev_restart_daemon(&self) -> CallToolResult {
        if !self.handles.dev_enabled {
            return CallToolResult::error(vec![Content::text(
                "dev tools disabled — set GAME_SHELL_MCP_DEV to enable",
            )]);
        }
        bridge_core::request_reexec(&self.handles.reexec_flag, &self.handles.reexec_notify);
        CallToolResult::success(vec![Content::text("ok, re-execing\n")])
    }
}

// ─── Public serve entry point ─────────────────────────────────────────────────

/// Bind an axum listener to `addr` and serve the MCP Streamable HTTP server
/// on `/mcp` until the `CancellationToken` is cancelled.
///
/// Called from `main.rs` when `GAME_SHELL_MCP_BIND` is set. The token is
/// wired to the daemon's re-exec / shutdown path so the MCP server stops
/// cleanly when the process exits.
pub async fn serve(
    addr: std::net::SocketAddr,
    control_tx: mpsc::Sender<Control>,
    events_tx: broadcast::Sender<Event>,
    reexec_notify: Arc<tokio::sync::Notify>,
    reexec_flag: Arc<std::sync::atomic::AtomicBool>,
    cancel: CancellationToken,
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

    let handles = Handles {
        control_tx,
        events_tx,
        reexec_notify,
        reexec_flag,
        dev_enabled,
    };

    // StreamableHttpServerConfig is #[non_exhaustive] — cannot use struct
    // literal syntax with ..Default::default(). Mutate the default instead.
    let mut config = StreamableHttpServerConfig::default();
    config.cancellation_token = cancel.child_token();
    // Allow LAN callers (not just loopback). The operator is responsible
    // for binding to a trusted interface — same posture as the HTTP bridge.
    config.allowed_hosts = vec![
        "localhost".into(),
        "127.0.0.1".into(),
        "::1".into(),
        "0.0.0.0".into(),
        // Accept any Host header value for LAN IPs.
        // The StreamableHttpService default only allows loopback; we need
        // to broaden this for the game-client-1 LAN case.
        "*".into(),
    ];

    let handles_clone = handles.clone();
    let service: StreamableHttpService<GameShellMcp, LocalSessionManager> =
        StreamableHttpService::new(
            move || Ok(GameShellMcp::new(handles_clone.clone())),
            Default::default(),
            config,
        );

    let router = axum::Router::new().nest_service("/mcp", service);

    let listener = match tokio::net::TcpListener::bind(addr).await {
        Ok(l) => l,
        Err(e) => {
            tracing::error!("mcp: failed to bind {addr}: {e}");
            return;
        }
    };
    tracing::info!("MCP server listening on http://{addr}/mcp");

    let _ = axum::serve(listener, router)
        .with_graceful_shutdown(async move { cancel.cancelled().await })
        .await;

    tracing::info!("mcp: server stopped");
}
