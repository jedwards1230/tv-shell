//! `/logs` — shell log (via the daemon HTTP bridge) and daemon log (via
//! `journalctl`, direct exec) side by side. Never 500s on a data-source
//! failure — each panel degrades inline.

use askama::Template;
use axum::extract::{Query, State};
use axum::response::{Html, IntoResponse};
use serde::Deserialize;

use crate::config;
use crate::state::{AppState, SharedState};

const DEFAULT_LINES: usize = 200;
const MAX_LINES: usize = 1000;

#[derive(Template)]
#[template(path = "logs.html")]
struct LogsTemplate {
    active: &'static str,
}

/// `GET /logs` — the page shell.
pub async fn page(State(_state): State<SharedState>) -> impl IntoResponse {
    super::render(LogsTemplate { active: "logs" })
}

#[derive(Deserialize)]
pub struct LogsQuery {
    lines: Option<usize>,
    filter: Option<String>,
}

/// `GET /logs/view` — the refreshable log partial.
pub async fn view(
    State(state): State<SharedState>,
    Query(q): Query<LogsQuery>,
) -> impl IntoResponse {
    let lines = q.lines.unwrap_or(DEFAULT_LINES).clamp(1, MAX_LINES);
    Html(render_view(&state, lines, q.filter.as_deref()).await)
}

#[derive(Template)]
#[template(path = "logs_view.html")]
struct LogsViewTemplate {
    shell_available: bool,
    shell_log: String,
    shell_message: String,
    daemon_log: String,
}

/// Build the log-panels partial HTML for `lines`/`filter`. The shell log
/// comes from the daemon HTTP bridge (`bridge.dev_logs`); when the bridge is
/// unconfigured or unreachable, that panel shows an inline message instead
/// of erroring. The daemon log comes from `journalctl` via the exec tier and
/// always attempts to render (a read failure degrades to an inline message
/// too).
pub async fn render_view(state: &AppState, lines: usize, filter: Option<&str>) -> String {
    let (shell_available, shell_log, shell_message) =
        match state.bridge.dev_logs(lines, filter).await {
            Ok(body) => (true, body, String::new()),
            Err(e) => {
                let reason = if e.is_configured() {
                    "unreachable"
                } else {
                    "not configured"
                };
                (
                    false,
                    String::new(),
                    format!("HTTP bridge {reason} — see the Dev page. ({e})"),
                )
            }
        };

    let daemon_log = match state
        .recovery
        .journal_unit(&config::daemon_journal_unit(), lines, filter)
        .await
    {
        Ok(body) => body,
        Err(e) => format!("journal read failed: {e}"),
    };

    let tmpl = LogsViewTemplate {
        shell_available,
        shell_log,
        shell_message,
        daemon_log,
    };
    tmpl.render()
        .unwrap_or_else(|e| format!("<p class=\"banner banner-error\">render error: {e}</p>"))
}
