//! Denon/Marantz ASCII telnet — **the command vocabulary, and it is pure**.
//!
//! A port of the never-merged jedwards1230/tv-shell#191's `daemon/src/av_net.rs`
//! onto this crate's typed TOML config. **Ported, not copied**: #191 was
//! env-var-driven (`GAME_SHELL_AVR_HOST` … via `AvNetConfig::from_env`), and v2
//! configuration is a typed `cec.toml` with `deny_unknown_fields`, so a typo is
//! a startup failure rather than a silently-inert feature. V2_DESIGN §8 already
//! asks for exactly that ("port `av_net.rs` … to typed config").
//!
//! **#191 was never hardware-tested, by its own admission** — "No on-device test
//! of the actual WoL/telnet against the real TV/AVR". Its nine tests are shape
//! coverage: they pin the command strings and the config parsing, and they are
//! not evidence that this deployment's receiver answers to them. Nothing here has been
//! run against a receiver either; see the crate README's on-box checklist.
//!
//! # Two site preconditions, carried over because they are still true
//!
//! 1. **The AVR's telnet port accepts exactly one client** (V2_DESIGN §8). So
//!    every use of this module opens one connection, writes every line of one
//!    command list in order, and closes — see [`crate::ip::IpWire`]. There is no
//!    persistent session, no connection pool, and no second concurrent caller.
//! 2. **The receiver powers its NIC down in standby** unless its menu enables
//!    network control in standby. With that setting off, the telnet leg can turn
//!    the AVR *off* and cannot turn it back *on* — which is why the wake path
//!    also carries Wake-on-LAN, and why a failed connect on wake is reported
//!    rather than swallowed.
//!
//! # What CEC cannot do at all, and this can
//!
//! §13 Q7 describes the IP leg as used "when the CEC bus is unavailable or the
//! adapter has wedged" — purely a failover. That framing is too narrow, and
//! #191's problem statement is where the two gaps are written down:
//!
//! * **Zone 2 is not CEC-addressable.** `Z2OFF` has no CEC equivalent at all, so
//!   if Zone 2 is wanted, telnet runs on **every** standby — healthy CEC or not.
//!   That is why [`Avr::standby_commands`] does not take a role.
//! * **A fully-off television cannot be cold-woken by CEC**, and neither can an
//!   AVR whose network control in standby is off. That is the WoL half, in
//!   [`crate::ip::wol`].
//!
//! So the IP leg is a **capability complement on the cold path** and a
//! **failover on the warm path**. [`crate::failover`] decides only the second.

use crate::ip::IpRole;

/// The default Denon/Marantz control port. They speak ASCII over TCP/23.
pub const DEFAULT_PORT: u16 = 23;

/// The line terminator the protocol uses. **Carriage return, not CRLF** — a
/// Denon control line ends with a bare `\r`.
pub const TERMINATOR: char = '\r';

/// Zone 2 off. The one command with no CEC equivalent whatsoever.
pub const ZONE2_OFF: &str = "Z2OFF";
/// Main-zone power on.
pub const MAIN_ON: &str = "PWON";
/// Main-zone standby.
pub const MAIN_STANDBY: &str = "PWSTANDBY";

/// Where the receiver is. Resolved at send time, never at parse time: a DNS
/// lookup in `validate()` would make startup depend on the network, and this
/// daemon has to start on a box whose AVR is unplugged.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AvrEndpoint {
    pub host: String,
    pub port: u16,
}

impl std::fmt::Display for AvrEndpoint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}", self.host, self.port)
    }
}

/// A configured receiver, in this crate's own terms.
///
/// Built from `cec.toml` by [`crate::config::CecConfig::ip`], which is where a
/// malformed value becomes a startup error naming the key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Avr {
    pub endpoint: AvrEndpoint,
    /// The source code to select on wake (`SI<input>`, e.g. `GAME`). `None`
    /// skips the input switch — on the CEC path the `<Active Source>` claim
    /// already selects our input.
    pub input: Option<String>,
    /// Drive the receiver's **main zone** power over telnet.
    ///
    /// Off by default, and it stays an explicit opt-in even when the IP leg is
    /// the authority: powering the main zone down is the action that can black
    /// out a television somebody is watching, and this daemon does not grant
    /// itself that authority because its own adapter stopped answering. What it
    /// does instead is say so — see [`crate::ip::standby_shortfall`].
    pub main_power: bool,
    /// Send [`ZONE2_OFF`] on every standby. On by default: it is the whole
    /// reason this leg exists on the cold path, and it cannot black out the main
    /// room.
    pub zone2_off: bool,
}

impl Avr {
    /// The telnet lines to send **on wake**, in order.
    ///
    /// The role is load-bearing here and only here:
    ///
    /// * [`IpRole::Complement`] — CEC is authoritative, so `<Image View On>` and
    ///   the `<Active Source>` claim already power the main zone and select our
    ///   input. `PWON` goes only if the operator asked for it.
    /// * [`IpRole::Authority`] — nothing else can power the main zone, so `PWON`
    ///   goes unconditionally. This is a power-**on**, which cannot black
    ///   anything out, so it needs no opt-in the way the standby half does.
    #[must_use]
    pub fn wake_commands(&self, role: IpRole) -> Vec<String> {
        let mut commands = Vec::new();
        if self.main_power || role == IpRole::Authority {
            commands.push(MAIN_ON.to_string());
        }
        if let Some(input) = &self.input {
            commands.push(format!("SI{input}"));
        }
        commands
    }

    /// The telnet lines to send **on standby**, in order.
    ///
    /// **No role parameter, deliberately.** Zone 2 has no CEC equivalent, so
    /// this list is identical whether CEC is healthy or not — the complement
    /// half of the model, rather than a fallback.
    ///
    /// **`Z2OFF` goes FIRST, and that is a correction to #191**, which ordered
    /// `[PWSTANDBY, Z2OFF]`. A receiver that has just been told to go to standby
    /// may drop the control connection before the second line is read, which
    /// would silently lose the one command this leg exists for. #191 never ran
    /// against hardware, so the order was never observed either way; ordering it
    /// the safe way costs nothing.
    #[must_use]
    pub fn standby_commands(&self) -> Vec<String> {
        let mut commands = Vec::new();
        if self.zone2_off {
            commands.push(ZONE2_OFF.to_string());
        }
        if self.main_power {
            commands.push(MAIN_STANDBY.to_string());
        }
        commands
    }
}

/// Reject a source code that could not be sent as one line.
///
/// The commands are `\r`-terminated and the input code is interpolated into one,
/// so a value carrying a control character would **inject a second command** —
/// a `Z2OFF` or a `PWSTANDBY` an operator never wrote. Restricting the code to
/// the ASCII alphanumerics the protocol actually uses makes that unspellable
/// rather than filtered.
pub fn validate_input_code(code: &str) -> Result<(), String> {
    if code.is_empty() {
        return Err("must not be empty (omit the key instead)".to_string());
    }
    if !code.chars().all(|c| c.is_ascii_alphanumeric()) {
        return Err(format!(
            "{code:?} must be ASCII alphanumeric: it is interpolated into an `SI<input>` \
             control line terminated by a carriage return, so any other character could \
             inject a second command"
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn avr() -> Avr {
        Avr {
            endpoint: AvrEndpoint {
                host: "192.0.2.10".to_string(),
                port: DEFAULT_PORT,
            },
            input: None,
            main_power: false,
            zone2_off: true,
        }
    }

    /// On the warm path with CEC healthy, an unconfigured wake sends nothing:
    /// CEC is already doing it.
    #[test]
    fn a_complement_wake_sends_only_what_was_configured() {
        assert!(avr().wake_commands(IpRole::Complement).is_empty());

        let with_input = Avr {
            input: Some("GAME".to_string()),
            ..avr()
        };
        assert_eq!(
            with_input.wake_commands(IpRole::Complement),
            vec!["SIGAME".to_string()]
        );

        let with_power = Avr {
            main_power: true,
            ..with_input
        };
        assert_eq!(
            with_power.wake_commands(IpRole::Complement),
            vec!["PWON".to_string(), "SIGAME".to_string()],
            "power first, then the input"
        );
    }

    /// **The rule: when the IP leg is the authority, the main zone is powered on
    /// whether or not `main_power` was set — because nothing else can.**
    ///
    /// The asymmetry with standby is the point: a power-on cannot black anything
    /// out, and a power-off can.
    #[test]
    fn an_authority_wake_powers_the_main_zone_without_an_opt_in() {
        assert_eq!(
            avr().wake_commands(IpRole::Authority),
            vec!["PWON".to_string()]
        );
        // …and the standby half does NOT gain PWSTANDBY the same way.
        assert_eq!(avr().standby_commands(), vec!["Z2OFF".to_string()]);
    }

    /// **THE CAPABILITY-COMPLEMENT RULE: `Z2OFF` is sent on every standby,
    /// healthy CEC or not, because CEC cannot address Zone 2 at all.**
    ///
    /// Asserted as the absence of a role parameter's effect: the same list comes
    /// out whatever the warm-path authority is.
    #[test]
    fn zone_two_is_switched_off_on_every_standby() {
        assert_eq!(avr().standby_commands(), vec!["Z2OFF".to_string()]);

        let both = Avr {
            main_power: true,
            ..avr()
        };
        assert_eq!(
            both.standby_commands(),
            vec!["Z2OFF".to_string(), "PWSTANDBY".to_string()],
            "Zone 2 first: a receiver told to stand by may drop the connection"
        );

        // Opting out of Zone 2 leaves only what was asked for.
        let no_zone2 = Avr {
            zone2_off: false,
            ..both
        };
        assert_eq!(no_zone2.standby_commands(), vec!["PWSTANDBY".to_string()]);
        let nothing = Avr {
            main_power: false,
            ..no_zone2
        };
        assert!(nothing.standby_commands().is_empty());
    }

    /// **The rule: a source code that could carry a second command is refused at
    /// startup.**
    ///
    /// The control line is `SI<input>\r`, so a `\r` in the value would append a
    /// command the operator never wrote — `Z2OFF`, or a `PWSTANDBY` that blacks
    /// out the room.
    #[test]
    fn an_input_code_that_could_inject_a_command_is_refused() {
        validate_input_code("GAME").unwrap();
        validate_input_code("MPLAY").unwrap();
        validate_input_code("SAT1").unwrap();
        for bad in [
            "",
            "GAME\rZ2OFF",
            "GAME\nPWSTANDBY",
            "GA ME",
            "GAME;PWSTANDBY",
            "GAME\0",
        ] {
            assert!(validate_input_code(bad).is_err(), "{bad:?} must be refused");
        }
    }

    #[test]
    fn the_endpoint_renders_as_host_and_port() {
        assert_eq!(avr().endpoint.to_string(), "192.0.2.10:23");
        assert_eq!(TERMINATOR, '\r');
    }
}
