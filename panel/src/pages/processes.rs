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
use serde::Deserialize;

use crate::config;
use crate::ipc::IpcError;
use crate::state::{AppState, SharedState};

struct UnitView {
    key: &'static str,
    label: &'static str,
    unit: String,
    state: String,
    /// A dedicated dot/word status pair (color always paired with explicit
    /// text — #6) — `dot_class`/`state_word` mirror
    /// `pages::dashboard`'s tile treatment for the same unit-state strings.
    dot_class: &'static str,
    state_word: &'static str,
    /// Confirm-dialog text for this unit's Restart button. The panel's own
    /// unit gets a distinct message (#5): restarting it drops the very page
    /// the operator is looking at, so the confirm says so explicitly rather
    /// than reusing the generic "Restart X now?" wording.
    confirm: String,
}

/// Map a raw `systemctl is-active` string to a colored dot class + a short
/// status word — color is always paired with explicit text (#6), never the
/// dot alone. `active` is the healthy state; `failed` is the one state that
/// reads as an outright problem; everything else (`inactive`, `activating`,
/// `deactivating`, `unknown`, ...) is a neutral "not running" state rather
/// than an alarm, since a stopped-but-not-failed unit isn't necessarily
/// wrong (e.g. between restarts).
fn unit_dot(state: &str) -> (&'static str, &'static str) {
    match state {
        "active" => ("dot-ok", "active"),
        "failed" => ("dot-error", "failed"),
        "activating" => ("dot-warn", "activating"),
        "deactivating" => ("dot-warn", "deactivating"),
        "inactive" => ("dot-neutral", "inactive"),
        _ => ("dot-neutral", "unknown"),
    }
}

/// `hypr-clients` reply shape (`docs/IPC_PROTOCOL.md` § `hypr-clients`).
#[derive(Deserialize)]
struct HyprClientJson {
    class: String,
    title: String,
    address: String,
    workspace: String,
}

struct HyprClientView {
    class: String,
    title: String,
    workspace: String,
    address: String,
}

/// One row of the `ps axo pid,pcpu,pmem,comm --sort=-pcpu` snapshot (#15 —
/// rendered as a styled table instead of raw `<pre>` text).
struct ProcRow {
    pid: String,
    pcpu: String,
    pmem: String,
    comm: String,
}

/// Parse `ps axo pid,pcpu,pmem,comm`'s whitespace-column output into rows,
/// skipping the header line. `comm` is whatever's left after the first three
/// whitespace-delimited fields (defensive — this is just a process name, not
/// an argv, so it shouldn't itself contain spaces, but joining the remainder
/// rather than taking a fixed 4th token is cheap insurance). A line that
/// doesn't even have 3 columns (never expected from real `ps` output) is
/// skipped rather than panicking or emitting a garbled row.
fn parse_top_processes(raw: &str) -> Vec<ProcRow> {
    raw.lines()
        .skip(1) // header: "PID %CPU %MEM COMMAND"
        .filter_map(|line| {
            let mut parts = line.split_whitespace();
            let pid = parts.next()?.to_string();
            let pcpu = parts.next()?.to_string();
            let pmem = parts.next()?.to_string();
            let comm: String = parts.collect::<Vec<_>>().join(" ");
            if comm.is_empty() {
                return None;
            }
            Some(ProcRow {
                pid,
                pcpu,
                pmem,
                comm,
            })
        })
        .collect()
}

#[derive(Template)]
#[template(path = "processes.html")]
struct ProcessesTemplate {
    active: &'static str,
    units: Vec<UnitView>,
    hypr_available: bool,
    hypr_active: String,
    hypr_clients_rows: Vec<HyprClientView>,
    hypr_clients_error: String,
    hypr_monitors: String,
    top_rows: Vec<ProcRow>,
    top_error: String,
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

    let (hypr_clients_rows, hypr_clients_error) = match clients_res {
        Ok(s) => match serde_json::from_str::<Vec<HyprClientJson>>(&s) {
            Ok(list) => (
                list.into_iter()
                    .map(|c| HyprClientView {
                        class: c.class,
                        title: c.title,
                        workspace: c.workspace,
                        address: c.address,
                    })
                    .collect(),
                String::new(),
            ),
            Err(e) => (
                Vec::new(),
                format!("failed to parse hypr-clients reply: {e}"),
            ),
        },
        Err(e) => (Vec::new(), e.to_string()),
    };

    let (top_rows, top_error) = match state.recovery.top_processes().await {
        Ok(out) => (parse_top_processes(&out), String::new()),
        Err(e) => (Vec::new(), format!("ps failed: {e}")),
    };

    let tmpl = ProcessesTemplate {
        active: "processes",
        units,
        hypr_available,
        hypr_active: pretty_or_raw(active_res),
        hypr_clients_rows,
        hypr_clients_error,
        hypr_monitors: pretty_or_raw(monitors_res),
        top_rows,
        top_error,
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
    let (dot_class, state_word) = unit_dot(&unit_state);
    let confirm = if key == "panel" {
        format!(
            "Restart {unit} now? This is the panel serving THIS page — it will disconnect \
             immediately. Reload the page after a few seconds to reconnect."
        )
    } else {
        format!("Restart {unit} now?")
    };
    UnitView {
        key,
        label,
        unit,
        state: unit_state,
        dot_class,
        state_word,
        confirm,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_top_processes_skips_header_and_splits_columns() {
        let raw = "  PID  %CPU  %MEM COMMAND\n\
                      1234  12.3   4.5 firefox\n\
                       567   0.1   0.2 systemd";
        let rows = parse_top_processes(raw);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].pid, "1234");
        assert_eq!(rows[0].pcpu, "12.3");
        assert_eq!(rows[0].pmem, "4.5");
        assert_eq!(rows[0].comm, "firefox");
        assert_eq!(rows[1].comm, "systemd");
    }

    #[test]
    fn parse_top_processes_joins_multi_word_comm() {
        // `comm` shouldn't realistically contain spaces, but the parser
        // shouldn't silently drop trailing tokens if it ever does.
        let raw = "PID %CPU %MEM COMMAND\n1 0.0 0.0 some odd name";
        let rows = parse_top_processes(raw);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].comm, "some odd name");
    }

    #[test]
    fn parse_top_processes_skips_malformed_lines() {
        let raw = "PID %CPU %MEM COMMAND\n1 2 3 ok\ntoo short\n";
        let rows = parse_top_processes(raw);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].comm, "ok");
    }

    #[test]
    fn parse_top_processes_empty_body_yields_no_rows() {
        assert!(parse_top_processes("PID %CPU %MEM COMMAND\n").is_empty());
        assert!(parse_top_processes("").is_empty());
    }

    #[test]
    fn unit_dot_maps_active_and_failed_to_distinct_colors() {
        assert_eq!(unit_dot("active"), ("dot-ok", "active"));
        assert_eq!(unit_dot("failed"), ("dot-error", "failed"));
        assert_eq!(unit_dot("inactive"), ("dot-neutral", "inactive"));
        assert_eq!(unit_dot("something-unexpected"), ("dot-neutral", "unknown"));
    }
}
