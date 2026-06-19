//! Wake-on-LAN: send a magic packet to a streaming host.
//!
//! The home-screen Steam row replaces its posters with a single "Wake <host>"
//! card when the streaming host is unreachable; activating it sends `wol <host>`
//! over the IPC socket, which lands here. We then fire a standard Wake-on-LAN
//! magic packet (6×0xFF + 16× the host's MAC) as a UDP broadcast on port 9.
//!
//! The hard part is resolving the host's MAC *while it is asleep*: the kernel
//! ARP/neighbor entry goes STALE and may be evicted entirely once the host stops
//! answering. So we resolve opportunistically — `ip neigh show` every call (the
//! host is normally online, and thus present in the neighbor table, shortly
//! before it goes to sleep, keeping the cache warm) and persist the learned
//! `host → MAC` mapping to a sibling of `settings.json`
//! (`~/.config/game-shell/host-macs.json`, NOT inside the user-authored config).
//! On a lookup miss we fall back to the cached MAC, so a wake works even from a
//! cold neighbor table.
//!
//! Cross-platform: the magic-packet build + the `ip neigh` parse are pure
//! functions unit-tested on every platform; only the live `ip neigh` shell-out
//! and the UDP send touch the system (and degrade gracefully off-Linux / when
//! `ip` is absent).

use serde_json::json;
use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, ToSocketAddrs, UdpSocket};
use std::path::PathBuf;

/// A parsed Ethernet MAC address (6 octets).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Mac(pub [u8; 6]);

impl Mac {
    /// Parse a colon- or dash-separated MAC (`aa:bb:cc:dd:ee:ff`). Returns `None`
    /// for the wrong octet count or a non-hex octet. Case-insensitive.
    pub fn parse(s: &str) -> Option<Mac> {
        let mut octets = [0u8; 6];
        let mut count = 0;
        for part in s.split([':', '-']) {
            if count >= 6 {
                return None; // too many octets
            }
            octets[count] = u8::from_str_radix(part.trim(), 16).ok()?;
            count += 1;
        }
        if count == 6 {
            Some(Mac(octets))
        } else {
            None
        }
    }

    /// Canonical lowercase colon-separated rendering (`aa:bb:cc:dd:ee:ff`).
    pub fn to_canonical(self) -> String {
        let b = self.0;
        format!(
            "{:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
            b[0], b[1], b[2], b[3], b[4], b[5]
        )
    }
}

/// Build a standard Wake-on-LAN magic packet: 6 bytes of 0xFF followed by the
/// target MAC repeated 16 times — 102 bytes total. Pure; unit-tested.
pub fn magic_packet(mac: Mac) -> Vec<u8> {
    let mut packet = Vec::with_capacity(102);
    packet.extend_from_slice(&[0xFF; 6]);
    for _ in 0..16 {
        packet.extend_from_slice(&mac.0);
    }
    packet
}

/// Parse `ip neigh show` output, returning a `host-ip → MAC` map for every line
/// that carries a resolvable `lladdr`. Lines without an `lladdr` (FAILED /
/// INCOMPLETE entries) are skipped. Pure; unit-tested.
///
/// A neighbor line looks like:
/// `192.168.8.10 dev eth0 lladdr aa:bb:cc:dd:ee:ff REACHABLE`
/// (or `... STALE`, `... DELAY`, etc.). The first token is the peer IP.
pub fn parse_ip_neigh(output: &str) -> HashMap<String, Mac> {
    let mut map = HashMap::new();
    for line in output.lines() {
        let mut toks = line.split_whitespace();
        let Some(ip) = toks.next() else {
            continue;
        };
        // Scan the remaining tokens for `lladdr <mac>`.
        let mut mac = None;
        let rest: Vec<&str> = toks.collect();
        for w in rest.windows(2) {
            if w[0] == "lladdr" {
                mac = Mac::parse(w[1]);
                break;
            }
        }
        if let Some(mac) = mac {
            map.insert(ip.to_string(), mac);
        }
    }
    map
}

/// Path to the learned `host → MAC` cache: a sibling of `settings.json`
/// (`~/.config/game-shell/host-macs.json`), NOT inside the user-authored config
/// so we never clobber hand-edited settings.
fn mac_cache_path() -> PathBuf {
    let mut p = crate::config::settings_path();
    p.set_file_name("host-macs.json");
    p
}

/// Load the persisted `host → MAC-string` cache. A missing/corrupt file is an
/// empty map (best-effort).
fn load_cache() -> HashMap<String, String> {
    let path = mac_cache_path();
    match std::fs::read_to_string(&path) {
        Ok(text) => serde_json::from_str(&text).unwrap_or_default(),
        Err(_) => HashMap::new(),
    }
}

/// Persist the `host → MAC-string` cache (best-effort; errors are logged, not
/// fatal — a failed cache write just means the next wake re-learns from `ip
/// neigh`).
fn save_cache(cache: &HashMap<String, String>) {
    let path = mac_cache_path();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    match serde_json::to_string(cache) {
        Ok(text) => {
            if let Err(e) = std::fs::write(&path, text) {
                tracing::debug!("wol: failed to write MAC cache {}: {e}", path.display());
            }
        }
        Err(e) => tracing::debug!("wol: failed to serialize MAC cache: {e}"),
    }
}

/// Resolve `host` to an IPv4 string. If `host` already parses as an IP it's
/// returned verbatim; otherwise the std resolver is consulted and the first IPv4
/// is taken. Returns `None` when the host can't be resolved to an IPv4.
fn resolve_ipv4(host: &str) -> Option<String> {
    if let Ok(ip) = host.parse::<IpAddr>() {
        return match ip {
            IpAddr::V4(_) => Some(host.to_string()),
            // An IPv6 literal can't be matched against the IPv4 neighbor table.
            IpAddr::V6(_) => None,
        };
    }
    // Hostname: resolve via the std resolver. Append a dummy port so
    // `to_socket_addrs` works, then take the first IPv4.
    let addrs = (host, 0u16).to_socket_addrs().ok()?;
    for addr in addrs {
        if let IpAddr::V4(v4) = addr.ip() {
            return Some(v4.to_string());
        }
    }
    None
}

/// Run `ip neigh show` and return its stdout, or `None` if the command is
/// unavailable / fails. Isolated so the rest of the resolution logic stays pure.
fn ip_neigh_output() -> Option<String> {
    let out = std::process::Command::new("ip")
        .args(["neigh", "show"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    String::from_utf8(out.stdout).ok()
}

/// Resolve `host` → MAC: first via the live neighbor table (updating the cache on
/// a hit), falling back to the persisted cache on a miss. `host` is the original
/// IP/hostname the shell passed; `ipv4` is its resolved IPv4 string (the key used
/// to match the neighbor table). Returns `None` when neither source has a MAC.
fn resolve_mac(host: &str, ipv4: &str) -> Option<Mac> {
    // Live lookup: the host is normally online (and thus in the neighbor table)
    // shortly before it goes to sleep, so this keeps the cache warm for the wake
    // that happens *after* it has gone to sleep.
    if let Some(output) = ip_neigh_output() {
        let table = parse_ip_neigh(&output);
        if let Some(mac) = table.get(ipv4).copied() {
            // Warm the cache under both the resolved IPv4 and the original host
            // string, so a later wake keyed by either resolves.
            let mut cache = load_cache();
            cache.insert(ipv4.to_string(), mac.to_canonical());
            cache.insert(host.to_string(), mac.to_canonical());
            save_cache(&cache);
            return Some(mac);
        }
    }
    // Miss: fall back to the cached MAC (host may already be asleep / evicted).
    let cache = load_cache();
    cache
        .get(ipv4)
        .or_else(|| cache.get(host))
        .and_then(|s| Mac::parse(s))
}

/// Send a magic packet for `mac` as a UDP broadcast on port 9. Broadcasts to the
/// global broadcast address `255.255.255.255` (and is best-effort — the OS routes
/// it onto the LAN). Returns `Ok(())` on a successful send.
fn send_magic_packet(mac: Mac) -> std::io::Result<()> {
    let packet = magic_packet(mac);
    // Bind to any local IPv4 address/port for the outbound broadcast.
    let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0))?;
    socket.set_broadcast(true)?;
    let dest = SocketAddr::from((Ipv4Addr::BROADCAST, 9));
    socket.send_to(&packet, dest)?;
    Ok(())
}

/// JSON success reply for a sent wake.
fn ok_json(mac: Mac) -> String {
    json!({"status": "ok", "mac": mac.to_canonical()}).to_string()
}

/// JSON error reply with a short machine-readable `reason`.
fn err_json(reason: &str) -> String {
    json!({"status": "error", "reason": reason}).to_string()
}

/// IPC entry point for `wol <host>`. Resolves the host's MAC (neighbor table →
/// cache), then fires the magic packet. Returns a compact-JSON reply:
/// `{"status":"ok","mac":"…"}` on success, or `{"status":"error","reason":"…"}`
/// (`no-host`, `no-ip`, `no-mac`, or `send-failed`) on failure.
pub async fn handle_wol(host: &str) -> String {
    // Defensive: an empty host shouldn't reach here (the parser routes those to
    // `WolUsage`), but guard anyway.
    if host.is_empty() {
        return err_json("no-host");
    }
    // The resolution + UDP send are blocking syscalls; run them off the reactor.
    let host = host.to_string();
    let result = tokio::task::spawn_blocking(move || {
        let Some(ipv4) = resolve_ipv4(&host) else {
            return err_json("no-ip");
        };
        let Some(mac) = resolve_mac(&host, &ipv4) else {
            return err_json("no-mac");
        };
        match send_magic_packet(mac) {
            Ok(()) => ok_json(mac),
            Err(e) => {
                tracing::debug!("wol: send failed for {host}: {e}");
                err_json("send-failed")
            }
        }
    })
    .await;
    result.unwrap_or_else(|e| {
        tracing::debug!("wol: join error: {e}");
        err_json("send-failed")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_mac_colon_and_dash() {
        assert_eq!(
            Mac::parse("aa:bb:cc:dd:ee:ff"),
            Some(Mac([0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff]))
        );
        assert_eq!(
            Mac::parse("AA-BB-CC-DD-EE-FF"),
            Some(Mac([0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff]))
        );
        // Mixed case + leading zeros.
        assert_eq!(
            Mac::parse("00:1A:2b:3C:4d:5E"),
            Some(Mac([0x00, 0x1a, 0x2b, 0x3c, 0x4d, 0x5e]))
        );
    }

    #[test]
    fn rejects_malformed_mac() {
        assert_eq!(Mac::parse(""), None);
        assert_eq!(Mac::parse("aa:bb:cc:dd:ee"), None); // too few
        assert_eq!(Mac::parse("aa:bb:cc:dd:ee:ff:00"), None); // too many
        assert_eq!(Mac::parse("zz:bb:cc:dd:ee:ff"), None); // non-hex
    }

    #[test]
    fn mac_to_canonical_is_lowercase_colon() {
        assert_eq!(
            Mac([0xaa, 0xbb, 0xcc, 0x00, 0x0e, 0xff]).to_canonical(),
            "aa:bb:cc:00:0e:ff"
        );
    }

    #[test]
    fn magic_packet_is_102_bytes_with_header_and_repetitions() {
        let mac = Mac([0x01, 0x02, 0x03, 0x04, 0x05, 0x06]);
        let packet = magic_packet(mac);
        assert_eq!(packet.len(), 102);
        // First 6 bytes are the 0xFF sync header.
        assert_eq!(&packet[0..6], &[0xFF; 6]);
        // Followed by 16 copies of the MAC.
        for i in 0..16 {
            let start = 6 + i * 6;
            assert_eq!(&packet[start..start + 6], &mac.0, "repetition {i}");
        }
    }

    #[test]
    fn parses_ip_neigh_matching_host_to_mac() {
        let output = "\
192.168.8.10 dev eth0 lladdr aa:bb:cc:dd:ee:ff REACHABLE
192.168.8.20 dev eth0 lladdr 11:22:33:44:55:66 STALE
192.168.8.30 dev eth0  FAILED
192.168.8.40 dev eth0 lladdr 77:88:99:aa:bb:cc DELAY
";
        let table = parse_ip_neigh(output);
        assert_eq!(
            table.get("192.168.8.10").copied(),
            Some(Mac([0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff]))
        );
        // STALE entries still carry a usable lladdr.
        assert_eq!(
            table.get("192.168.8.20").copied(),
            Some(Mac([0x11, 0x22, 0x33, 0x44, 0x55, 0x66]))
        );
        // FAILED entry (no lladdr) is skipped.
        assert!(!table.contains_key("192.168.8.30"));
        // DELAY entry resolves.
        assert_eq!(
            table.get("192.168.8.40").copied(),
            Some(Mac([0x77, 0x88, 0x99, 0xaa, 0xbb, 0xcc]))
        );
    }

    #[test]
    fn parses_ip_neigh_empty_and_garbage() {
        assert!(parse_ip_neigh("").is_empty());
        assert!(parse_ip_neigh("garbage line with no lladdr").is_empty());
    }

    #[test]
    fn resolve_ipv4_passes_through_literal_v4() {
        assert_eq!(resolve_ipv4("192.0.2.1").as_deref(), Some("192.0.2.1"));
    }

    #[test]
    fn resolve_ipv4_rejects_v6_literal() {
        assert_eq!(resolve_ipv4("::1"), None);
    }
}
