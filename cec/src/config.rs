//! v2 AV-control configuration — `~/.config/tv-shell/cec.toml`.
//!
//! # Why this is a THIRD file, and a third socket
//!
//! V2_DESIGN §11: "beside, not instead, at every shared layer", and §13 Q12's
//! precedent — a new crate beside, never an evolution of. The v1 daemon's
//! `DaemonConfig` root carries `#[serde(deny_unknown_fields)]`, so a `[device]`
//! table added to `config.toml` would make the **v1 daemon abort at startup**,
//! and the symptom would read as "v1 is broken". The same argument applies to
//! `core.toml`, whose root is `deny_unknown_fields` too. So this daemon gets
//! `cec.toml` and `tv-shell-v2-cec.sock`: a third file and a third socket,
//! sharing none with v1 or the core.
//!
//! # Conventions carried from `core/src/config.rs`
//!
//! * Root and every section are `#[serde(default, deny_unknown_fields)]`, so a
//!   typo fails loudly at startup rather than silently running a default.
//! * Three tiers: [`CecConfig::load`] (path from env) → [`CecConfig::load_from`]
//!   (I/O, testable) → [`CecConfig::parse`] (pure).
//! * A missing file is not an error (all-defaults); a present-but-malformed one
//!   is, because an operator should learn their config was ignored.
//! * [`CecConfig::validate`] is separate and runs before anything uses a value,
//!   with messages naming the bad value.
//! * **Every key here has a reader in this crate.** A key whose stated consumer
//!   does not exist is the jedwards1230/tv-shell#416 class, and `core.toml`'s
//!   `[supervisor]` is the standing example of it — thresholds written and read
//!   by nothing. Nothing is added here ahead of the code that reads it.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::failover::Thresholds;
use crate::ip::avr::{self, Avr, AvrEndpoint};
use crate::ip::wol::{self, Mac};
use crate::ip::{IpConfig, WolTarget};
use crate::state::PhysAddr;

/// Env var overriding the config path.
pub const CONFIG_PATH_ENV: &str = "TV_SHELL_CEC_CONFIG";
/// Env var overriding the IPC socket path.
pub const SOCKET_PATH_ENV: &str = "TV_SHELL_CEC_SOCK";
/// Default socket basename.
///
/// Deliberately neither `tv-shell-input.sock` nor `tv-shell-core.sock`: §11
/// requires v1 and v2 to share no socket, and this daemon's grammar is a third
/// one again, so a stray client reaching the wrong socket must fail to connect
/// rather than be answered by the wrong vocabulary.
pub const DEFAULT_SOCKET_NAME: &str = "tv-shell-v2-cec.sock";

/// The longest OSD name the CEC specification allows, in bytes.
pub const MAX_OSD_NAME: usize = 14;

/// The full typed configuration.
#[derive(Debug, Default, Clone, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct CecConfig {
    pub device: DeviceConfig,
    pub avr: AvrSection,
    pub tv: TvSection,
    pub failover: FailoverSection,
}

/// `[device]` — which adapter to open and how to identify ourselves on it.
#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct DeviceConfig {
    /// The CEC device node. Consumed by `kernel::device::open`.
    ///
    /// The unit's `ConditionPathExists=` names `/dev/cec0` literally, so a box
    /// pointing this elsewhere must adjust both; that is deliberate, since the
    /// condition is what keeps the unit inert on every box without an adapter.
    pub path: String,

    /// The adapter's physical address — its `a.b.c.d` port path in the HDMI
    /// topology. Consumed by `kernel::device::open` via `CEC_ADAP_S_PHYS_ADDR`.
    ///
    /// **NOT AUTO-DERIVED, ON PURPOSE.** `cec-ctl --phys-addr-from-edid` reads
    /// the EDID of the connector the adapter sits on, and this adapter sits on a
    /// *different* HDMI input from the video leg: `card1-HDMI-A-1`'s EDID is
    /// readable but belongs to the AVR's video path, so it is the wrong port's
    /// answer. An explicit key is the only honest option.
    ///
    /// **The default is VERIFIED BY MEASUREMENT on the reference deployment,
    /// 2026-09-16.** A live topology scan put the daemon on the bus at this
    /// address, and the receiver independently reported the matching input
    /// selected — so the address asserted here and the input actually carrying
    /// picture agree. It was carried for a while as an unverified pre-2026-08-07
    /// value; it is not one now.
    ///
    /// That measurement settles this deployment and nothing else. A wrong value
    /// fails SILENTLY — a later `<Active Source>` addresses a port that does not
    /// exist and nothing on the bus complains — so the read-back stays the
    /// standing guard for a DIFFERENT installation: the daemon logs the value it
    /// set alongside what `CEC_ADAP_G_PHYS_ADDR` reads back, warns on a mismatch,
    /// and `av-state` publishes both. Verify against `cec-ctl --show-topology`
    /// there before trusting this default.
    pub phys_addr: String,

    /// The OSD name this adapter announces — the input label a TV or AVR shows.
    /// Consumed by `kernel::device::open`.
    ///
    /// At most [`MAX_OSD_NAME`] bytes, per the CEC specification; `validate`
    /// refuses a longer one rather than letting the kernel truncate it into
    /// something the operator did not choose.
    pub osd_name: String,
}

impl Default for DeviceConfig {
    fn default() -> DeviceConfig {
        DeviceConfig {
            path: "/dev/cec0".to_string(),
            // See the field docs: measured against a live bus on 2026-09-16,
            // and a default for one deployment rather than a universal truth.
            phys_addr: "2.5.0.0".to_string(),
            osd_name: "tv-shell".to_string(),
        }
    }
}

/// `[avr]` — the Denon/Marantz receiver's telnet control port.
///
/// **Absent by default, and absent means absent**: an empty `host` is not a
/// receiver at "", it is no receiver at all, and every telnet step is then
/// skipped. This is the IP leg's *capability complement* half — Zone 2 has no
/// CEC expression whatsoever — as well as half of the warm-path failover.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct AvrSection {
    /// Hostname or IP. Empty disables every AVR step.
    ///
    /// Resolved at send time, never here: a name that does not resolve must not
    /// stop the daemon from starting on a box whose receiver is unplugged.
    pub host: String,
    /// The control port. Denon/Marantz speak ASCII over TCP/23.
    pub port: u16,
    /// The source code to select on wake (`SI<input>`, e.g. `GAME`). Empty
    /// skips the input switch.
    ///
    /// Validated as ASCII alphanumeric, because it is interpolated into a
    /// carriage-return-terminated control line — see
    /// [`crate::ip::avr::validate_input_code`].
    pub input: String,
    /// Drive the receiver's MAIN-zone power over telnet (`PWON` / `PWSTANDBY`).
    ///
    /// **Off by default.** On the CEC path the main zone is already powered by
    /// `<Image View On>` / `<Standby>`; and powering the main zone *down* is the
    /// action that can black out a television somebody is watching, so this
    /// daemon does not grant itself that authority merely because its adapter
    /// stopped answering.
    pub main_power: bool,
    /// Send `Z2OFF` on every standby.
    ///
    /// **On by default, and it runs with a perfectly healthy CEC bus**: Zone 2
    /// is not CEC-addressable at all, so this is a capability the bus does not
    /// have rather than a fallback for when it fails.
    pub zone2_off: bool,
}

impl Default for AvrSection {
    fn default() -> AvrSection {
        AvrSection {
            host: String::new(),
            port: avr::DEFAULT_PORT,
            input: String::new(),
            main_power: false,
            zone2_off: true,
        }
    }
}

/// `[tv]` — the television's IP leg, which is **Wake-on-LAN and nothing else**.
///
/// There is no webOS/SSAP client here and none is planned: see
/// [`crate::ip::wol`] for why the TV IP leg is write-only with no state read.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct TvSection {
    /// The television's MAC. Empty disables Wake-on-LAN.
    pub wol_mac: String,
    /// Where to broadcast the magic packet. Numeric `addr:port` — a magic packet
    /// is a broadcast, and a DNS name would resolve to a single host.
    pub wol_broadcast: String,
}

impl Default for TvSection {
    fn default() -> TvSection {
        TvSection {
            wol_mac: String::new(),
            wol_broadcast: "255.255.255.255:9".to_string(),
        }
    }
}

/// `[failover]` — the warm-path decision's four thresholds.
///
/// Every key here is read by [`crate::failover::Failover`], and
/// `every_threshold_changes_a_decision` fails if one stops being.
#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct FailoverSection {
    /// Consecutive transmit failures, with nothing heard in the same window,
    /// that count as a wedged adapter. **At least 2** — a threshold of 1 makes
    /// one NAK a backend change.
    pub tx_error_threshold: u32,
    /// How far apart those failures may be and still be one run.
    pub tx_error_window_ms: u64,
    /// How long a failing observation must hold before the backend changes.
    pub fail_after_ms: u64,
    /// How long health must hold, across re-observations, before it changes
    /// back.
    pub recover_after_ms: u64,
}

impl Default for FailoverSection {
    fn default() -> FailoverSection {
        let t = Thresholds::default();
        FailoverSection {
            tx_error_threshold: t.tx_error_threshold,
            tx_error_window_ms: t.tx_error_window_ms,
            fail_after_ms: t.fail_after_ms,
            recover_after_ms: t.recover_after_ms,
        }
    }
}

impl CecConfig {
    /// Load from the resolved path. A missing file yields all-defaults.
    pub fn load() -> anyhow::Result<Self> {
        Self::load_from(&config_path())
    }

    /// Load from an explicit path (testable; no env or global state).
    pub fn load_from(path: &Path) -> anyhow::Result<Self> {
        match std::fs::read_to_string(path) {
            Ok(text) => Self::parse(&text),
            // Absent ⇒ defaults, so a fresh install still starts. Any other read
            // error (permissions, a directory) surfaces.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(anyhow::anyhow!("reading {}: {e}", path.display())),
        }
    }

    /// Parse a TOML document (no I/O).
    pub fn parse(text: &str) -> anyhow::Result<Self> {
        toml::from_str(text).map_err(|e| anyhow::anyhow!("parsing cec.toml: {e}"))
    }

    /// Reject values that would fail confusingly later.
    pub fn validate(&self) -> anyhow::Result<()> {
        if self.device.path.is_empty() {
            anyhow::bail!("config: [device] path must not be empty");
        }
        self.phys_addr()?;
        if self.device.osd_name.is_empty() {
            anyhow::bail!("config: [device] osd_name must not be empty");
        }
        if self.device.osd_name.len() > MAX_OSD_NAME {
            anyhow::bail!(
                "config: [device] osd_name must be at most {MAX_OSD_NAME} bytes (got {}, {:?}); \
                 the CEC specification caps it there and the kernel would truncate it",
                self.device.osd_name.len(),
                self.device.osd_name
            );
        }
        if !self.device.osd_name.is_ascii() {
            anyhow::bail!(
                "config: [device] osd_name must be ASCII (got {:?}); CEC carries no other \
                 encoding, so a TV would display something other than what was configured",
                self.device.osd_name
            );
        }
        // Both of these parse the IP leg and the thresholds for their errors
        // alone: a bad MAC or a zero hysteresis must fail here, naming the key,
        // rather than at the first packet — where a WoL failure is SILENT.
        self.ip()?;
        self.thresholds()?;
        Ok(())
    }

    /// The IP leg, in [`crate::ip`]'s own terms.
    ///
    /// Fallible rather than parsed at deserialize time so each failure carries a
    /// `config:` message naming the key, like every other validation here. An
    /// empty `[avr].host` or `[tv].wol_mac` is **not** an error: it means that
    /// half is not configured, which is the default and the common case.
    pub fn ip(&self) -> anyhow::Result<IpConfig> {
        let avr = if self.avr.host.trim().is_empty() {
            None
        } else {
            if !self.avr.input.is_empty() {
                avr::validate_input_code(&self.avr.input)
                    .map_err(|e| anyhow::anyhow!("config: [avr] input {e}"))?;
            }
            if self.avr.port == 0 {
                anyhow::bail!("config: [avr] port must not be zero");
            }
            Some(Avr {
                endpoint: AvrEndpoint {
                    host: self.avr.host.trim().to_string(),
                    port: self.avr.port,
                },
                input: Some(self.avr.input.clone()).filter(|i| !i.is_empty()),
                main_power: self.avr.main_power,
                zone2_off: self.avr.zone2_off,
            })
        };

        let tv_wol = if self.tv.wol_mac.trim().is_empty() {
            None
        } else {
            let mac = Mac::parse(&self.tv.wol_mac).ok_or_else(|| {
                anyhow::anyhow!(
                    "config: [tv] wol_mac is not a MAC address ({:?}); it must be six hex \
                     octets separated by ':' or '-'",
                    self.tv.wol_mac
                )
            })?;
            let broadcast: SocketAddr = self.tv.wol_broadcast.parse().map_err(|e| {
                anyhow::anyhow!(
                    "config: [tv] wol_broadcast must be a numeric addr:port ({:?}: {e}); a \
                     magic packet is a broadcast, so a name that resolves to one host is not \
                     a destination for it",
                    self.tv.wol_broadcast
                )
            })?;
            if !wol::is_limited_broadcast(&broadcast) && broadcast.ip().is_loopback() {
                anyhow::bail!(
                    "config: [tv] wol_broadcast {broadcast} is a loopback address, which no \
                     television can receive"
                );
            }
            Some(WolTarget { mac, broadcast })
        };

        Ok(IpConfig { avr, tv_wol })
    }

    /// The failover thresholds, validated.
    pub fn thresholds(&self) -> anyhow::Result<Thresholds> {
        let t = Thresholds {
            tx_error_threshold: self.failover.tx_error_threshold,
            tx_error_window_ms: self.failover.tx_error_window_ms,
            fail_after_ms: self.failover.fail_after_ms,
            recover_after_ms: self.failover.recover_after_ms,
        };
        t.validate()
            .map_err(|e| anyhow::anyhow!("config: [failover] {e}"))?;
        Ok(t)
    }

    /// The configured physical address, parsed.
    ///
    /// Fallible rather than parsed at deserialize time so the failure carries a
    /// `config:` message naming the key, like every other validation here.
    pub fn phys_addr(&self) -> anyhow::Result<PhysAddr> {
        self.device.phys_addr.parse::<PhysAddr>().map_err(|e| {
            anyhow::anyhow!("config: [device] phys_addr is not a physical address: {e}")
        })
    }
}

/// `$TV_SHELL_CEC_CONFIG`, else `${XDG_CONFIG_HOME:-$HOME/.config}/tv-shell/cec.toml`.
pub fn config_path() -> PathBuf {
    if let Some(p) = std::env::var_os(CONFIG_PATH_ENV) {
        return PathBuf::from(p);
    }
    config_dir().join("cec.toml")
}

/// `$TV_SHELL_CEC_SOCK`, else `/run/user/<uid>/tv-shell-v2-cec.sock`.
pub fn socket_path() -> String {
    if let Ok(p) = std::env::var(SOCKET_PATH_ENV) {
        if !p.is_empty() {
            return p;
        }
    }
    // SAFETY: `getuid` is always safe — it cannot fail and touches no memory.
    let uid = unsafe { libc::getuid() };
    format!("/run/user/{uid}/{DEFAULT_SOCKET_NAME}")
}

/// `${XDG_CONFIG_HOME:-$HOME/.config}/tv-shell`.
fn config_dir() -> PathBuf {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| {
            let home = std::env::var_os("HOME").unwrap_or_default();
            PathBuf::from(home).join(".config")
        });
    base.join("tv-shell")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_document_is_all_defaults() {
        let c = CecConfig::parse("").unwrap();
        assert_eq!(c, CecConfig::default());
        assert_eq!(c.device.path, "/dev/cec0");
        assert_eq!(c.device.phys_addr, "2.5.0.0");
        assert_eq!(c.device.osd_name, "tv-shell");
        c.validate().unwrap();
    }

    #[test]
    fn a_populated_document_parses() {
        let c = CecConfig::parse(
            r#"
            [device]
            path = "/dev/cec1"
            phys_addr = "1.0.0.0"
            osd_name = "htpc"
            "#,
        )
        .unwrap();
        assert_eq!(c.device.path, "/dev/cec1");
        assert_eq!(c.phys_addr().unwrap().to_string(), "1.0.0.0");
        assert_eq!(c.device.osd_name, "htpc");
    }

    /// `deny_unknown_fields` is what turns a typo into a startup failure rather
    /// than a silently-defaulted value.
    #[test]
    fn an_unknown_key_is_a_parse_error() {
        assert!(CecConfig::parse("[device]\nphysaddr = \"1.0.0.0\"").is_err());
        assert!(CecConfig::parse("[devices]\npath = \"/dev/cec0\"").is_err());
        assert!(CecConfig::parse("nonsense = 1").is_err());
    }

    /// Nothing here may be read out of v1's or the core's file, and nothing of
    /// theirs may be read out of this one.
    #[test]
    fn this_file_shares_no_name_with_v1_or_the_core() {
        // Deliberately NOT via `config_path()`: that reads the environment, and
        // an environment READ concurrent with another test's write is unsound on
        // its own (the core crate keeps a whole-crate lock for exactly this).
        // The default path is what this asserts, so it asks the default path.
        let p = config_dir().join("cec.toml");
        let name = p.file_name().unwrap().to_string_lossy();
        assert_eq!(name, "cec.toml");
        assert_ne!(name, "config.toml", "v1's file");
        assert_ne!(name, "core.toml", "the core's file");
        assert_ne!(DEFAULT_SOCKET_NAME, "tv-shell-input.sock");
        assert_ne!(DEFAULT_SOCKET_NAME, "tv-shell-core.sock");
    }

    #[test]
    fn a_malformed_physical_address_fails_validation_naming_the_key() {
        let c = CecConfig::parse("[device]\nphys_addr = \"25.0.0\"").unwrap();
        let e = c.validate().unwrap_err().to_string();
        assert!(e.contains("[device] phys_addr"), "{e}");
    }

    /// **The rule: an OSD name the CEC specification cannot carry is refused at
    /// startup, not truncated at runtime.**
    ///
    /// A truncated label is the operator's chosen name silently replaced by a
    /// different one on the television.
    #[test]
    fn an_osd_name_that_cec_cannot_carry_is_refused() {
        let long = "x".repeat(MAX_OSD_NAME + 1);
        let c = CecConfig::parse(&format!("[device]\nosd_name = {long:?}")).unwrap();
        let e = c.validate().unwrap_err().to_string();
        assert!(e.contains("osd_name"), "{e}");

        // Exactly at the limit is fine.
        let at = "x".repeat(MAX_OSD_NAME);
        CecConfig::parse(&format!("[device]\nosd_name = {at:?}"))
            .unwrap()
            .validate()
            .unwrap();

        // Empty and non-ASCII are refused too.
        assert!(CecConfig::parse("[device]\nosd_name = \"\"")
            .unwrap()
            .validate()
            .is_err());
        assert!(CecConfig::parse("[device]\nosd_name = \"tv-shéll\"")
            .unwrap()
            .validate()
            .is_err());
    }

    #[test]
    fn an_empty_device_path_fails_validation() {
        let c = CecConfig::parse("[device]\npath = \"\"").unwrap();
        assert!(c.validate().unwrap_err().to_string().contains("path"));
    }

    /// **The rule: the IP leg is absent until it is configured.**
    ///
    /// A default box has no receiver and no television MAC, so `wake` and
    /// `standby` put nothing on any network — and [`crate::failover`] reports
    /// `cec` as the only available backend rather than announcing one that does
    /// not exist.
    #[test]
    fn the_ip_leg_is_absent_by_default() {
        let c = CecConfig::default();
        let ip = c.ip().unwrap();
        assert_eq!(ip, crate::ip::IpConfig::default());
        assert!(!ip.is_configured());
        // The defaults that do exist are the harmless ones.
        assert_eq!(c.avr.port, 23);
        assert!(c.avr.zone2_off, "Zone 2 is the whole point of this leg");
        assert!(!c.avr.main_power, "powering the main zone down is opt-in");
        assert_eq!(c.tv.wol_broadcast, "255.255.255.255:9");
    }

    #[test]
    fn a_configured_ip_leg_parses_into_the_domain_types() {
        let c = CecConfig::parse(
            r#"
            [avr]
            host = "192.0.2.10"
            port = 23
            input = "GAME"
            main_power = true
            zone2_off = true

            [tv]
            wol_mac = "aa:bb:cc:dd:ee:ff"
            wol_broadcast = "255.255.255.255:9"
            "#,
        )
        .unwrap();
        c.validate().unwrap();
        let ip = c.ip().unwrap();
        assert!(ip.is_configured());
        let avr = ip.avr.unwrap();
        assert_eq!(avr.endpoint.to_string(), "192.0.2.10:23");
        assert_eq!(avr.input.as_deref(), Some("GAME"));
        assert!(avr.main_power);
        let wol = ip.tv_wol.unwrap();
        assert_eq!(wol.mac.to_canonical(), "aa:bb:cc:dd:ee:ff");
        assert_eq!(wol.broadcast.port(), 9);
    }

    /// **The rule: a malformed IP-leg value fails at startup naming the key.**
    ///
    /// Every failure mode this guards is SILENT at runtime — nothing
    /// acknowledges a magic packet, and a control line with an embedded carriage
    /// return is accepted by the receiver as a second command.
    #[test]
    fn a_malformed_ip_leg_value_fails_validation_naming_its_key() {
        let cases = [
            ("[tv]\nwol_mac = \"nonsense\"", "[tv] wol_mac"),
            ("[tv]\nwol_mac = \"aa:bb:cc:dd:ee\"", "[tv] wol_mac"),
            (
                "[tv]\nwol_mac = \"aa:bb:cc:dd:ee:ff\"\nwol_broadcast = \"broadcast-host\"",
                "[tv] wol_broadcast",
            ),
            (
                "[tv]\nwol_mac = \"aa:bb:cc:dd:ee:ff\"\nwol_broadcast = \"127.0.0.1:9\"",
                "[tv] wol_broadcast",
            ),
            (
                "[avr]\nhost = \"192.0.2.10\"\ninput = \"GAME\\rZ2OFF\"",
                "[avr] input",
            ),
            ("[avr]\nhost = \"192.0.2.10\"\nport = 0", "[avr] port"),
            (
                "[failover]\ntx_error_threshold = 1",
                "[failover] tx_error_threshold",
            ),
            ("[failover]\nfail_after_ms = 0", "[failover] fail_after_ms"),
            (
                "[failover]\nrecover_after_ms = 0",
                "[failover] recover_after_ms",
            ),
        ];
        for (doc, expected) in cases {
            let c = CecConfig::parse(doc).unwrap_or_else(|e| panic!("{doc:?} must parse: {e}"));
            let e = c.validate().unwrap_err().to_string();
            assert!(e.contains(expected), "{doc:?} -> {e}");
        }
    }

    /// The thresholds reach [`crate::failover::Thresholds`] as written — the
    /// config half of "every key here has a reader".
    #[test]
    fn the_failover_thresholds_reach_the_decision_module() {
        let c = CecConfig::parse(
            r#"
            [failover]
            tx_error_threshold = 7
            tx_error_window_ms = 1234
            fail_after_ms = 4321
            recover_after_ms = 9876
            "#,
        )
        .unwrap();
        let t = c.thresholds().unwrap();
        assert_eq!(t.tx_error_threshold, 7);
        assert_eq!(t.tx_error_window_ms, 1234);
        assert_eq!(t.fail_after_ms, 4321);
        assert_eq!(t.recover_after_ms, 9876);
    }

    /// `deny_unknown_fields` reaches the new sections too.
    #[test]
    fn an_unknown_key_in_a_new_section_is_a_parse_error() {
        for doc in [
            "[avr]\nhostname = \"192.0.2.10\"",
            "[tv]\nmac = \"aa:bb:cc:dd:ee:ff\"",
            "[failover]\nthreshold = 3",
            "[avrs]\nhost = \"192.0.2.10\"",
        ] {
            assert!(CecConfig::parse(doc).is_err(), "{doc:?}");
        }
    }

    #[test]
    fn a_missing_file_is_all_defaults_and_an_unreadable_one_is_not() {
        let missing = std::path::Path::new("/nonexistent/tv-shell/cec.toml");
        assert_eq!(CecConfig::load_from(missing).unwrap(), CecConfig::default());
        // A directory is not "absent" — it is a misconfiguration, and it surfaces.
        assert!(CecConfig::load_from(std::path::Path::new("/tmp")).is_err());
    }
}
