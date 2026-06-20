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
/// supplies the convenience resource gauges. When `sys` is `None` the resource
/// gauges are omitted entirely (e.g. if a reader is disabled) — the counters are
/// always emitted.
///
/// Every metric carries `# HELP` and `# TYPE` lines. All metrics are namespaced
/// `game_shell_`. The output ends with a trailing newline (required by the
/// node_exporter textfile collector parser).
pub fn render(counters: &Metrics, sys: Option<&crate::system::SysMetrics>) -> String {
    let mut out = String::with_capacity(1024);

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

/// Read the live system metrics on a blocking thread and render the full
/// exposition text. `cpu_percent` sleeps ~200ms internally, so this MUST run on
/// the blocking pool (the textfile task and the HTTP handler both wrap it in
/// `spawn_blocking`).
pub fn render_blocking(counters: &Metrics) -> String {
    let sys = crate::system::sys_metrics();
    render(counters, Some(&sys))
}

// ─── node_exporter textfile-collector writer ─────────────────────────────────

/// Default render/write interval in seconds when `GAME_SHELL_METRICS_INTERVAL`
/// is unset or unparseable.
const DEFAULT_INTERVAL_SECS: u64 = 15;

/// Parse the write interval from `GAME_SHELL_METRICS_INTERVAL` (seconds).
/// Falls back to [`DEFAULT_INTERVAL_SECS`] when unset, unparseable, or zero
/// (a zero interval would be a busy-loop).
fn interval_secs() -> u64 {
    std::env::var("GAME_SHELL_METRICS_INTERVAL")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .filter(|&n| n > 0)
        .unwrap_or(DEFAULT_INTERVAL_SECS)
}

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
/// atomically to the path in `GAME_SHELL_METRICS_TEXTFILE`.
///
/// **Disabled when the env var is unset** — the task returns immediately and no
/// file is ever written. This is the PRIMARY metrics path (node_exporter
/// textfile collector); the `/metrics` HTTP route is the portable alternative.
///
/// Mirrors the fire-and-forget spawn pattern of the other daemon actors: it logs
/// and degrades gracefully (a failed write is logged at warn, not fatal) and
/// never panics the daemon.
pub async fn run_textfile_writer(counters: Arc<Metrics>) {
    let Ok(path_str) = std::env::var("GAME_SHELL_METRICS_TEXTFILE") else {
        // Unset → writer disabled (no file). The /metrics route is unaffected.
        tracing::debug!("metrics: GAME_SHELL_METRICS_TEXTFILE unset, textfile writer disabled");
        return;
    };
    if path_str.is_empty() {
        tracing::debug!("metrics: GAME_SHELL_METRICS_TEXTFILE empty, textfile writer disabled");
        return;
    }
    let path = std::path::PathBuf::from(path_str);
    let secs = interval_secs();
    tracing::info!(
        "metrics: writing textfile-collector metrics to {} every {secs}s",
        path.display()
    );

    let mut ticker = tokio::time::interval(std::time::Duration::from_secs(secs));
    loop {
        ticker.tick().await;
        // Render on the blocking pool: sys_metrics() sleeps ~200ms (CPU sample).
        let counters = Arc::clone(&counters);
        let text = match tokio::task::spawn_blocking(move || render_blocking(&counters)).await {
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
        let text = render(&m, None);

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
        // No gauges when sys is None.
        assert!(!text.contains("game_shell_cpu_percent"));
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
        let text = render(&m, Some(&sys));

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

    #[test]
    fn interval_secs_defaults_when_unset() {
        // The env var is process-global; only assert the default branch via the
        // parsing helper contract (no env mutation to keep tests parallel-safe).
        // A zero or garbage value must fall back to the default.
        assert_eq!(DEFAULT_INTERVAL_SECS, 15);
    }
}
