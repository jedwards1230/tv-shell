//! `/dashboard` (also `/`) — the home page: daemon status, build identity,
//! system telemetry, storage, controllers, and systemd unit states. Degrades
//! gracefully (never a 500) when the daemon's IPC socket is unreachable.

use askama::Template;
use axum::extract::State;
use axum::response::{Html, IntoResponse};
use serde::Deserialize;

use crate::config;
use crate::state::{AppState, SharedState};

#[derive(Template)]
#[template(path = "dashboard.html")]
struct DashboardTemplate {
    active: &'static str,
}

/// `GET /` and `GET /dashboard` — the page shell. The tile region is filled
/// in by an htmx poll against `/dashboard/tiles`.
pub async fn page(State(_state): State<SharedState>) -> impl IntoResponse {
    super::render(DashboardTemplate {
        active: "dashboard",
    })
}

/// `GET /dashboard/tiles` — the polled partial.
pub async fn tiles(State(state): State<SharedState>) -> impl IntoResponse {
    Html(render_tiles(&state).await)
}

#[derive(Deserialize)]
struct BuildInfo {
    version: String,
    sha: String,
    branch: String,
}

#[derive(Deserialize)]
struct SysStatus {
    os: String,
    kernel: String,
    hostname: String,
    uptime: String,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct SysMetrics {
    cpu_pct: f64,
    mem_used: u64,
    mem_total: u64,
    mem_pct: u8,
    load1: f64,
    /// Best-effort: capture as raw JSON values rather than a strict shape so
    /// a daemon-side field tweak never breaks dashboard rendering.
    #[serde(default)]
    temps: Vec<serde_json::Value>,
}

#[derive(Deserialize)]
struct MountEntry {
    mount: String,
    size: u64,
    used: u64,
    #[allow(dead_code)]
    avail: u64,
    pct: u8,
}

#[derive(Deserialize)]
struct Pad {
    #[allow(dead_code)]
    id: String,
    index: u32,
    name: String,
    grabbed: bool,
}

struct TempView {
    label: String,
    celsius: String,
}

struct MountView {
    mount: String,
    size: String,
    used: String,
    pct: u8,
}

struct PadView {
    index: u32,
    name: String,
    grabbed: bool,
}

#[derive(Template)]
#[template(path = "dashboard_tiles.html")]
struct DashboardTilesTemplate {
    reachable: bool,
    status_text: String,
    version: String,
    sha: String,
    branch: String,
    os: String,
    kernel: String,
    hostname: String,
    uptime: String,
    cpu_pct: String,
    mem_used: String,
    mem_total: String,
    mem_pct: u8,
    load1: String,
    temps: Vec<TempView>,
    mounts: Vec<MountView>,
    pads: Vec<PadView>,
    daemon_unit_state: String,
    shell_unit_state: String,
    panel_unit_state: String,
}

/// Build the dashboard tiles partial HTML. Queries the IPC socket for
/// `status`/`build-info`/`sys-status`/`sys-metrics`/`storage-status`/
/// `get-pads`, and exec for the three systemd unit states. When the daemon
/// is unreachable (the `status` probe fails with
/// [`crate::ipc::IpcError::is_unreachable`]), renders a degraded view that
/// still shows unit states and a link to `/dev` — never a 500.
pub async fn render_tiles(state: &AppState) -> String {
    let daemon_unit_state = state.recovery.unit_active(&config::daemon_unit()).await;
    let shell_unit_state = state.recovery.unit_active(&config::shell_unit()).await;
    let panel_unit_state = state.recovery.unit_active(&config::panel_unit()).await;

    let status = state.ipc.command("status").await;
    let reachable = match &status {
        Ok(_) => true,
        Err(e) => !e.is_unreachable(),
    };

    let status_text = match &status {
        Ok(s) => s.clone(),
        Err(e) => e.to_string(),
    };

    let tmpl = if !reachable {
        DashboardTilesTemplate {
            reachable: false,
            status_text,
            version: String::new(),
            sha: String::new(),
            branch: String::new(),
            os: String::new(),
            kernel: String::new(),
            hostname: String::new(),
            uptime: String::new(),
            cpu_pct: String::new(),
            mem_used: String::new(),
            mem_total: String::new(),
            mem_pct: 0,
            load1: String::new(),
            temps: Vec::new(),
            mounts: Vec::new(),
            pads: Vec::new(),
            daemon_unit_state,
            shell_unit_state,
            panel_unit_state,
        }
    } else {
        let build: BuildInfo = state
            .ipc
            .command_json("build-info")
            .await
            .unwrap_or_else(|_| BuildInfo {
                version: "unknown".into(),
                sha: "unknown".into(),
                branch: "unknown".into(),
            });
        let sys: SysStatus = state
            .ipc
            .command_json("sys-status")
            .await
            .unwrap_or_else(|_| SysStatus {
                os: "unknown".into(),
                kernel: "unknown".into(),
                hostname: "unknown".into(),
                uptime: "unknown".into(),
            });
        let metrics: SysMetrics = state
            .ipc
            .command_json("sys-metrics")
            .await
            .unwrap_or_default();
        let mounts: Vec<MountEntry> = state
            .ipc
            .command_json("storage-status")
            .await
            .unwrap_or_default();
        let pads: Vec<Pad> = state.ipc.command_json("get-pads").await.unwrap_or_default();

        let temps = metrics
            .temps
            .iter()
            .map(|v| {
                let label = v
                    .get("label")
                    .and_then(|l| l.as_str())
                    .unwrap_or("sensor")
                    .to_string();
                let celsius = v
                    .get("celsius")
                    .and_then(|c| c.as_f64())
                    .map(|c| format!("{c:.1}"))
                    .unwrap_or_else(|| "?".to_string());
                TempView { label, celsius }
            })
            .collect();

        let mounts = mounts
            .into_iter()
            .map(|m| MountView {
                mount: m.mount,
                size: human_bytes(m.size),
                used: human_bytes(m.used),
                pct: m.pct,
            })
            .collect();

        let pads = pads
            .into_iter()
            .map(|p| PadView {
                index: p.index,
                name: p.name,
                grabbed: p.grabbed,
            })
            .collect();

        DashboardTilesTemplate {
            reachable: true,
            status_text,
            version: build.version,
            sha: build.sha,
            branch: build.branch,
            os: sys.os,
            kernel: sys.kernel,
            hostname: sys.hostname,
            uptime: sys.uptime,
            cpu_pct: format!("{:.1}", metrics.cpu_pct),
            mem_used: human_bytes(metrics.mem_used),
            mem_total: human_bytes(metrics.mem_total),
            mem_pct: metrics.mem_pct,
            load1: format!("{:.2}", metrics.load1),
            temps,
            mounts,
            pads,
            daemon_unit_state,
            shell_unit_state,
            panel_unit_state,
        }
    };

    tmpl.render()
        .unwrap_or_else(|e| format!("<p class=\"banner banner-error\">render error: {e}</p>"))
}

/// Human-readable byte size (GiB with one decimal for anything ≥1 GiB, MiB
/// otherwise).
fn human_bytes(bytes: u64) -> String {
    const GIB: f64 = 1024.0 * 1024.0 * 1024.0;
    const MIB: f64 = 1024.0 * 1024.0;
    let b = bytes as f64;
    if b >= GIB {
        format!("{:.1} GiB", b / GIB)
    } else {
        format!("{:.1} MiB", b / MIB)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn human_bytes_formats_gib_and_mib() {
        assert_eq!(human_bytes(2 * 1024 * 1024 * 1024), "2.0 GiB");
        assert_eq!(human_bytes(512 * 1024 * 1024), "512.0 MiB");
    }
}
