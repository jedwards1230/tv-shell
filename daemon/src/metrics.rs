//! Observability metrics: app-specific counters + a Prometheus/OpenMetrics text
//! renderer shared by the `/metrics` HTTP route and the node_exporter
//! textfile-collector writer.
//!
//! **Design goals** (see `docs/OBSERVABILITY.md`):
//! - Emit Linux-native, standard, self-describing formats so ANY consumer can
//!   collect it their way. Collection/forwarding stays out of this repo.
//! - The genuinely valuable signal is the **app-specific counters**
//!   (`game_shell_*_total`) that node_exporter cannot give: input events,
//!   intents, shell↔game transitions, pad joins/leaves, shell restarts.
//! - Resource gauges (cpu/mem/load/temps) are a convenience reusing the existing
//!   `system::SysMetrics` reader; they are better sourced from node_exporter if
//!   one is present on the host.
//!
//! **Shared render**: [`render`] produces the full exposition text used by both
//! the HTTP endpoint and the textfile writer, so the two never drift.
//!
//! **Cross-platform**: no Linux-only imports — the struct and renderer compile
//! and unit-test on macOS/CI. The sys gauges degrade to zero/empty there (see
//! `system::sys_metrics_json` / `SysMetrics`).

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

/// Shared, cheap-to-update counters for daemon activity. Held behind an `Arc`
/// and cloned into every subsystem that records an event. All increments are
/// `Relaxed` — these are independent monotonic counters with no inter-counter
/// ordering requirement, and the reader (textfile/HTTP) only needs eventual
/// consistency.
#[derive(Debug, Default)]
pub struct Metrics {
    /// Raw evdev input events read from the gamepad fleet and processed by the
    /// input runtime (`handle_event`). The hot path; `Relaxed` add is a single
    /// atomic instruction.
    pub input_events: AtomicU64,
    /// `intent:<name>` broadcasts accepted and emitted on the event bus
    /// (IPC `intent`, HTTP `/intent/*`, MCP `send_intent`, and the gamepad
    /// Home-tap/Home-hold all funnel through `Shared::publish(Event::Intent)`).
    pub intents_emitted: AtomicU64,
    /// Shell↔game presenter transitions (`grab`/`release`/`handoff`). Each
    /// presenter switch increments this once.
    pub transitions: AtomicU64,
    /// Pads that joined the fleet (hot-join or initial enumeration).
    pub pad_joins: AtomicU64,
    /// Pads that left the fleet (USB/Bluetooth disconnect).
    pub pad_leaves: AtomicU64,
    /// Daemon starts. Incremented once at startup; because the daemon re-execs
    /// on `/dev/restart-daemon` and is otherwise a supervised long-runner, the
    /// running total is the shell-input restart count for this boot session.
    pub shell_restarts: AtomicU64,

    // --- Input-runtime supervision (in-process respawn on panic) -------------
    /// Input-runtime liveness gauge (1 = running, 0 = dead). Set by the input
    /// runtime supervisor: 1 while the supervised event loop runs, 0 during a
    /// respawn gap and after retries are exhausted. A gauge stored as `AtomicU64`
    /// (0/1) alongside the counters for uniform access.
    pub runtime_up: AtomicU64,
    /// In-process input-runtime respawns after a caught panic. DISTINCT from
    /// `shell_restarts` (whole-daemon process starts): this counts the supervisor
    /// rebuilding the input event loop without re-execing the daemon, so a
    /// nonzero value flags a recurring panic in the input path.
    pub runtime_restarts: AtomicU64,
    /// Detected grab-state drift: a pad's physical `EVIOCGRAB` disagreed with the
    /// presenter policy (`should_grab`) after a transition. Should stay 0; a
    /// nonzero value means the daemon's grab bookkeeping and the kernel diverged.
    pub grab_invariant_violations: AtomicU64,

    // --- Dev/deployment action counters (HTTP-bridge handlers) ---------------
    /// `POST /dev/deploy` attempts that succeeded (git fetch+checkout+reset OK).
    pub deploy_ok: AtomicU64,
    /// `POST /dev/deploy` attempts that failed (git error). Together with
    /// `deploy_ok` these render as `game_shell_deploy_total{outcome="ok|error"}`.
    pub deploy_err: AtomicU64,
    /// `POST /dev/build` attempts (build via scripts/build-daemon.sh + install).
    pub build_actions: AtomicU64,
    /// `POST /dev/restart-shell` attempts (kill + relaunch quickshell).
    pub restart_shell_actions: AtomicU64,
    /// `POST /dev/restart-daemon` attempts (re-exec the daemon). Counted when the
    /// re-exec is requested — the response is written before the process image is
    /// replaced, so the increment is durable in this process's metrics until the
    /// re-exec lands (the new process starts its own counters at zero).
    pub restart_daemon_actions: AtomicU64,
    /// Times a shell restart (HTTP `/dev/restart-shell` or MCP `restart_shell`,
    /// which share `dev_restart_shell`) detected >1 quickshell process after the
    /// restart settle — the #254 stacked-instance bug; should stay 0.
    pub quickshell_multi_instance: AtomicU64,
}

impl Metrics {
    /// Build a fresh, zeroed metrics set behind an `Arc` for sharing.
    pub fn new() -> Arc<Metrics> {
        Arc::new(Metrics::default())
    }

    #[inline]
    pub fn inc_input_events(&self) {
        self.input_events.fetch_add(1, Ordering::Relaxed);
    }

    #[inline]
    pub fn inc_intents(&self) {
        self.intents_emitted.fetch_add(1, Ordering::Relaxed);
    }

    #[inline]
    pub fn inc_transitions(&self) {
        self.transitions.fetch_add(1, Ordering::Relaxed);
    }

    #[inline]
    pub fn inc_pad_joins(&self) {
        self.pad_joins.fetch_add(1, Ordering::Relaxed);
    }

    #[inline]
    pub fn inc_pad_leaves(&self) {
        self.pad_leaves.fetch_add(1, Ordering::Relaxed);
    }

    #[inline]
    pub fn inc_shell_restarts(&self) {
        self.shell_restarts.fetch_add(1, Ordering::Relaxed);
    }

    /// Set the input-runtime liveness gauge (`true` = running, `false` = dead).
    #[inline]
    pub fn set_runtime_up(&self, up: bool) {
        self.runtime_up.store(up as u64, Ordering::Relaxed);
    }

    /// Count one in-process input-runtime respawn (supervisor caught a panic).
    #[inline]
    pub fn inc_runtime_restarts(&self) {
        self.runtime_restarts.fetch_add(1, Ordering::Relaxed);
    }

    /// Count one detected grab-invariant violation (grab-state drift).
    #[inline]
    pub fn inc_grab_invariant_violations(&self) {
        self.grab_invariant_violations
            .fetch_add(1, Ordering::Relaxed);
    }

    /// Record a `/dev/deploy` outcome (`true` = success, `false` = failure).
    #[inline]
    pub fn inc_deploy(&self, ok: bool) {
        if ok {
            self.deploy_ok.fetch_add(1, Ordering::Relaxed);
        } else {
            self.deploy_err.fetch_add(1, Ordering::Relaxed);
        }
    }

    #[inline]
    pub fn inc_build(&self) {
        self.build_actions.fetch_add(1, Ordering::Relaxed);
    }

    #[inline]
    pub fn inc_restart_shell(&self) {
        self.restart_shell_actions.fetch_add(1, Ordering::Relaxed);
    }

    #[inline]
    pub fn inc_restart_daemon(&self) {
        self.restart_daemon_actions.fetch_add(1, Ordering::Relaxed);
    }

    #[inline]
    pub fn inc_quickshell_multi_instance(&self) {
        self.quickshell_multi_instance
            .fetch_add(1, Ordering::Relaxed);
    }
}

/// Current-deployment provenance for the `game_shell_build_info` info-metric.
/// Resolved live (re-read on each render) from the same `capture_meta()` source
/// that backs the `/screenshot` `X-GameShell-*` headers and `/dev/status`, so a
/// `/dev/deploy` HEAD swap under the live daemon is reflected next render.
#[derive(Debug, Clone)]
pub struct BuildInfo {
    pub sha: String,
    pub branch: String,
    pub version: String,
}

/// Escape a Prometheus label value per the exposition format: backslash,
/// double-quote, and newline are the only characters that must be escaped.
fn escape_label_value(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            other => out.push(other),
        }
    }
    out
}

/// Format an `f64` for the exposition format. Prometheus accepts plain decimal;
/// we keep it compact and locale-independent (Rust's `Display` for f64 already
/// uses `.` and never a thousands separator).
fn fmt_f64(v: f64) -> String {
    // Guard against NaN/inf which are invalid in the text format apart from the
    // literal `+Inf`/`-Inf`/`NaN` tokens — clamp to 0 for our gauge use.
    if v.is_finite() {
        format!("{v}")
    } else {
        "0".to_string()
    }
}

/// Render the full OpenMetrics/Prometheus exposition text for the daemon.
///
/// `counters` supplies the app-specific `*_total` values; `sys` (optional)
/// supplies the convenience resource gauges; `build` (optional) supplies the
/// `game_shell_build_info` deployment identity. When an optional is `None` that
/// section is omitted — the counters are always emitted.
///
/// Every metric carries `# HELP` and `# TYPE` lines. All metrics are namespaced
/// `game_shell_`. The output ends with a trailing newline (required by the
/// node_exporter textfile collector parser).
pub fn render(
    counters: &Metrics,
    sys: Option<&crate::system::SysMetrics>,
    build: Option<&BuildInfo>,
) -> String {
    let mut out = String::with_capacity(1024);

    // ── Current-deployment info metric (always value 1; identity in labels) ───
    if let Some(b) = build {
        out.push_str(
            "# HELP game_shell_build_info Currently deployed game-shell revision (value is always 1; identity is in the labels).\n",
        );
        out.push_str("# TYPE game_shell_build_info gauge\n");
        out.push_str(&format!(
            "game_shell_build_info{{sha=\"{}\",branch=\"{}\",version=\"{}\"}} 1\n",
            escape_label_value(&b.sha),
            escape_label_value(&b.branch),
            escape_label_value(&b.version),
        ));
    }

    // ── App-specific counters ────────────────────────────────────────────────
    let counter = |out: &mut String, name: &str, help: &str, val: u64| {
        out.push_str(&format!("# HELP {name} {help}\n"));
        out.push_str(&format!("# TYPE {name} counter\n"));
        out.push_str(&format!("{name} {val}\n"));
    };

    counter(
        &mut out,
        "game_shell_input_events_total",
        "Raw gamepad input events read and processed by the input runtime.",
        counters.input_events.load(Ordering::Relaxed),
    );
    counter(
        &mut out,
        "game_shell_intents_emitted_total",
        "Shell intents broadcast on the event bus (intent:<name>).",
        counters.intents_emitted.load(Ordering::Relaxed),
    );
    counter(
        &mut out,
        "game_shell_transitions_total",
        "Shell<->game presenter transitions (grab/release/handoff).",
        counters.transitions.load(Ordering::Relaxed),
    );
    counter(
        &mut out,
        "game_shell_pad_joins_total",
        "Gamepads that joined the fleet (hot-join or initial enumeration).",
        counters.pad_joins.load(Ordering::Relaxed),
    );
    counter(
        &mut out,
        "game_shell_pad_leaves_total",
        "Gamepads that left the fleet (disconnect).",
        counters.pad_leaves.load(Ordering::Relaxed),
    );
    counter(
        &mut out,
        "game_shell_shell_restarts_total",
        "game-shell-input daemon starts observed this boot session.",
        counters.shell_restarts.load(Ordering::Relaxed),
    );

    // ── Input-runtime supervision ────────────────────────────────────────────
    counter(
        &mut out,
        "game_shell_input_runtime_restarts_total",
        "In-process input-runtime respawns after a panic (distinct from daemon process starts in game_shell_shell_restarts_total).",
        counters.runtime_restarts.load(Ordering::Relaxed),
    );
    counter(
        &mut out,
        "game_shell_grab_invariant_violations_total",
        "Grab-state drift detected: a pad's physical EVIOCGRAB disagreed with the presenter policy after a transition (should stay 0).",
        counters.grab_invariant_violations.load(Ordering::Relaxed),
    );
    // The input-runtime liveness gauge is app-level and ALWAYS emitted (unlike the
    // convenience sys gauges gated behind `Some(sys)` below): a scrape must be able
    // to alert on the runtime being dead even when no sys metrics are present.
    out.push_str(
        "# HELP game_shell_input_runtime_up Input runtime liveness (1 = supervised event loop running, 0 = dead/panic-exhausted).\n",
    );
    out.push_str("# TYPE game_shell_input_runtime_up gauge\n");
    out.push_str(&format!(
        "game_shell_input_runtime_up {}\n",
        counters.runtime_up.load(Ordering::Relaxed),
    ));

    // ── Dev/deployment action counters ───────────────────────────────────────
    // deploy carries an outcome label so failed deploys are visible; one
    // HELP/TYPE block, two labelled samples.
    out.push_str(
        "# HELP game_shell_deploy_total /dev/deploy attempts via the HTTP bridge, by outcome.\n",
    );
    out.push_str("# TYPE game_shell_deploy_total counter\n");
    out.push_str(&format!(
        "game_shell_deploy_total{{outcome=\"ok\"}} {}\n",
        counters.deploy_ok.load(Ordering::Relaxed),
    ));
    out.push_str(&format!(
        "game_shell_deploy_total{{outcome=\"error\"}} {}\n",
        counters.deploy_err.load(Ordering::Relaxed),
    ));
    counter(
        &mut out,
        "game_shell_build_total",
        "/dev/build attempts via the HTTP bridge.",
        counters.build_actions.load(Ordering::Relaxed),
    );
    counter(
        &mut out,
        "game_shell_restart_shell_total",
        "/dev/restart-shell attempts via the HTTP bridge.",
        counters.restart_shell_actions.load(Ordering::Relaxed),
    );
    counter(
        &mut out,
        "game_shell_restart_daemon_total",
        "/dev/restart-daemon (re-exec) requests via the HTTP bridge.",
        counters.restart_daemon_actions.load(Ordering::Relaxed),
    );
    counter(
        &mut out,
        "game_shell_quickshell_multi_instance_total",
        "Times a shell restart (HTTP /dev/restart-shell or MCP restart_shell) detected >1 quickshell process after a restart settle (#254; should stay 0).",
        counters.quickshell_multi_instance.load(Ordering::Relaxed),
    );

    // ── Convenience resource gauges (better sourced from node_exporter) ───────
    if let Some(m) = sys {
        let gauge = |out: &mut String, name: &str, help: &str, val: String| {
            out.push_str(&format!("# HELP {name} {help}\n"));
            out.push_str(&format!("# TYPE {name} gauge\n"));
            out.push_str(&format!("{name} {val}\n"));
        };

        gauge(
            &mut out,
            "game_shell_cpu_percent",
            "Aggregate CPU utilisation 0..=100 (convenience; prefer node_exporter).",
            fmt_f64(m.cpu_pct),
        );
        gauge(
            &mut out,
            "game_shell_mem_used_bytes",
            "Used memory in bytes (convenience; prefer node_exporter).",
            m.mem_used.to_string(),
        );
        gauge(
            &mut out,
            "game_shell_mem_total_bytes",
            "Total memory in bytes (convenience; prefer node_exporter).",
            m.mem_total.to_string(),
        );
        gauge(
            &mut out,
            "game_shell_load1",
            "1-minute load average (convenience; prefer node_exporter).",
            fmt_f64(m.load1),
        );

        // Temperature gauges carry a `sensor` label. One HELP/TYPE block, then a
        // sample per sensor (multiple labelled samples of the same metric).
        if !m.temps.is_empty() {
            out.push_str(
                "# HELP game_shell_temperature_celsius Hardware temperature sensor reading in degrees Celsius (convenience; prefer node_exporter).\n",
            );
            out.push_str("# TYPE game_shell_temperature_celsius gauge\n");
            for t in &m.temps {
                out.push_str(&format!(
                    "game_shell_temperature_celsius{{sensor=\"{}\"}} {}\n",
                    escape_label_value(&t.label),
                    fmt_f64(t.celsius),
                ));
            }
        }
    }

    out
}

/// Resolve the current-deployment [`BuildInfo`] live from the shared
/// `capture_meta()` provenance resolver (same source as the `/screenshot`
/// `X-GameShell-*` headers and `/dev/status`). Async because it shells out to
/// `git`; callers `.await` this BEFORE `render_blocking` and pass the result in,
/// so a `/dev/deploy` HEAD swap is reflected on the next render (re-read on
/// render, not cached at startup).
pub async fn resolve_build_info() -> BuildInfo {
    let meta = crate::bridge_core::capture_meta().await;
    BuildInfo {
        sha: meta.sha,
        branch: meta.branch,
        version: meta.version.to_owned(),
    }
}

/// Read the live system metrics on a blocking thread and render the full
/// exposition text. `cpu_percent` sleeps ~200ms internally, so this MUST run on
/// the blocking pool (the textfile task and the HTTP handler both wrap it in
/// `spawn_blocking`).
///
/// `build` is resolved by the caller via [`resolve_build_info`] (async git) and
/// passed in, since this fn runs on the blocking pool and cannot `.await`.
pub fn render_blocking(counters: &Metrics, build: Option<BuildInfo>) -> String {
    let sys = crate::system::sys_metrics();
    render(counters, Some(&sys), build.as_ref())
}

// ─── node_exporter textfile-collector writer ─────────────────────────────────

// The metrics write interval now comes from `[observability].metrics_interval`
// (default 15, clamped to ≥1 by `DaemonConfig::metrics_interval_secs()`); there
// is no longer a local DEFAULT_INTERVAL_SECS const here.

/// Atomically write `text` to `path` via the temp-file + rename pattern the
/// node_exporter textfile collector requires (it reads `*.prom` files and a
/// partial read of a non-atomic write would surface a malformed scrape).
///
/// The temp file is created in the SAME directory as the target so the final
/// `rename(2)` is on one filesystem (cross-device rename fails). The temp name
/// carries the daemon pid to avoid collisions if two writers ever share a dir.
fn write_atomic(path: &std::path::Path, text: &str) -> std::io::Result<()> {
    use std::io::Write;
    let dir = path.parent().unwrap_or_else(|| std::path::Path::new("."));
    let pid = std::process::id();
    let tmp = dir.join(format!(
        ".{}.{}.tmp",
        path.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("game-shell"),
        pid
    ));
    {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(text.as_bytes())?;
        f.flush()?;
        // fsync so the rename publishes durable bytes, matching the collector's
        // atomic-write contract under power loss.
        let _ = f.sync_all();
    }
    // rename is atomic on the same filesystem; on error, clean up the temp file.
    match std::fs::rename(&tmp, path) {
        Ok(()) => Ok(()),
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            Err(e)
        }
    }
}

/// Background task: periodically render the metrics exposition text and write it
/// atomically to `textfile_path` (from `[observability].metrics_textfile`).
///
/// **Disabled when `textfile_path` is `None`/empty** — the task returns
/// immediately and no file is ever written. This is the PRIMARY metrics path
/// (node_exporter textfile collector); the `/metrics` HTTP route is the portable
/// alternative. `interval_secs` comes from `[observability].metrics_interval`
/// (already clamped ≥1 by `DaemonConfig::metrics_interval_secs`).
///
/// Mirrors the fire-and-forget spawn pattern of the other daemon actors: it logs
/// and degrades gracefully (a failed write is logged at warn, not fatal) and
/// never panics the daemon.
pub async fn run_textfile_writer(
    counters: Arc<Metrics>,
    textfile_path: Option<String>,
    interval_secs: u64,
) {
    let Some(path_str) = textfile_path.filter(|p| !p.is_empty()) else {
        // Unset/empty → writer disabled (no file). The /metrics route is unaffected.
        tracing::debug!(
            "metrics: [observability].metrics_textfile unset, textfile writer disabled"
        );
        return;
    };
    let path = std::path::PathBuf::from(path_str);
    let secs = interval_secs;
    tracing::info!(
        "metrics: writing textfile-collector metrics to {} every {secs}s",
        path.display()
    );

    let mut ticker = tokio::time::interval(std::time::Duration::from_secs(secs));
    loop {
        ticker.tick().await;
        // Resolve build identity live (async git) so a /dev/deploy HEAD swap is
        // reflected next render — then render on the blocking pool (sys_metrics()
        // sleeps ~200ms for the CPU sample).
        let build = resolve_build_info().await;
        let counters = Arc::clone(&counters);
        let text = match tokio::task::spawn_blocking(move || {
            render_blocking(&counters, Some(build))
        })
        .await
        {
            Ok(t) => t,
            Err(e) => {
                tracing::warn!("metrics: render task failed: {e}");
                continue;
            }
        };
        let path = path.clone();
        // The atomic write is also blocking I/O.
        let write_res = tokio::task::spawn_blocking(move || write_atomic(&path, &text)).await;
        match write_res {
            Ok(Ok(())) => {}
            Ok(Err(e)) => tracing::warn!("metrics: textfile write failed: {e}"),
            Err(e) => tracing::warn!("metrics: write task panicked: {e}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::system::{SysMetrics, TempEntry};

    #[test]
    fn counters_render_with_help_and_type() {
        let m = Metrics::default();
        m.inc_input_events();
        m.inc_input_events();
        m.inc_intents();
        m.inc_transitions();
        m.inc_pad_joins();
        m.inc_shell_restarts();
        let text = render(&m, None, None);

        // Each counter has HELP + TYPE + a sample line.
        assert!(text.contains("# HELP game_shell_input_events_total"));
        assert!(text.contains("# TYPE game_shell_input_events_total counter"));
        assert!(text.contains("\ngame_shell_input_events_total 2\n"));
        assert!(text.contains("\ngame_shell_intents_emitted_total 1\n"));
        assert!(text.contains("\ngame_shell_transitions_total 1\n"));
        assert!(text.contains("\ngame_shell_pad_joins_total 1\n"));
        assert!(text.contains("\ngame_shell_pad_leaves_total 0\n"));
        assert!(text.contains("\ngame_shell_shell_restarts_total 1\n"));
        // Trailing newline (textfile collector requirement).
        assert!(text.ends_with('\n'));
        // No gauges when sys is None; no build_info when build is None.
        assert!(!text.contains("game_shell_cpu_percent"));
        assert!(!text.contains("game_shell_build_info"));
    }

    #[test]
    fn runtime_supervision_metrics_render() {
        let m = Metrics::default();
        // Fresh: the up-gauge defaults to 0 and both new counters are 0, but all
        // three are ALWAYS present (no Some(sys)/Some(build) gating).
        let text0 = render(&m, None, None);
        assert!(text0.contains("# TYPE game_shell_input_runtime_up gauge"));
        assert!(text0.contains("\ngame_shell_input_runtime_up 0\n"));
        assert!(text0.contains("# TYPE game_shell_input_runtime_restarts_total counter"));
        assert!(text0.contains("\ngame_shell_input_runtime_restarts_total 0\n"));
        assert!(text0.contains("# TYPE game_shell_grab_invariant_violations_total counter"));
        assert!(text0.contains("\ngame_shell_grab_invariant_violations_total 0\n"));

        m.set_runtime_up(true);
        m.inc_runtime_restarts();
        m.inc_runtime_restarts();
        m.inc_grab_invariant_violations();
        let text = render(&m, None, None);
        assert!(text.contains("\ngame_shell_input_runtime_up 1\n"));
        assert!(text.contains("\ngame_shell_input_runtime_restarts_total 2\n"));
        assert!(text.contains("\ngame_shell_grab_invariant_violations_total 1\n"));

        // The gauge tracks liveness both directions.
        m.set_runtime_up(false);
        assert!(render(&m, None, None).contains("\ngame_shell_input_runtime_up 0\n"));
    }

    #[test]
    fn dev_action_counters_render() {
        let m = Metrics::default();
        m.inc_deploy(true);
        m.inc_deploy(true);
        m.inc_deploy(false);
        m.inc_build();
        m.inc_restart_shell();
        m.inc_restart_daemon();
        m.inc_quickshell_multi_instance();
        let text = render(&m, None, None);

        assert!(text.contains("# TYPE game_shell_deploy_total counter"));
        assert!(text.contains("game_shell_deploy_total{outcome=\"ok\"} 2\n"));
        assert!(text.contains("game_shell_deploy_total{outcome=\"error\"} 1\n"));
        assert!(text.contains("\ngame_shell_build_total 1\n"));
        assert!(text.contains("\ngame_shell_restart_shell_total 1\n"));
        assert!(text.contains("\ngame_shell_restart_daemon_total 1\n"));
        assert!(text.contains("\ngame_shell_quickshell_multi_instance_total 1\n"));
    }

    #[test]
    fn build_info_renders_value_1_with_labels() {
        let m = Metrics::default();
        let build = BuildInfo {
            sha: "a1b2c3d".into(),
            branch: "feat/daemon-observability".into(),
            version: "0.1.0".into(),
        };
        let text = render(&m, None, Some(&build));
        assert!(text.contains("# TYPE game_shell_build_info gauge"));
        assert!(text.contains(
            "game_shell_build_info{sha=\"a1b2c3d\",branch=\"feat/daemon-observability\",version=\"0.1.0\"} 1\n"
        ));
    }

    #[test]
    fn gauges_render_when_sys_present() {
        let m = Metrics::default();
        let sys = SysMetrics {
            cpu_pct: 12.5,
            mem_used: 1024,
            mem_total: 4096,
            mem_pct: 25,
            load1: 0.42,
            temps: vec![
                TempEntry {
                    label: "CPU Tctl".into(),
                    celsius: 55.0,
                },
                TempEntry {
                    label: "GPU edge".into(),
                    celsius: 48.5,
                },
            ],
        };
        let text = render(&m, Some(&sys), None);

        assert!(text.contains("# TYPE game_shell_cpu_percent gauge"));
        assert!(text.contains("\ngame_shell_cpu_percent 12.5\n"));
        assert!(text.contains("\ngame_shell_mem_used_bytes 1024\n"));
        assert!(text.contains("\ngame_shell_mem_total_bytes 4096\n"));
        assert!(text.contains("\ngame_shell_load1 0.42\n"));
        assert!(text.contains("# TYPE game_shell_temperature_celsius gauge"));
        assert!(text.contains("game_shell_temperature_celsius{sensor=\"CPU Tctl\"} 55\n"));
        assert!(text.contains("game_shell_temperature_celsius{sensor=\"GPU edge\"} 48.5\n"));
    }

    #[test]
    fn label_value_escaping() {
        assert_eq!(escape_label_value("plain"), "plain");
        assert_eq!(escape_label_value("a\"b"), "a\\\"b");
        assert_eq!(escape_label_value("a\\b"), "a\\\\b");
        assert_eq!(escape_label_value("a\nb"), "a\\nb");
    }

    #[test]
    fn non_finite_gauge_is_zero() {
        assert_eq!(fmt_f64(f64::NAN), "0");
        assert_eq!(fmt_f64(f64::INFINITY), "0");
        assert_eq!(fmt_f64(3.0), "3");
    }

    #[test]
    fn write_atomic_creates_file_with_exact_contents() {
        let dir = std::env::temp_dir().join(format!("gs-metrics-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("game-shell.prom");
        let body = "# HELP x test\n# TYPE x counter\nx 1\n";
        write_atomic(&path, body).expect("atomic write succeeds");
        let read_back = std::fs::read_to_string(&path).unwrap();
        assert_eq!(read_back, body);
        // No stray temp files left behind in the dir.
        let leftovers: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.ends_with(".tmp"))
            .collect();
        assert!(leftovers.is_empty(), "temp files left: {leftovers:?}");
        std::fs::remove_dir_all(&dir).ok();
    }

    // The interval default/clamp now lives in
    // `DaemonConfig::metrics_interval_secs()` and is tested in daemon_config.rs.
}
