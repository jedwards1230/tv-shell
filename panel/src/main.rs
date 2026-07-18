//! tv-shell-panel — a LAN-only, server-rendered (axum + askama + vendored
//! HTMX) web control panel for the tv-shell HTPC daemon.
//!
//! M1 scope: the crate scaffold, three data-tier clients (`ipc` — the
//! primary Unix-socket IPC tier, `bridge` — the daemon's opt-in HTTP dev-ops
//! bridge, `exec` — a direct-exec recovery tier for when both of the above
//! are down), the app shell with nav for all nine pages, and three fully
//! implemented pages (Dashboard, Logs, Dev). The other six pages
//! (Processes, Settings, Widgets, Tools, Controllers, CEC) render an honest
//! stub until their milestone lands.
//!
//! Auth: none in v1 (LAN-only deployment; `[panel].token_file` is parsed but
//! unused for the panel's own auth surface — reserved for a later milestone).

mod assets;
mod bridge;
mod config;
mod exec;
mod ipc;
mod pages;
mod state;

#[cfg(test)]
mod tests;

use std::sync::Arc;

use axum::routing::{get, post};
use axum::Router;

use state::{AppState, SharedState};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    let cfg = config::load();
    if !cfg.enabled {
        tracing::info!("tv-shell-panel: disabled ([panel].enabled = false) — exiting cleanly");
        return Ok(());
    }

    let panel_bind = cfg.panel_bind;
    tracing::debug!(
        "tv-shell-panel: config resolved — [panel].bind={:?}, [panel].token_file={:?} \
         (parsed but unused for the panel's own auth in v1 — LAN-only, no auth yet)",
        cfg.panel_bind_raw,
        cfg.panel_token_file
    );
    let sock = config::socket_path();
    let ipc = ipc::IpcClient::new(sock);
    let bridge = bridge::BridgeClient::new(cfg.http_bridge_base.clone(), cfg.http_token.clone());
    let recovery = exec::Recovery::new();

    let state: SharedState = Arc::new(AppState {
        cfg,
        ipc,
        bridge,
        recovery,
    });

    // Route registration is one line per page so later milestones can add
    // routes without touching neighboring lines.
    let app = Router::new()
        .route("/", get(pages::dashboard::page))
        .route("/dashboard", get(pages::dashboard::page))
        .route("/dashboard/tiles", get(pages::dashboard::tiles)) // htmx poll partial
        .route("/processes", get(pages::processes::page))
        .route("/settings", get(pages::settings::page))
        .route("/settings/save", post(pages::settings::save))
        .route("/settings/raw", post(pages::settings::save_raw))
        .route("/widgets", get(pages::widgets::page))
        .route("/widgets/save", post(pages::widgets::save))
        .route("/tools", get(pages::tools::page))
        .route("/controllers", get(pages::controllers::page))
        .route("/cec", get(pages::cec::page))
        .route("/logs", get(pages::logs::page))
        .route("/logs/view", get(pages::logs::view)) // htmx refresh partial
        .route("/dev", get(pages::dev::page))
        .route("/dev/deploy", post(pages::dev::deploy))
        .route("/dev/build", post(pages::dev::build))
        .route("/dev/restart-daemon", post(pages::dev::restart_daemon))
        .route("/dev/restart-shell", post(pages::dev::restart_shell))
        .route("/dev/reboot", post(pages::dev::reboot))
        .route("/dev/suspend", post(pages::dev::suspend))
        .route("/assets/htmx.min.js", get(assets::htmx_js))
        .route("/assets/style.css", get(assets::style_css))
        .with_state(state);

    tracing::info!("tv-shell-panel listening on {panel_bind}");
    let listener = tokio::net::TcpListener::bind(panel_bind).await?;
    axum::serve(listener, app).await?;
    Ok(())
}
