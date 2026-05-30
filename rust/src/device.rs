//! Gamepad discovery + device identity.
//!
//! Replaces the Python daemon's hardcoded Xbox `0x045e:0x028e` vendor/product
//! match with SDL-style controller identification: compute the SDL joystick
//! GUID from the device's `input_id` and check membership in the bundled
//! `SDL_GameControllerDB`. This lets the daemon grab an *arbitrary* known
//! controller, not just the Xbox pad.
//!
//! Device identity has two layers:
//!   * **in-process ownership** — a [`VirtualRegistry`] of the raw fds of every
//!     uinput device *we* create (virtual keyboard/mouse, per-player virtual
//!     pads). Discovery skips any device whose open fd we already own. This
//!     replaces the old `is_synthetic` name-string match.
//!   * **stable wire id** — a per-pad string derived from evdev `uniq`/`phys`
//!     (else `vendor:product:path`), used in `pad:*` IPC payloads so the UI can
//!     track a physical pad across reconnects. The in-process key stays the fd.
//!
//! The GUID math, the registry, and the wire-id derivation are all
//! platform-independent and unit-tested; the actual `/dev/input/event*`
//! enumeration and grab live behind `cfg(linux)`.

use std::collections::HashSet;
use std::os::fd::RawFd;

/// Compute the 16-byte SDL joystick GUID for a Linux device.
///
/// Layout (matches SDL's `SDL_CreateJoystickGUID` on Linux):
/// `bus(LE16) | crc(LE16) | vendor(LE16) | 0 0 | product(LE16) | 0 0 | version(LE16) | 0 0`.
/// We leave the CRC field zero (we don't hash the name); DB matching ignores it.
pub fn sdl_guid(bus: u16, vendor: u16, product: u16, version: u16) -> [u8; 16] {
    let mut g = [0u8; 16];
    g[0..2].copy_from_slice(&bus.to_le_bytes());
    // g[2..4] crc -> left zero
    g[4..6].copy_from_slice(&vendor.to_le_bytes());
    g[8..10].copy_from_slice(&product.to_le_bytes());
    g[12..14].copy_from_slice(&version.to_le_bytes());
    g
}

/// Lowercase 32-char hex rendering of a GUID, as used in `gamecontrollerdb.txt`.
pub fn guid_to_string(guid: &[u8; 16]) -> String {
    let mut s = String::with_capacity(32);
    for b in guid {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

/// Parse a 32-char hex GUID string into bytes. Returns `None` if malformed.
fn parse_guid(s: &str) -> Option<[u8; 16]> {
    let s = s.trim();
    if s.len() != 32 || !s.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let mut g = [0u8; 16];
    for (i, byte) in g.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).ok()?;
    }
    Some(g)
}

fn vendor_of(guid: &[u8; 16]) -> u16 {
    u16::from_le_bytes([guid[4], guid[5]])
}

fn product_of(guid: &[u8; 16]) -> u16 {
    u16::from_le_bytes([guid[8], guid[9]])
}

/// A parsed controller database: the set of known `(vendor, product)` pairs.
///
/// We match on vendor/product rather than full-GUID equality: the bus and
/// version fields and the optional CRC vary between how a device presents and
/// how the DB recorded it, but vendor/product reliably identifies a controller
/// model. Entries with a zero vendor (SDL name-encoded GUIDs) are ignored.
#[derive(Debug, Default, Clone)]
pub struct ControllerDb {
    known: HashSet<(u16, u16)>,
}

impl ControllerDb {
    pub fn parse(text: &str) -> ControllerDb {
        let mut known = HashSet::new();
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let Some(first) = line.split(',').next() else {
                continue;
            };
            let Some(guid) = parse_guid(first) else {
                continue;
            };
            let (v, p) = (vendor_of(&guid), product_of(&guid));
            if v != 0 {
                known.insert((v, p));
            }
        }
        ControllerDb { known }
    }

    pub fn is_known(&self, vendor: u16, product: u16) -> bool {
        self.known.contains(&(vendor, product))
    }

    pub fn len(&self) -> usize {
        self.known.len()
    }

    pub fn is_empty(&self) -> bool {
        self.known.is_empty()
    }
}

/// The bundled baseline DB (common controllers). A fuller upstream
/// `SDL_GameControllerDB` can be supplied at runtime via the
/// `GAME_SHELL_GAMECONTROLLERDB` env var (see `load_db`).
const BUNDLED_DB: &str = include_str!("../assets/gamecontrollerdb.txt");

/// Load the controller DB: an operator-supplied file (env override) layered
/// over the bundled baseline.
pub fn load_db() -> ControllerDb {
    let mut db = ControllerDb::parse(BUNDLED_DB);
    if let Some(path) = std::env::var_os("GAME_SHELL_GAMECONTROLLERDB") {
        if let Ok(text) = std::fs::read_to_string(&path) {
            let extra = ControllerDb::parse(&text);
            db.known.extend(extra.known);
        }
    }
    db
}

/// Registry of the raw fds of every uinput device the daemon owns.
///
/// The daemon both *produces* virtual input devices (the virtual keyboard and
/// mouse, plus one clean virtual gamepad per player in local-multiplayer mode)
/// and *consumes* physical gamepads. Without a way to tell its own devices
/// apart it would re-grab them during the next discovery pass — the old code
/// papered over this with `is_synthetic`, a brittle name-string match against
/// `game-shell-virtual-*` and `ydotoold`.
///
/// We replace that with ownership by fd: every uinput device registers its open
/// fd at creation, and discovery skips any enumerated device whose fd we own.
/// The fd is the daemon's stable in-process key for a device for the whole
/// fleet (Phase 4); registering it here is the single source of truth for "this
/// is ours."
///
/// Note this guards only *our own* devices. Rejecting a *foreign* software
/// injector such as ydotoold (which also advertises `BTN_SOUTH`) is handled
/// separately by the DB-match gate in [`find_gamepad`] — fd ownership and the
/// DB gate are complementary, not redundant.
#[derive(Debug, Default, Clone)]
pub struct VirtualRegistry {
    owned_fds: HashSet<RawFd>,
}

impl VirtualRegistry {
    pub fn new() -> VirtualRegistry {
        VirtualRegistry {
            owned_fds: HashSet::new(),
        }
    }

    /// Record a uinput device's raw fd as daemon-owned.
    pub fn register(&mut self, fd: RawFd) {
        self.owned_fds.insert(fd);
    }

    /// Forget a uinput device's fd (e.g. a virtual pad torn down on player leave).
    pub fn unregister(&mut self, fd: RawFd) {
        self.owned_fds.remove(&fd);
    }

    /// True if `fd` belongs to a uinput device the daemon created.
    pub fn owns(&self, fd: RawFd) -> bool {
        self.owned_fds.contains(&fd)
    }

    pub fn len(&self) -> usize {
        self.owned_fds.len()
    }

    pub fn is_empty(&self) -> bool {
        self.owned_fds.is_empty()
    }
}

/// Derive a stable wire id for a pad from its evdev identity.
///
/// Preference order, most-to-least stable:
///   1. `uniq` (evdev "unique name" — a controller serial / BT MAC) when present;
///   2. `phys` (the physical port/path the device hangs off) when present;
///   3. `vendor:product:path` as a last resort (path keeps two identical pads on
///      different ports distinct, even if neither exposes uniq/phys).
///
/// Empty `uniq`/`phys` strings (the kernel reports `""` rather than absent for
/// some devices) are treated as missing. The result is purely for `pad:*` IPC
/// payloads so the UI can follow a physical pad across reconnects; the daemon's
/// own in-process key stays the fd.
pub fn derive_wire_id(
    uniq: Option<&str>,
    phys: Option<&str>,
    vendor: u16,
    product: u16,
    path: &str,
) -> String {
    if let Some(u) = uniq.map(str::trim).filter(|s| !s.is_empty()) {
        return format!("uniq:{u}");
    }
    if let Some(p) = phys.map(str::trim).filter(|s| !s.is_empty()) {
        return format!("phys:{p}");
    }
    format!("vp:{vendor:04x}:{product:04x}:{path}")
}

/// Parse a `GAMEPAD_VENDOR`/`GAMEPAD_PRODUCT`-style override (supports `0x`
/// hex, like Python's `int(x, 0)`). Returns `None` if unset/unparseable.
pub fn parse_id_env(var: &str) -> Option<u16> {
    let raw = std::env::var(var).ok()?;
    let raw = raw.trim();
    let parsed = if let Some(hex) = raw.strip_prefix("0x").or_else(|| raw.strip_prefix("0X")) {
        u16::from_str_radix(hex, 16)
    } else {
        raw.parse::<u16>()
    };
    parsed.ok()
}

// ---------------------------------------------------------------------------
// Linux device enumeration / grab
// ---------------------------------------------------------------------------

#[cfg(target_os = "linux")]
pub use linux::{find_gamepad, GamepadHandle};

#[cfg(target_os = "linux")]
mod linux {
    use super::*;
    use evdev::{Device, KeyCode};
    use std::os::fd::AsRawFd;
    use std::path::PathBuf;

    /// A discovered gamepad: its evdev device plus its display name/path and the
    /// stable wire id derived at discovery time.
    pub struct GamepadHandle {
        pub device: Device,
        pub name: String,
        pub path: PathBuf,
        pub wire_id: String,
    }

    fn has_btn_south(dev: &Device) -> bool {
        dev.supported_keys()
            .is_some_and(|keys| keys.contains(KeyCode::BTN_SOUTH))
    }

    /// Find a gamepad. Selection order:
    /// 1. If both `GAMEPAD_VENDOR` and `GAMEPAD_PRODUCT` are set -> exact match
    ///    (legacy operator pin). The pin is an explicit operator decision, so it
    ///    bypasses the DB gate.
    /// 2. Else require a controller-DB GUID match.
    ///
    /// In every case we skip devices whose fd we already own (our own uinput
    /// keyboard/mouse/virtual pads), tracked by `reg`.
    ///
    /// There is deliberately **no bare-`BTN_SOUTH` fallback**. ydotoold's virtual
    /// device advertises `BTN_SOUTH` but is not in any controller DB, so a "grab
    /// the first `BTN_SOUTH` device" fallback would grab it as a bogus pad —
    /// that is the exact failure the old `is_synthetic` name match patched over.
    /// Requiring a DB match rejects foreign injectors structurally. An operator
    /// with a controller the bundled DB doesn't know can either pin it via
    /// `GAMEPAD_VENDOR`/`GAMEPAD_PRODUCT` or extend the DB via
    /// `GAME_SHELL_GAMECONTROLLERDB`.
    pub fn find_gamepad(db: &ControllerDb, reg: &VirtualRegistry) -> Option<GamepadHandle> {
        let pin_vendor = parse_id_env("GAMEPAD_VENDOR");
        let pin_product = parse_id_env("GAMEPAD_PRODUCT");
        let pinned = matches!((pin_vendor, pin_product), (Some(_), Some(_)));

        let mut devices: Vec<(PathBuf, Device)> = evdev::enumerate().collect();
        // Deterministic order (mirrors Python's sorted(list_devices())).
        devices.sort_by(|a, b| a.0.cmp(&b.0));

        for (path, dev) in devices {
            if !has_btn_south(&dev) {
                continue;
            }
            // Skip our own uinput devices by fd ownership. (A freshly-opened
            // physical pad can never collide with one of our control fds, so
            // this only ever filters devices we created.)
            if reg.owns(dev.as_raw_fd()) {
                continue;
            }
            let id = dev.input_id();
            let (vendor, product) = (id.vendor(), id.product());

            // Compute the SDL joystick GUID for diagnostics: an operator can
            // copy it from the log straight into a `gamecontrollerdb.txt` entry
            // to teach the daemon a controller it didn't recognize.
            let guid = super::guid_to_string(&super::sdl_guid(
                id.bus_type().0,
                vendor,
                product,
                id.version(),
            ));
            tracing::debug!(
                "gamepad candidate {} guid={guid} vendor={vendor:04x} product={product:04x}",
                dev.name().unwrap_or("unknown"),
            );

            if pinned {
                if Some(vendor) == pin_vendor && Some(product) == pin_product {
                    return Some(make_handle(dev, path));
                }
                continue;
            }

            if db.is_known(vendor, product) {
                return Some(make_handle(dev, path));
            }
            // Unknown vendor/product and not pinned: reject (foreign injector or
            // an unrecognized pad the operator must pin / teach to the DB).
            tracing::debug!(
                "skipping unknown BTN_SOUTH device {} (vendor={vendor:04x} product={product:04x}): \
                 not in controller DB and not pinned",
                dev.name().unwrap_or("unknown"),
            );
        }
        None
    }

    fn make_handle(dev: Device, path: PathBuf) -> GamepadHandle {
        let name = dev.name().unwrap_or("unknown").to_string();
        let id = dev.input_id();
        let wire_id = super::derive_wire_id(
            dev.unique_name(),
            dev.physical_path(),
            id.vendor(),
            id.product(),
            &path.to_string_lossy(),
        );
        GamepadHandle {
            device: dev,
            name,
            path,
            wire_id,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xbox360_guid_matches_known_sdl_string() {
        // Wired Xbox 360 controller on Linux (bus=USB(3), vendor=0x045e,
        // product=0x028e, version=0x0110). This is the canonical SDL GUID.
        let guid = sdl_guid(0x0003, 0x045e, 0x028e, 0x0110);
        assert_eq!(guid_to_string(&guid), "030000005e0400008e02000010010000");
    }

    #[test]
    fn guid_roundtrip_extracts_vendor_product() {
        let guid = sdl_guid(0x0003, 0x045e, 0x028e, 0x0110);
        let s = guid_to_string(&guid);
        let back = parse_guid(&s).unwrap();
        assert_eq!(vendor_of(&back), 0x045e);
        assert_eq!(product_of(&back), 0x028e);
    }

    #[test]
    fn db_parse_and_match() {
        let db_text = "\
# comment line
030000005e0400008e02000010010000,Xbox 360 Controller,a:b0,b:b1,platform:Linux,
030000004c050000c405000011810000,PS4 Controller,a:b1,platform:Linux,
00000000000000000000000000000000,Zero vendor (name-encoded),platform:Linux,
";
        let db = ControllerDb::parse(db_text);
        assert_eq!(db.len(), 2); // zero-vendor entry skipped
        assert!(db.is_known(0x045e, 0x028e)); // Xbox 360
        assert!(db.is_known(0x054c, 0x05c4)); // PS4 (vendor 4c05 LE -> 0x054c)
        assert!(!db.is_known(0x1234, 0x5678));
    }

    #[test]
    fn bundled_db_loads() {
        let db = ControllerDb::parse(BUNDLED_DB);
        // Baseline should contain the common Xbox 360 pad.
        assert!(db.is_known(0x045e, 0x028e));
    }

    #[test]
    fn load_db_includes_baseline() {
        // No env override set -> just the bundled baseline.
        std::env::remove_var("GAME_SHELL_GAMECONTROLLERDB");
        let db = load_db();
        assert!(db.is_known(0x045e, 0x028e));
        assert!(db.is_known(0x054c, 0x09cc)); // DualShock 4 v2
    }

    #[test]
    fn virtual_registry_tracks_ownership() {
        let mut reg = VirtualRegistry::new();
        assert!(reg.is_empty());
        assert!(!reg.owns(7));

        reg.register(7);
        reg.register(9);
        assert_eq!(reg.len(), 2);
        assert!(reg.owns(7));
        assert!(reg.owns(9));
        assert!(!reg.owns(8));

        // Idempotent registration.
        reg.register(7);
        assert_eq!(reg.len(), 2);

        reg.unregister(7);
        assert!(!reg.owns(7));
        assert!(reg.owns(9));
        assert_eq!(reg.len(), 1);
    }

    #[test]
    fn wire_id_prefers_uniq() {
        // uniq present -> used regardless of phys/path.
        let id = derive_wire_id(
            Some("e4:17:d8:01:02:03"),
            Some("usb-0000:00:14.0-1/input0"),
            0x045e,
            0x028e,
            "/dev/input/event5",
        );
        assert_eq!(id, "uniq:e4:17:d8:01:02:03");
    }

    #[test]
    fn wire_id_falls_back_to_phys_then_vp() {
        // No uniq -> phys.
        let id = derive_wire_id(
            None,
            Some("usb-0000:00:14.0-1/input0"),
            0x045e,
            0x028e,
            "/dev/input/event5",
        );
        assert_eq!(id, "phys:usb-0000:00:14.0-1/input0");

        // Neither uniq nor phys -> vendor:product:path.
        let id = derive_wire_id(None, None, 0x045e, 0x028e, "/dev/input/event5");
        assert_eq!(id, "vp:045e:028e:/dev/input/event5");
    }

    #[test]
    fn wire_id_treats_empty_strings_as_missing() {
        // The kernel reports "" (not None) for some devices' uniq/phys.
        let id = derive_wire_id(Some(""), Some("  "), 0x054c, 0x09cc, "/dev/input/event3");
        assert_eq!(id, "vp:054c:09cc:/dev/input/event3");

        // Empty uniq but real phys -> phys wins.
        let id = derive_wire_id(
            Some(""),
            Some("usb-1/input0"),
            0x054c,
            0x09cc,
            "/dev/input/event3",
        );
        assert_eq!(id, "phys:usb-1/input0");
    }

    #[test]
    fn wire_id_distinguishes_identical_pads_on_different_ports() {
        // Two identical pads with no uniq/phys must not collide: the path keeps
        // them distinct so the fleet allocates separate slots.
        let a = derive_wire_id(None, None, 0x045e, 0x028e, "/dev/input/event5");
        let b = derive_wire_id(None, None, 0x045e, 0x028e, "/dev/input/event7");
        assert_ne!(a, b);
    }

    #[test]
    fn id_env_parsing() {
        std::env::set_var("TEST_GP_VENDOR_HEX", "0x045e");
        std::env::set_var("TEST_GP_VENDOR_DEC", "1118");
        assert_eq!(parse_id_env("TEST_GP_VENDOR_HEX"), Some(0x045e));
        assert_eq!(parse_id_env("TEST_GP_VENDOR_DEC"), Some(1118));
        assert_eq!(parse_id_env("TEST_GP_UNSET_XYZ"), None);
        std::env::remove_var("TEST_GP_VENDOR_HEX");
        std::env::remove_var("TEST_GP_VENDOR_DEC");
    }
}
