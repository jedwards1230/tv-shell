//! Wake-on-LAN — **the magic packet, and nothing else**.
//!
//! # Why this is the whole TV IP leg
//!
//! V2_DESIGN §8 promises "webOS for state and standby" for the television's IP
//! path. **There is no webOS code in this tree** — no SSAP client, no pairing
//! key, nothing — and none is written here. The TV IP leg is **WoL-only,
//! write-only, no state read**, by decision:
//!
//! * WoL is the only IP operation the television genuinely needs that CEC cannot
//!   do. `<Image View On>` reaches nothing when the panel is at mains standby;
//!   a magic packet is what cold-starts it.
//! * A webOS client is a **second auth surface** with a documented history of
//!   breaking across firmware — one of the three reasons §13 Q7 demoted IP in
//!   the first place. Adding one to fetch a power state this daemon already
//!   reads over CEC when CEC works, and cannot trust when CEC does not, buys a
//!   pairing key and a maintenance burden.
//!
//! So this module writes and never reads, and [`crate::state::AvState`] gains no
//! field from it. A WoL packet that leaves the host is **not** evidence the
//! television woke: nothing acknowledges a magic packet. That is reported as
//! what it is (see [`crate::ip::IpOutcome`]) rather than as a success.
//!
//! # Ported, not copied
//!
//! [`Mac::parse`] and [`magic_packet`] are ports of `daemon/src/wol.rs:43` and
//! `:77` — the two pure functions there, and **only** those two. Everything from
//! `pick_mac` onward in that file (the `ip neigh` scrape, the
//! `~/.config/tv-shell/host-macs.json` cache, `handle_wol`,
//! `wake_active_host_if_enabled`) is Steam-host wiring for waking the streaming
//! PC and has no business here: `daemon/src/wol.rs` is not the television's WoL.
//!
//! The never-merged jedwards1230/tv-shell#191 carried its own `MacAddr` /
//! `magic_packet` pair. Those are deliberately dropped in favour of these, so
//! there is one implementation rather than two.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};

/// The canonical magic-packet length: 6 sync bytes + the MAC 16 times.
pub const MAGIC_PACKET_LEN: usize = 102;

/// How many times the target MAC is repeated after the sync stream.
const REPETITIONS: usize = 16;

/// A 48-bit Ethernet MAC address.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Mac(pub [u8; 6]);

impl Mac {
    /// Parse a colon- or dash-separated MAC (`aa:bb:cc:dd:ee:ff`).
    ///
    /// `None` for the wrong octet count or a non-hex octet; case-insensitive.
    /// Fallible rather than lenient on purpose: a MAC this daemon cannot parse
    /// must fail at `validate()` naming the key, because a magic packet sent to
    /// the wrong address is **silent** — nothing on the network reports it, and
    /// the only symptom is a television that does not come on.
    #[must_use]
    pub fn parse(s: &str) -> Option<Mac> {
        let mut octets = [0u8; 6];
        let mut count = 0;
        for part in s.split([':', '-']) {
            if count >= octets.len() {
                // Too many octets. Bailing here rather than after the loop
                // keeps the index in range without an assertion.
                return None;
            }
            octets[count] = u8::from_str_radix(part.trim(), 16).ok()?;
            count += 1;
        }
        (count == octets.len()).then_some(Mac(octets))
    }

    /// Canonical lowercase colon-separated rendering.
    #[must_use]
    pub fn to_canonical(self) -> String {
        let b = self.0;
        format!(
            "{:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
            b[0], b[1], b[2], b[3], b[4], b[5]
        )
    }
}

impl std::fmt::Display for Mac {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.to_canonical())
    }
}

/// Build the standard Wake-on-LAN magic packet: six `0xFF` sync bytes followed
/// by the target MAC repeated sixteen times, 102 bytes in all.
///
/// Pure — no I/O — so the layout is covered by CI with no network at all.
#[must_use]
pub fn magic_packet(mac: Mac) -> [u8; MAGIC_PACKET_LEN] {
    let mut packet = [0xFFu8; MAGIC_PACKET_LEN];
    for rep in 0..REPETITIONS {
        let off = 6 + rep * 6;
        packet[off..off + 6].copy_from_slice(&mac.0);
    }
    packet
}

/// True when this address is the IPv4 limited broadcast (`255.255.255.255`).
///
/// Used by the default-config check: the shipped default has to be a broadcast
/// or the packet reaches exactly one host, which is the one case WoL is not for.
#[must_use]
pub fn is_limited_broadcast(addr: &SocketAddr) -> bool {
    matches!(addr.ip(), IpAddr::V4(v4) if v4 == Ipv4Addr::BROADCAST)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_mac_parses_with_either_separator_and_either_case() {
        let a = Mac::parse("aa:bb:cc:dd:ee:ff").unwrap();
        let b = Mac::parse("AA-BB-CC-DD-EE-FF").unwrap();
        assert_eq!(a, Mac([0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff]));
        assert_eq!(a, b);
        assert_eq!(a.to_canonical(), "aa:bb:cc:dd:ee:ff");
    }

    /// **The rule: a MAC this daemon cannot parse is refused, never coerced.**
    ///
    /// A magic packet to the wrong address fails silently — nothing
    /// acknowledges one — so the only place the operator can learn about a typo
    /// is here.
    #[test]
    fn a_malformed_mac_is_refused() {
        for s in [
            "",
            "aa:bb:cc:dd:ee",
            "aa:bb:cc:dd:ee:ff:00",
            "gg:bb:cc:dd:ee:ff",
            "aabbccddeeff",
            "aa:bb:cc:dd:ee:",
            "aa::bb:cc:dd:ee:ff",
            "0xaa:bb:cc:dd:ee:ff",
        ] {
            assert!(Mac::parse(s).is_none(), "{s:?} must not parse as a MAC");
        }
    }

    /// The canonical 6 + 16×6 layout, asserted byte by byte.
    #[test]
    fn the_magic_packet_is_six_sync_bytes_then_the_mac_sixteen_times() {
        let mac = Mac([0x01, 0x23, 0x45, 0x67, 0x89, 0xab]);
        let packet = magic_packet(mac);
        assert_eq!(packet.len(), 102);
        assert_eq!(&packet[0..6], &[0xFF; 6]);
        for rep in 0..16 {
            let off = 6 + rep * 6;
            assert_eq!(&packet[off..off + 6], &mac.0, "repetition {rep}");
        }
    }

    #[test]
    fn the_broadcast_mac_yields_an_all_ff_packet() {
        assert!(magic_packet(Mac([0xFF; 6])).iter().all(|&b| b == 0xFF));
    }

    #[test]
    fn the_limited_broadcast_is_recognised() {
        assert!(is_limited_broadcast(
            &"255.255.255.255:9".parse::<SocketAddr>().unwrap()
        ));
        assert!(!is_limited_broadcast(
            &"192.0.2.10:9".parse::<SocketAddr>().unwrap()
        ));
    }
}
