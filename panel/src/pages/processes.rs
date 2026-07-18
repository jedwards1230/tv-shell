//! `/processes` — three read surfaces: the three tv-shell systemd user units
//! (daemon/shell/panel) with a per-unit restart, Hyprland windows via IPC
//! (`hypr-active`/`hypr-clients`/`hypr-monitors`), and a read-only top-
//! processes snapshot via `ps`.
//!
//! Degradation: the unit states and process list are exec-based (always
//! available regardless of the daemon); the Hyprland section is IPC-based and
//! shows its own "unavailable" note (daemon down, or the Hyprland actor
//! itself down) without failing the rest of the page — `GET /processes` is
//! always 200, never a 500.

use askama::Template;
use axum::extract::{Path, State};
use axum::response::{Html, IntoResponse};

use crate::config;
use crate::ipc::IpcError;
use crate::state::{AppState, SharedState};

struct UnitView {
    key: &'static str,
    label: &'static str,
    unit: String,
    state: String,
}

#[derive(Template)]
#[template(path = "processes.html")]
struct ProcessesTemplate {
    active: &'static str,
    units: Vec<UnitView>,
    hypr_available: bool,
    hypr_active: String,
    hypr_clients: String,
    hypr_monitors: String,
    top_processes: String,
}

/// `GET /processes` — gathers all three sections synchronously (mirrors
/// `pages::dashboard::render_tiles`'s degrade-per-section approach, just
/// folded into the one page render rather than a separate polled partial).
pub async fn page(State(state): State<SharedState>) -> impl IntoResponse {
    Html(render_page(&state).await)
}

pub async fn render_page(state: &AppState) -> String {
    let units = vec![
        unit_view(state, "daemon", "Daemon", config::daemon_unit()).await,
        unit_view(state, "shell", "Shell", config::shell_unit()).await,
        unit_view(state, "panel", "Panel", config::panel_unit()).await,
    ];

    let active_res = state.ipc.command("hypr-active").await;
    let clients_res = state.ipc.command("hypr-clients").await;
    let monitors_res = state.ipc.command("hypr-monitors").await;
    // Reachable if any one of the three succeeded — a single command
    // failing (e.g. a transient IPC hiccup) shouldn't blank the whole
    // section when the others came back fine.
    let hypr_available = active_res.is_ok() || clients_res.is_ok() || monitors_res.is_ok();

    let top_processes = match state.recovery.top_processes().await {
        Ok(out) => out,
        Err(e) => format!("ps failed: {e}"),
    };

    let tmpl = ProcessesTemplate {
        active: "processes",
        units,
        hypr_available,
        hypr_active: pretty_or_raw(active_res),
        hypr_clients: pretty_or_raw(clients_res),
        hypr_monitors: pretty_or_raw(monitors_res),
        top_processes,
    };
    tmpl.render()
        .unwrap_or_else(|e| format!("<p class=\"banner banner-error\">render error: {e}</p>"))
}

async fn unit_view(
    state: &AppState,
    key: &'static str,
    label: &'static str,
    unit: String,
) -> UnitView {
    let unit_state = state.recovery.unit_active(&unit).await;
    UnitView {
        key,
        label,
        unit,
        state: unit_state,
    }
}

fn pretty_or_raw(res: Result<String, IpcError>) -> String {
    match res {
        Ok(s) => match serde_json::from_str::<serde_json::Value>(&s) {
            Ok(v) => serde_json::to_string_pretty(&v).unwrap_or(s),
            Err(_) => s,
        },
        Err(e) => e.to_string(),
    }
}

#[derive(Template)]
#[template(path = "processes_result.html")]
struct ProcessesResultTemplate {
    ok: bool,
    message: String,
}

fn result_html(ok: bool, message: &str) -> String {
    let tmpl = ProcessesResultTemplate {
        ok,
        message: message.to_string(),
    };
    tmpl.render()
        .unwrap_or_else(|e| format!("<p class=\"banner banner-error\">render error: {e}</p>"))
}

/// `POST /processes/restart/:key` — restart one of the three tv-shell units.
/// `key` is matched against a fixed set (`daemon`/`shell`/`panel`) and
/// resolved to the real unit name server-side — never an arbitrary
/// client-supplied unit name reaches `systemctl`.
pub async fn restart(
    State(state): State<SharedState>,
    Path(key): Path<String>,
) -> impl IntoResponse {
    Html(render_restart(&state, &key).await)
}

pub async fn render_restart(state: &AppState, key: &str) -> String {
    let unit = match key {
        "daemon" => config::daemon_unit(),
        "shell" => config::shell_unit(),
        "panel" => config::panel_unit(),
        other => return result_html(false, &format!("unknown unit key {other:?}")),
    };
    match state.recovery.restart_unit(&unit).await {
        Ok(out) => result_html(true, &format!("restarted {unit}\n{out}")),
        Err(e) => result_html(false, &format!("restart {unit} failed: {e}")),
    }
}
