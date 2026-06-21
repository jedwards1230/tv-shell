//! Stateless network reads for the QML shell: per-interface throughput counters
//! and a bounded connectivity/latency ping.
//!
//! These replace two QML shell-outs that the daemon should own (the daemon owns
//! all *reads* of system state; QML keeps only write/action shell-outs):
//!   - `net-throughput <iface>` replaces NetworkOverlay's
//!     `paste /sys/class/net/<iface>/statistics/{rx,tx}_bytes` one-liner.
//!   - `net-ping <host> [count]` replaces the `ping` shell-outs in
//!     NetworkSettings (test-connection) and StreamCard (target reachability).
//!
//! Both are stateless and served directly from the IPC dispatcher (like
//! `sunshine-status`/`wol`), NOT through the NetworkManager D-Bus actor — the
//! throughput read is pure sysfs and the ping is a subprocess, neither needs NM.
//!
//! Cross-platform: the parse helpers are pure and unit-tested on every host. The
//! sysfs read is Linux-only (degrades to an `error` field off-Linux); the ping
//! subprocess runs anywhere `ping` exists (the daemon ships on Linux, where the
//! `-c`/`-W` flags below are correct).

use serde_json::json;

/// Cap on the ping count so a caller can't ask for an unbounded run.
const MAX_PING_COUNT: u32 = 10;

/// Per-packet ping timeout in seconds (Linux `ping -W`).
const PING_WAIT_SECS: u32 = 2;

/// Validate an interface name before it touches a `/sys` path. Rejects empty
/// names and anything with a path separator or `..` so a crafted `iface` can't
/// traverse out of `/sys/class/net`. Linux iface names are short and limited to
/// a conservative charset; we allow alphanumerics plus the few punctuation marks
/// real names use (`.`, `_`, `-`, `:`, `@`).
fn valid_iface(iface: &str) -> bool {
    !iface.is_empty()
        && iface.len() <= 64
        && iface
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | ':' | '@'))
}

/// Build the `{iface,rxBytes,txBytes}` success body. Only the Linux sysfs path
/// builds a success body; off-Linux the lib never calls this (the tests do).
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn throughput_json(iface: &str, rx: u64, tx: u64) -> String {
    json!({ "iface": iface, "rxBytes": rx, "txBytes": tx }).to_string()
}

/// Build the throughput error body — same shape with zeroed counters plus an
/// `error` field, so QML can render "—" without special-casing a protocol error.
fn throughput_error(iface: &str, reason: &str) -> String {
    json!({ "iface": iface, "rxBytes": 0, "txBytes": 0, "error": reason }).to_string()
}

/// `net-throughput <iface>` handler: read the interface's cumulative rx/tx byte
/// counters from sysfs. Raw counters (not a rate) — the caller computes the
/// delta over its own sampling interval. Runs the (small) blocking sysfs reads
/// on the blocking pool so the IPC reactor isn't stalled.
pub async fn handle_net_throughput(iface: String) -> String {
    if !valid_iface(&iface) {
        return throughput_error(&iface, "invalid interface name");
    }
    tokio::task::spawn_blocking(move || read_throughput(&iface))
        .await
        .unwrap_or_else(|_| throughput_error("", "throughput task failed"))
}

/// Blocking sysfs read of an interface's rx/tx byte counters.
#[cfg(target_os = "linux")]
fn read_throughput(iface: &str) -> String {
    let base = format!("/sys/class/net/{iface}/statistics");
    let read_counter = |which: &str| -> Option<u64> {
        std::fs::read_to_string(format!("{base}/{which}"))
            .ok()
            .and_then(|s| s.trim().parse::<u64>().ok())
    };
    match (read_counter("rx_bytes"), read_counter("tx_bytes")) {
        (Some(rx), Some(tx)) => throughput_json(iface, rx, tx),
        _ => throughput_error(iface, "interface not found"),
    }
}

/// Off-Linux there is no `/sys/class/net`; degrade to the error body.
#[cfg(not(target_os = "linux"))]
fn read_throughput(iface: &str) -> String {
    throughput_error(iface, "unsupported on this platform")
}

/// Parse the average RTT (milliseconds) out of a `ping` summary. Handles both
/// the Linux form (`rtt min/avg/max/mdev = 0.1/0.2/0.3/0.0 ms`) and the macOS
/// form (`round-trip min/avg/max/stddev = 0.1/0.2/0.3/0.0 ms`). Returns `None`
/// if no summary line is present (e.g. 100% packet loss).
fn parse_avg_rtt(output: &str) -> Option<f64> {
    for line in output.lines() {
        let line = line.trim();
        let is_summary = line.starts_with("rtt ") || line.starts_with("round-trip ");
        if !is_summary {
            continue;
        }
        // Take the "a/b/c/d" group after '=', then its 2nd field (avg).
        let stats = line.split('=').nth(1)?.trim();
        let avg = stats.split('/').nth(1)?.trim();
        // The first field may carry a trailing unit on some platforms; split it.
        if let Ok(v) = avg.split_whitespace().next()?.parse::<f64>() {
            return Some(v);
        }
    }
    None
}

/// Build the `{host,reachable,rttMs}` body. `rttMs` is JSON `null` when the host
/// is unreachable (or no RTT could be parsed).
fn ping_json(host: &str, reachable: bool, rtt_ms: Option<f64>) -> String {
    json!({ "host": host, "reachable": reachable, "rttMs": rtt_ms }).to_string()
}

/// `net-ping <host> [count]` handler: run a bounded `ping` and report
/// reachability + average RTT. Fail-soft — an unreachable host, a missing `ping`
/// binary, or a spawn failure all degrade to `reachable:false`, never a protocol
/// error. `host` is passed as a single argv (no shell), so it can't inject.
pub async fn handle_net_ping(host: String, count: u32) -> String {
    let count = count.clamp(1, MAX_PING_COUNT);
    let output = tokio::process::Command::new("ping")
        .args([
            "-c",
            &count.to_string(),
            "-W",
            &PING_WAIT_SECS.to_string(),
            &host,
        ])
        .output()
        .await;

    match output {
        Ok(o) if o.status.success() => {
            let text = String::from_utf8_lossy(&o.stdout);
            ping_json(&host, true, parse_avg_rtt(&text))
        }
        // Non-zero exit = unreachable / packet loss / unknown host.
        Ok(_) => ping_json(&host, false, None),
        // `ping` missing or un-spawnable — treat as unreachable, don't error.
        Err(_) => ping_json(&host, false, None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_iface_accepts_real_names_rejects_traversal() {
        assert!(valid_iface("eth0"));
        assert!(valid_iface("wlan0"));
        assert!(valid_iface("enp3s0"));
        assert!(valid_iface("br-lan"));
        assert!(valid_iface("eth0.100")); // VLAN
        assert!(!valid_iface(""));
        assert!(!valid_iface("../../etc/passwd"));
        assert!(!valid_iface("eth0/statistics"));
        assert!(!valid_iface("a b"));
    }

    #[test]
    fn throughput_bodies_are_compact_single_line() {
        let ok = throughput_json("eth0", 100, 200);
        assert_eq!(ok, r#"{"iface":"eth0","rxBytes":100,"txBytes":200}"#);
        let err = throughput_error("eth0", "interface not found");
        assert!(err.contains(r#""error":"interface not found""#));
        assert!(err.contains(r#""rxBytes":0"#));
        assert!(!ok.contains('\n'));
    }

    #[test]
    fn invalid_iface_handler_degrades_to_error_body() {
        let body = tokio_test_block_on(handle_net_throughput("../escape".to_string()));
        assert!(body.contains(r#""error":"invalid interface name""#));
    }

    #[test]
    fn parse_avg_rtt_linux_form() {
        let out = "PING 1.1.1.1 (1.1.1.1) 56(84) bytes of data.\n\
                   --- 1.1.1.1 ping statistics ---\n\
                   3 packets transmitted, 3 received, 0% packet loss, time 2003ms\n\
                   rtt min/avg/max/mdev = 12.345/14.567/16.789/1.234 ms";
        assert_eq!(parse_avg_rtt(out), Some(14.567));
    }

    #[test]
    fn parse_avg_rtt_macos_form() {
        let out = "round-trip min/avg/max/stddev = 10.1/20.2/30.3/4.4 ms";
        assert_eq!(parse_avg_rtt(out), Some(20.2));
    }

    #[test]
    fn parse_avg_rtt_none_on_total_loss() {
        let out = "3 packets transmitted, 0 received, 100% packet loss, time 2040ms";
        assert_eq!(parse_avg_rtt(out), None);
    }

    #[test]
    fn ping_json_null_rtt_when_unreachable() {
        let body = ping_json("1.1.1.1", false, None);
        assert_eq!(body, r#"{"host":"1.1.1.1","reachable":false,"rttMs":null}"#);
        let ok = ping_json("1.1.1.1", true, Some(14.5));
        assert_eq!(ok, r#"{"host":"1.1.1.1","reachable":true,"rttMs":14.5}"#);
    }

    /// Minimal block-on for the one async handler test (no extra dev-dep).
    fn tokio_test_block_on<F: std::future::Future>(f: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(f)
    }
}
