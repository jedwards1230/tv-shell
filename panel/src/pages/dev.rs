//! `/dev` — the operator recovery page: deploy/build/restart/reboot/suspend
//! actions. Deploy/build/restart-daemon/restart-shell prefer the daemon HTTP
//! bridge and fall back to direct exec when the bridge is unconfigured or
//! unreachable (deploy has no exec equivalent — it needs the daemon's own
//! git checkout). Reboot/suspend always go through direct exec. All
//! destructive exec calls are single-flighted inside [`crate::exec::Recovery`].

use askama::Template;
use axum::body::Body;
use axum::extract::State;
use axum::http::{header, StatusCode};
use axum::response::{Html, IntoResponse, Response};
use axum::Form;
use serde::Deserialize;

use crate::bridge::BridgeError;
use crate::state::{AppState, SharedState};

#[derive(Template)]
#[template(path = "dev.html")]
struct DevTemplate {
    active: &'static str,
    daemon_up: bool,
    bridge_configured: bool,
}

/// `GET /dev` — probes daemon reachability (bridge `dev_status`, else IPC
/// `status`) and renders the action panel with an up/down banner.
pub async fn page(State(state): State<SharedState>) -> impl IntoResponse {
    Html(render_page(&state).await)
}

pub async fn render_page(state: &AppState) -> String {
    let daemon_up = probe_daemon_up(state).await;
    let tmpl = DevTemplate {
        active: "dev",
        daemon_up,
        bridge_configured: state.cfg.http_bridge_base.is_some(),
    };
    tmpl.render()
        .unwrap_or_else(|e| format!("<p class=\"banner banner-error\">render error: {e}</p>"))
}

async fn probe_daemon_up(state: &AppState) -> bool {
    if state.bridge.dev_status().await.is_ok() {
        return true;
    }
    state.ipc.command("status").await.is_ok()
}

#[derive(Template)]
#[template(path = "dev_result.html")]
struct DevResultTemplate {
    tier: &'static str,
    action: &'static str,
    ok: bool,
    output: String,
}

fn result_html(tier: &'static str, action: &'static str, ok: bool, output: &str) -> String {
    let tmpl = DevResultTemplate {
        tier,
        action,
        ok,
        output: output.to_string(),
    };
    tmpl.render()
        .unwrap_or_else(|e| format!("<p class=\"banner banner-error\">render error: {e}</p>"))
}

/// `true` when the bridge failure means "no bridge available at all" (so the
/// caller should fall back to direct exec) as opposed to "the bridge is up
/// but the operation itself failed" ([`BridgeError::Status`]).
fn bridge_unavailable(e: &BridgeError) -> bool {
    matches!(e, BridgeError::NotConfigured | BridgeError::Unreachable(_))
}

#[derive(Deserialize)]
pub struct DeployForm {
    git_ref: Option<String>,
}

/// `POST /dev/deploy` — bridge only (no exec equivalent for a git deploy).
pub async fn deploy(
    State(state): State<SharedState>,
    Form(form): Form<DeployForm>,
) -> impl IntoResponse {
    let git_ref = form.git_ref.filter(|s| !s.trim().is_empty());
    Html(render_deploy(&state, git_ref.as_deref()).await)
}

async fn render_deploy(state: &AppState, git_ref: Option<&str>) -> String {
    match state.bridge.deploy(git_ref).await {
        Ok(body) => result_html("Bridge", "deploy", true, &body),
        Err(e) => result_html(
            "Bridge",
            "deploy",
            false,
            &format!(
                "{e} — deploy requires the daemon HTTP bridge (no direct-exec equivalent); \
                 try Restart daemon or Build instead"
            ),
        ),
    }
}

/// `POST /dev/build` — bridge if the daemon is up, else direct exec.
pub async fn build(State(state): State<SharedState>) -> impl IntoResponse {
    Html(render_build(&state).await)
}

async fn render_build(state: &AppState) -> String {
    match state.bridge.build().await {
        Ok(body) => result_html("Bridge", "build", true, &body),
        Err(e) if bridge_unavailable(&e) => match state.recovery.build_daemon().await {
            Ok(body) => result_html("Direct exec", "build", true, &body),
            Err(e2) => result_html("Direct exec", "build", false, &e2.to_string()),
        },
        Err(e) => result_html("Bridge", "build", false, &e.to_string()),
    }
}

/// `POST /dev/restart-daemon` — bridge if the daemon is up, else direct exec.
pub async fn restart_daemon(State(state): State<SharedState>) -> impl IntoResponse {
    Html(render_restart_daemon(&state).await)
}

async fn render_restart_daemon(state: &AppState) -> String {
    match state.bridge.restart_daemon().await {
        Ok(body) => result_html("Bridge", "restart-daemon", true, &body),
        Err(e) if bridge_unavailable(&e) => match state.recovery.restart_daemon().await {
            Ok(body) => result_html("Direct exec", "restart-daemon", true, &body),
            Err(e2) => result_html("Direct exec", "restart-daemon", false, &e2.to_string()),
        },
        Err(e) => result_html("Bridge", "restart-daemon", false, &e.to_string()),
    }
}

/// `POST /dev/restart-shell` — bridge if the daemon is up, else direct exec.
pub async fn restart_shell(State(state): State<SharedState>) -> impl IntoResponse {
    Html(render_restart_shell(&state).await)
}

async fn render_restart_shell(state: &AppState) -> String {
    match state.bridge.restart_shell().await {
        Ok(body) => result_html("Bridge", "restart-shell", true, &body),
        Err(e) if bridge_unavailable(&e) => match state.recovery.restart_shell().await {
            Ok(body) => result_html("Direct exec", "restart-shell", true, &body),
            Err(e2) => result_html("Direct exec", "restart-shell", false, &e2.to_string()),
        },
        Err(e) => result_html("Bridge", "restart-shell", false, &e.to_string()),
    }
}

/// `POST /dev/reboot` — always direct exec.
pub async fn reboot(State(state): State<SharedState>) -> impl IntoResponse {
    Html(render_reboot(&state).await)
}

async fn render_reboot(state: &AppState) -> String {
    match state.recovery.reboot().await {
        Ok(body) => result_html("Direct exec", "reboot", true, &body),
        Err(e) => result_html("Direct exec", "reboot", false, &e.to_string()),
    }
}

/// `POST /dev/suspend` — always direct exec.
pub async fn suspend(State(state): State<SharedState>) -> impl IntoResponse {
    Html(render_suspend(&state).await)
}

async fn render_suspend(state: &AppState) -> String {
    match state.recovery.suspend().await {
        Ok(body) => result_html("Direct exec", "suspend", true, &body),
        Err(e) => result_html("Direct exec", "suspend", false, &e.to_string()),
    }
}

// ---------------------------------------------------------------------------
// Screenshot viewer (M3)
// ---------------------------------------------------------------------------

#[derive(Template)]
#[template(path = "dev_screenshot.html")]
struct DevScreenshotTemplate {
    ok: bool,
    message: String,
    sha: String,
    branch: String,
    version: String,
    captured_at: String,
    cache_bust: u128,
}

/// `POST /dev/screenshot/capture` — calls the bridge screenshot endpoint to
/// confirm reachability and read provenance, then (on success) renders an
/// `<img>` pointing at the `GET /dev/screenshot` proxy route. The `<img>` tag
/// is only ever emitted when this call already succeeded, so a daemon-down
/// or bridge-unconfigured state degrades to a banner — never a broken image.
pub async fn screenshot_capture(State(state): State<SharedState>) -> impl IntoResponse {
    Html(render_screenshot_capture(&state).await)
}

pub async fn render_screenshot_capture(state: &AppState) -> String {
    match state.bridge.screenshot().await {
        Ok(shot) => {
            let tmpl = DevScreenshotTemplate {
                ok: true,
                message: String::new(),
                sha: shot.sha,
                branch: shot.branch,
                version: shot.version,
                captured_at: shot.captured_at,
                cache_bust: now_millis(),
            };
            tmpl.render().unwrap_or_else(|e| {
                format!("<p class=\"banner banner-error\">render error: {e}</p>")
            })
        }
        Err(e) => {
            let reason = if e.is_configured() {
                "unreachable"
            } else {
                "not configured"
            };
            let tmpl = DevScreenshotTemplate {
                ok: false,
                message: format!("HTTP bridge {reason} — see the banner above. ({e})"),
                sha: String::new(),
                branch: String::new(),
                version: String::new(),
                captured_at: String::new(),
                cache_bust: 0,
            };
            tmpl.render().unwrap_or_else(|e| {
                format!("<p class=\"banner banner-error\">render error: {e}</p>")
            })
        }
    }
}

fn now_millis() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}

/// `GET /dev/screenshot` — proxies the daemon's `GET /screenshot` PNG bytes
/// (`Content-Type: image/png`). Only ever linked from the DOM after
/// [`screenshot_capture`] has already confirmed the bridge is reachable, so
/// a direct hit here (bridge down between the two calls) degrades to a
/// `503` text body rather than corrupting an `<img>` tag's expected type.
pub async fn screenshot_png(State(state): State<SharedState>) -> Response {
    match state.bridge.screenshot().await {
        Ok(shot) => {
            let mut resp = Response::new(Body::from(shot.png));
            *resp.status_mut() = StatusCode::OK;
            resp.headers_mut().insert(
                header::CONTENT_TYPE,
                header::HeaderValue::from_static("image/png"),
            );
            resp
        }
        Err(e) => (
            StatusCode::SERVICE_UNAVAILABLE,
            format!("screenshot unavailable: {e}"),
        )
            .into_response(),
    }
}
