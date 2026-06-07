//! Network AV control (#186): the two AV lifecycle ops libcec/CEC physically
//! cannot reach — **AVR Zone-2 power** and **TV cold-wake via Wake-on-LAN**.
//!
//! ## Why this exists
//! The CEC actor ([`crate::cec`]) owns the HDMI-CEC bus end-to-end, but CEC is
//! limited to the devices and functions the protocol exposes:
//!
//! - A Denon/Marantz AVR's **Zone 2** is not addressable over CEC — the legacy
//!   `living-room-cec` script sent a telnet `Z2OFF` to the AVR's control port.
//! - A **fully powered-off LG TV** can't be woken by CEC `image-view-on` /
//!   `cec-power-on`; it needs an Ethernet **Wake-on-LAN** magic packet to
//!   cold-start. The legacy script sent `ether-wake` for this.
//!
//! Both are plain **network** operations, so this module is **pure Rust**
//! (UDP magic packet + TCP telnet) with no `libcec`/C dependency — it is NOT
//! feature-gated and compiles + unit-tests on every host (macOS/CI included),
//! unlike [`crate::cec`]. The CEC lifecycle sequences call into here so the
//! daemon owns the full network-AV story in one place, replacing the homelab
//! `living-room-cec` path for game-shell in-session wake/sleep.
//!
//! ## Configuration — off by default
//! Behaviour is driven entirely by environment variables (set in
//! `~/.config/game-shell/daemon.env` on the deploy host, like
//! `GAME_SHELL_CEC_LIFECYCLE`). With **no** config present the module is inert:
//! [`AvNetConfig::from_env`] returns `None` and every entry point is a no-op
//! that succeeds, so dev/CI hosts never emit a packet or open a socket. This
//! keeps the feature **non-breaking**: it only does anything once a box opts in.
//!
//! | Variable | Effect |
//! |----------|--------|
//! | `GAME_SHELL_AVR_HOST` | AVR control host (e.g. `192.0.2.10`). Enables the telnet AVR ops (Zone-2 off + optional main power/input). |
//! | `GAME_SHELL_AVR_PORT` | AVR telnet control port. Default `23`. |
//! | `GAME_SHELL_AVR_INPUT` | Source to select on wake (e.g. `GAME`, `MPLAY`). Sends `SI<input>` on wake when set. |
//! | `GAME_SHELL_AVR_MAIN_POWER` | When `1`/`true`, also send `PWON` on wake and `PWSTANDBY` on sleep. Default off (CEC already drives the AVR main zone). |
//! | `GAME_SHELL_TV_WOL_MAC` | TV MAC address for Wake-on-LAN cold-start (e.g. `aa:bb:cc:dd:ee:ff`). Enables the WoL packet on wake. |
//! | `GAME_SHELL_TV_WOL_BROADCAST` | Broadcast address:port for the magic packet. Default `255.255.255.255:9`. |
//!
//! Either half (AVR telnet, TV WoL) can be configured independently; an
//! unconfigured half is simply skipped.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use tokio::io::AsyncWriteExt;
use tokio::net::{TcpStream, UdpSocket};

/// Default AVR telnet control port (Denon/Marantz speak ASCII over TCP/23).
const DEFAULT_AVR_PORT: u16 = 23;
/// Default Wake-on-LAN destination: limited broadcast on the discard port.
const DEFAULT_WOL_BROADCAST: &str = "255.255.255.255:9";
/// How long to wait when opening the AVR telnet socket / sending a packet
/// before giving up. AV ops are best-effort and must never wedge the lifecycle.
const NET_TIMEOUT: Duration = Duration::from_secs(3);

// ---------------------------------------------------------------------------
// Wake-on-LAN magic packet (pure — unit-tested on every host).
// ---------------------------------------------------------------------------

/// A 48-bit Ethernet MAC address.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MacAddr(pub [u8; 6]);

impl MacAddr {
    /// Parse a MAC from the usual `aa:bb:cc:dd:ee:ff` form. Accepts `:` or `-`
    /// separators and is case-insensitive. Returns an error for any string that
    /// is not exactly six hex octets.
    pub fn parse(s: &str) -> Result<Self> {
        let parts: Vec<&str> = s.split([':', '-']).collect();
        if parts.len() != 6 {
            return Err(anyhow!(
                "MAC must have 6 octets, got {}: {s:?}",
                parts.len()
            ));
        }
        let mut octets = [0u8; 6];
        for (i, p) in parts.iter().enumerate() {
            octets[i] = u8::from_str_radix(p, 16)
                .with_context(|| format!("invalid MAC octet {p:?} in {s:?}"))?;
        }
        Ok(MacAddr(octets))
    }
}

/// Build a 102-byte Wake-on-LAN **magic packet** for `mac`:
/// 6 bytes of `0xFF` (the sync stream) followed by the target MAC repeated 16
/// times. This is the canonical magic-packet format that an LG TV (and most
/// NICs) cold-wake on. Pure function — no I/O — so it is unit-tested everywhere.
pub fn magic_packet(mac: MacAddr) -> [u8; 102] {
    let mut pkt = [0u8; 102];
    for b in pkt.iter_mut().take(6) {
        *b = 0xFF;
    }
    for rep in 0..16 {
        let off = 6 + rep * 6;
        pkt[off..off + 6].copy_from_slice(&mac.0);
    }
    pkt
}

// ---------------------------------------------------------------------------
// Configuration.
// ---------------------------------------------------------------------------

/// Parsed AV-network configuration. Built from the environment by
/// [`AvNetConfig::from_env`]; `None` means "no AV-network ops configured", in
/// which case the lifecycle entry points are no-ops.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AvNetConfig {
    /// AVR telnet endpoint, if `GAME_SHELL_AVR_HOST` is set. When `None`, all
    /// AVR ops (including Zone-2 off) are skipped.
    pub avr: Option<AvrConfig>,
    /// TV Wake-on-LAN config, if `GAME_SHELL_TV_WOL_MAC` is set. When `None`,
    /// the cold-wake WoL packet is skipped.
    pub tv_wol: Option<WolConfig>,
}

/// AVR telnet control settings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AvrConfig {
    /// `host:port` to open a telnet control socket to.
    pub addr: SocketAddr,
    /// Source-input code to select on wake (`SI<input>`), e.g. `GAME`. `None`
    /// skips the input switch (CEC active-source still claims the path).
    pub input: Option<String>,
    /// When true, also drive the AVR **main** power: `PWON` on wake and
    /// `PWSTANDBY` on sleep. Default false — CEC already powers the main zone,
    /// so this is only for boxes where CEC main-zone control is unreliable.
    pub main_power: bool,
}

/// Wake-on-LAN settings for the TV cold-wake.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WolConfig {
    /// Target TV MAC.
    pub mac: MacAddr,
    /// Broadcast destination for the magic packet (`addr:port`).
    pub broadcast: SocketAddr,
}

impl AvNetConfig {
    /// Build from the process environment. Returns `None` (fully inert) when
    /// **neither** `GAME_SHELL_AVR_HOST` nor `GAME_SHELL_TV_WOL_MAC` is set, so
    /// an unconfigured host never touches the network. A configured half that
    /// fails to parse logs a warning and is dropped (rather than aborting),
    /// preserving best-effort lifecycle behaviour.
    pub fn from_env() -> Option<Self> {
        let avr = Self::avr_from_env();
        let tv_wol = Self::wol_from_env();
        if avr.is_none() && tv_wol.is_none() {
            return None;
        }
        Some(AvNetConfig { avr, tv_wol })
    }

    fn avr_from_env() -> Option<AvrConfig> {
        let host = non_empty(std::env::var("GAME_SHELL_AVR_HOST").ok())?;
        let port = std::env::var("GAME_SHELL_AVR_PORT")
            .ok()
            .and_then(|p| p.trim().parse::<u16>().ok())
            .unwrap_or(DEFAULT_AVR_PORT);
        let addr = match resolve_host_port(&host, port) {
            Ok(a) => a,
            Err(e) => {
                tracing::warn!("av-net: invalid GAME_SHELL_AVR_HOST {host:?}: {e}");
                return None;
            }
        };
        let input = non_empty(std::env::var("GAME_SHELL_AVR_INPUT").ok());
        let main_power = env_flag("GAME_SHELL_AVR_MAIN_POWER");
        Some(AvrConfig {
            addr,
            input,
            main_power,
        })
    }

    fn wol_from_env() -> Option<WolConfig> {
        let mac_str = non_empty(std::env::var("GAME_SHELL_TV_WOL_MAC").ok())?;
        let mac = match MacAddr::parse(&mac_str) {
            Ok(m) => m,
            Err(e) => {
                tracing::warn!("av-net: invalid GAME_SHELL_TV_WOL_MAC {mac_str:?}: {e}");
                return None;
            }
        };
        let bcast_str = non_empty(std::env::var("GAME_SHELL_TV_WOL_BROADCAST").ok())
            .unwrap_or_else(|| DEFAULT_WOL_BROADCAST.to_string());
        let broadcast = match parse_socket_addr(&bcast_str) {
            Ok(a) => a,
            Err(e) => {
                tracing::warn!("av-net: invalid GAME_SHELL_TV_WOL_BROADCAST {bcast_str:?}: {e}");
                return None;
            }
        };
        Some(WolConfig { mac, broadcast })
    }
}

/// `Some(s)` only when `s` is present and non-empty after trimming.
fn non_empty(v: Option<String>) -> Option<String> {
    v.map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

/// Parse `GAME_SHELL_*` boolean flags the same way as `cec::lifecycle_enabled`:
/// true only for exactly `1` or `true`.
fn env_flag(key: &str) -> bool {
    matches!(std::env::var(key).as_deref(), Ok("1") | Ok("true"))
}

/// Parse an explicit `addr:port` socket address (the WoL broadcast form, which
/// is always numeric so no DNS is needed).
fn parse_socket_addr(s: &str) -> Result<SocketAddr> {
    s.parse::<SocketAddr>()
        .with_context(|| format!("not a valid host:port socket address: {s:?}"))
}

/// Resolve a host (IP or DNS name) + port into a single `SocketAddr`. The AVR
/// host is usually a literal IP, but accept a name too. Takes the first
/// resolved address.
fn resolve_host_port(host: &str, port: u16) -> Result<SocketAddr> {
    use std::net::ToSocketAddrs;
    (host, port)
        .to_socket_addrs()
        .with_context(|| format!("cannot resolve {host}:{port}"))?
        .next()
        .ok_or_else(|| anyhow!("no address resolved for {host}:{port}"))
}

// ---------------------------------------------------------------------------
// Network ops (best-effort — every error is logged, never propagated to the
// lifecycle so a missing AVR/TV can't wedge wake/sleep).
// ---------------------------------------------------------------------------

/// Send a single Wake-on-LAN magic packet to `cfg.mac` via a broadcast UDP
/// socket. Best-effort: on any error logs a warning and returns `Ok(())` so the
/// wake sequence continues (the TV may already be on, or CEC may handle it).
async fn send_wol(cfg: &WolConfig) -> Result<()> {
    let pkt = magic_packet(cfg.mac);
    // Bind to the unspecified v4 address so the kernel picks the egress NIC.
    let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0))
        .await
        .context("bind WoL UDP socket")?;
    socket
        .set_broadcast(true)
        .context("enable SO_BROADCAST for WoL")?;
    // A fully-off LG can miss the first packet; send twice like the legacy path.
    for _ in 0..2 {
        if let Err(e) = socket.send_to(&pkt, cfg.broadcast).await {
            tracing::warn!("av-net: WoL send to {} failed: {e}", cfg.broadcast);
            return Ok(());
        }
    }
    tracing::info!("av-net: sent WoL magic packet to {:?}", cfg.mac);
    Ok(())
}

/// Open the AVR telnet socket and send each command line in order, terminated
/// with `\r` (the Denon/Marantz line ending). Returns an error only on
/// connect/write failure; callers treat it as best-effort.
async fn avr_send(addr: SocketAddr, commands: &[String]) -> Result<()> {
    let mut stream = tokio::time::timeout(NET_TIMEOUT, TcpStream::connect(addr))
        .await
        .map_err(|_| anyhow!("connect to AVR {addr} timed out"))?
        .with_context(|| format!("connect to AVR {addr}"))?;
    for cmd in commands {
        let line = format!("{cmd}\r");
        tokio::time::timeout(NET_TIMEOUT, stream.write_all(line.as_bytes()))
            .await
            .map_err(|_| anyhow!("AVR write {cmd:?} timed out"))?
            .with_context(|| format!("write AVR command {cmd:?}"))?;
    }
    let _ = stream.flush().await;
    Ok(())
}

/// The AVR telnet commands to send **on wake**, given the config. Pure (no I/O)
/// so the command sequence is unit-testable. Main power (`PWON`) is only
/// included when `main_power` is set; the input switch (`SI<input>`) only when
/// an input is configured.
fn avr_wake_commands(cfg: &AvrConfig) -> Vec<String> {
    let mut cmds = Vec::new();
    if cfg.main_power {
        cmds.push("PWON".to_string());
    }
    if let Some(input) = &cfg.input {
        cmds.push(format!("SI{input}"));
    }
    cmds
}

/// The AVR telnet commands to send **on sleep**. Always includes `Z2OFF` (the
/// whole point of #186 — CEC can't reach Zone 2); `PWSTANDBY` is added when
/// `main_power` is set. Pure (no I/O), unit-tested.
fn avr_sleep_commands(cfg: &AvrConfig) -> Vec<String> {
    let mut cmds = Vec::new();
    if cfg.main_power {
        cmds.push("PWSTANDBY".to_string());
    }
    // Zone 2 off — the gap CEC cannot cover. Always sent.
    cmds.push("Z2OFF".to_string());
    cmds
}

// ---------------------------------------------------------------------------
// Lifecycle entry points (called from the CEC wake/standby sequences).
// ---------------------------------------------------------------------------

/// Run the **network** half of an AV wake: send the TV cold-wake WoL packet and
/// any configured AVR main-power/input telnet commands. A no-op when `cfg` is
/// `None`. Best-effort — individual failures are logged, not returned, so a
/// missing TV/AVR never blocks the CEC wake.
pub async fn wake(cfg: Option<&AvNetConfig>) {
    let Some(cfg) = cfg else { return };
    if let Some(wol) = &cfg.tv_wol {
        if let Err(e) = send_wol(wol).await {
            tracing::warn!("av-net: WoL wake failed: {e}");
        }
    }
    if let Some(avr) = &cfg.avr {
        let cmds = avr_wake_commands(avr);
        if !cmds.is_empty() {
            if let Err(e) = avr_send(avr.addr, &cmds).await {
                tracing::warn!("av-net: AVR wake telnet failed: {e}");
            }
        }
    }
}

/// Run the **network** half of an AV standby: send the AVR Zone-2 off (and
/// optional main standby) telnet commands. No TV op — sleep is CEC-driven for
/// the display. A no-op when `cfg` is `None` or no AVR is configured.
/// Best-effort — failures are logged, not returned.
pub async fn standby(cfg: Option<&AvNetConfig>) {
    let Some(cfg) = cfg else { return };
    if let Some(avr) = &cfg.avr {
        let cmds = avr_sleep_commands(avr);
        if let Err(e) = avr_send(avr.addr, &cmds).await {
            tracing::warn!("av-net: AVR standby telnet failed: {e}");
        }
    }
}

/// True when this address is the IPv4 limited broadcast (`255.255.255.255`).
/// Exposed for the default-config sanity test.
#[allow(dead_code)]
fn is_limited_broadcast(addr: &SocketAddr) -> bool {
    matches!(addr.ip(), IpAddr::V4(v4) if v4 == Ipv4Addr::BROADCAST)
}

// ---------------------------------------------------------------------------
// Tests (pure helpers only — no sockets opened).
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mac_parses_colon_and_dash() {
        let a = MacAddr::parse("aa:bb:cc:dd:ee:ff").unwrap();
        let b = MacAddr::parse("AA-BB-CC-DD-EE-FF").unwrap();
        assert_eq!(a, MacAddr([0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff]));
        assert_eq!(a, b);
    }

    #[test]
    fn mac_rejects_malformed() {
        assert!(MacAddr::parse("aa:bb:cc:dd:ee").is_err()); // 5 octets
        assert!(MacAddr::parse("aa:bb:cc:dd:ee:ff:00").is_err()); // 7 octets
        assert!(MacAddr::parse("gg:bb:cc:dd:ee:ff").is_err()); // non-hex
        assert!(MacAddr::parse("").is_err());
    }

    #[test]
    fn magic_packet_layout() {
        let mac = MacAddr([0x01, 0x23, 0x45, 0x67, 0x89, 0xab]);
        let pkt = magic_packet(mac);
        // 6 sync bytes of 0xFF.
        assert_eq!(&pkt[0..6], &[0xFF; 6]);
        // Then the MAC repeated exactly 16 times.
        for rep in 0..16 {
            let off = 6 + rep * 6;
            assert_eq!(&pkt[off..off + 6], &mac.0, "repetition {rep}");
        }
        // Total length is the canonical 6 + 16*6 = 102 bytes.
        assert_eq!(pkt.len(), 102);
    }

    #[test]
    fn magic_packet_all_ff_for_broadcast_mac() {
        let pkt = magic_packet(MacAddr([0xFF; 6]));
        assert!(pkt.iter().all(|&b| b == 0xFF));
    }

    #[test]
    fn avr_wake_commands_respect_config() {
        // Minimal: no main power, no input -> empty (CEC drives everything).
        let cfg = AvrConfig {
            addr: "192.0.2.10:23".parse().unwrap(),
            input: None,
            main_power: false,
        };
        assert!(avr_wake_commands(&cfg).is_empty());

        // Input only.
        let cfg = AvrConfig {
            input: Some("GAME".to_string()),
            ..cfg
        };
        assert_eq!(avr_wake_commands(&cfg), vec!["SIGAME".to_string()]);

        // Main power + input, in order (power first, then input).
        let cfg = AvrConfig {
            main_power: true,
            ..cfg
        };
        assert_eq!(
            avr_wake_commands(&cfg),
            vec!["PWON".to_string(), "SIGAME".to_string()]
        );
    }

    #[test]
    fn avr_sleep_commands_always_include_zone2_off() {
        let cfg = AvrConfig {
            addr: "192.0.2.10:23".parse().unwrap(),
            input: Some("GAME".to_string()),
            main_power: false,
        };
        // Zone-2 off is the #186 gap — always present, even without main power.
        assert_eq!(avr_sleep_commands(&cfg), vec!["Z2OFF".to_string()]);

        // With main power: standby first, then Zone-2 off.
        let cfg = AvrConfig {
            main_power: true,
            ..cfg
        };
        assert_eq!(
            avr_sleep_commands(&cfg),
            vec!["PWSTANDBY".to_string(), "Z2OFF".to_string()]
        );
    }

    #[test]
    fn default_wol_broadcast_is_limited_broadcast() {
        let addr = parse_socket_addr(DEFAULT_WOL_BROADCAST).unwrap();
        assert!(is_limited_broadcast(&addr));
        assert_eq!(addr.port(), 9);
    }

    #[test]
    fn parse_socket_addr_rejects_bare_host() {
        // The broadcast form must be numeric host:port (no DNS).
        assert!(parse_socket_addr("not-an-addr").is_err());
        assert!(parse_socket_addr("255.255.255.255").is_err()); // no port
    }

    #[test]
    fn non_empty_trims_and_filters() {
        assert_eq!(non_empty(Some("  x ".to_string())), Some("x".to_string()));
        assert_eq!(non_empty(Some("   ".to_string())), None);
        assert_eq!(non_empty(None), None);
    }
}
