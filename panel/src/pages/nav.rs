//! Global daemon-reachability dot in `base.html`'s topnav. Deliberately its
//! own tiny module rather than owned by any one page — `base.html`/nav is
//! M1 territory per the page-ownership contract, and this partial is nav
//! chrome, not a page.

use std::time::Duration;

use axum::extract::State;
use axum::response::{Html, IntoResponse};

use crate::state::{AppState, SharedState};

/// Timeout for the nav dot's `status` probe — deliberately much shorter
/// than [`crate::ipc::IpcClient`]'s 3s default so a hung/unreachable daemon
/// can never make the ~10s polling loop pile up requests.
const PROBE_TIMEOUT: Duration = Duration::from_millis(800);

/// `GET /nav/daemon-status` — the topnav's polled partial (`hx-trigger="load,
/// every 10s"`, `hx-swap="innerHTML"` on the containing span).
pub async fn daemon_status_dot(State(state): State<SharedState>) -> impl IntoResponse {
    Html(render_dot(&state).await)
}

/// A fast, cheap `status` probe: green dot when the daemon answers within
/// [`PROBE_TIMEOUT`], red when it doesn't. Always succeeds — this must never
/// slow or break page navigation.
pub async fn render_dot(state: &AppState) -> String {
    let reachable = state
        .ipc
        .command_timeout("status", PROBE_TIMEOUT)
        .await
        .is_ok();
    let (dot_class, title) = if reachable {
        ("dot-ok", "daemon reachable")
    } else {
        ("dot-error", "daemon unreachable")
    };
    format!(r#"<span class="dot {dot_class}" title="{title}"></span>daemon"#)
}
