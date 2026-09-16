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

use std::path::{Path, PathBuf};

use serde::Deserialize;

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
    /// **`2.5.0.0` is UNVERIFIED against the current rack** (plan §7 item 3). It
    /// is the pre-2026-08-07 value and the rack has changed since. A wrong value
    /// fails SILENTLY — a later `<Active Source>` addresses a port that does not
    /// exist and nothing on the bus complains — which is why the daemon logs the
    /// value it set alongside what `CEC_ADAP_G_PHYS_ADDR` reads back, and why
    /// `av-state` publishes both. Verify against `cec-ctl --show-topology` once
    /// `/dev/cec0` exists.
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
            // See the field docs: the pre-2026-08-07 value, carried forward as a
            // starting point and flagged as unverified, not as a known-good one.
            phys_addr: "2.5.0.0".to_string(),
            osd_name: "tv-shell".to_string(),
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
        Ok(())
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

    #[test]
    fn a_missing_file_is_all_defaults_and_an_unreadable_one_is_not() {
        let missing = std::path::Path::new("/nonexistent/tv-shell/cec.toml");
        assert_eq!(CecConfig::load_from(missing).unwrap(), CecConfig::default());
        // A directory is not "absent" — it is a misconfiguration, and it surfaces.
        assert!(CecConfig::load_from(std::path::Path::new("/tmp")).is_err());
    }
}
