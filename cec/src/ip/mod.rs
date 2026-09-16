//! The IP leg — **a capability complement on the cold path, and a failover on
//! the warm path**.
//!
//! Two wires and nothing else: a Wake-on-LAN magic packet ([`wol`]) and a
//! Denon/Marantz ASCII telnet session ([`avr`]). Both are write-only; neither
//! reads any state back into [`crate::state::AvState`], and the reasons are with
//! each module.
//!
//! # The model, and where §13 Q7's wording is too narrow
//!
//! Q7 describes IP as used "when the CEC bus is unavailable or the adapter has
//! wedged". That is the **warm path** — the failover [`crate::failover`]
//! decides. It is not the whole story, because two things have no CEC
//! expression at all:
//!
//! 1. **AVR Zone 2 is not CEC-addressable.** `Z2OFF` has no equivalent, so when
//!    Zone 2 is configured the telnet leg runs on *every* standby, with a
//!    perfectly healthy CEC bus.
//! 2. **A fully-off television cannot be cold-woken by CEC.** `<Image View On>`
//!    reaches nothing at mains standby; that needs a magic packet — as does the
//!    receiver, when its network-control-in-standby setting is off.
//!
//! So the IP steps run **before** the CEC steps on both `wake` and `standby`,
//! unconditionally, exactly as jedwards1230/tv-shell#191 sequenced them, and
//! [`IpRole`] says only whether CEC is expected to follow.
//!
//! Standby is ordered the same way for a second reason: a `Z2OFF` has to reach a
//! receiver that is still **awake**. Sending the CEC `<Standby>` first would put
//! the AVR to sleep — and, with network control in standby off, take its NIC
//! down — before the one command CEC cannot express had been sent.
//!
//! # Nothing here dials anything in a test
//!
//! All I/O is behind [`IpWire`], the way the bus is behind
//! [`crate::volume::VolumeBus`] and [`crate::kernel::ops::Transmitter`]. The
//! tests drive a recording fake. **No test in this crate opens a socket to an
//! AVR or puts a packet on a network** — the endpoints are live equipment in a
//! living room, and the receiver's telnet port accepts exactly one client.

pub mod avr;
pub mod wol;

use std::net::SocketAddr;
use std::time::Duration;

use crate::action::Action;
use crate::ip::avr::{Avr, AvrEndpoint};
use crate::ip::wol::Mac;

/// How long any single network step may take — connect, write or send.
///
/// Bounded, and short: this runs inside a client's request, and a receiver that
/// is unplugged must cost a reply that says so rather than a caller that waits.
/// The same rule the panel applies from the other side of the socket.
pub const NET_TIMEOUT: Duration = Duration::from_secs(3);

/// How many magic packets one wake sends.
///
/// Two, as jedwards1230/tv-shell#191 did and as the shell script before it did:
/// a fully-off television can miss the first. Nothing acknowledges a magic
/// packet, so this is the only redundancy available.
pub const WOL_PACKET_REPEATS: usize = 2;

/// Whether CEC is expected to do the rest of the job.
///
/// This is the **only** thing the warm-path failover decision changes about the
/// IP leg. The cold-path steps run either way.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IpRole {
    /// CEC is authoritative and will follow. The IP steps cover only what CEC
    /// cannot express.
    Complement,
    /// CEC is not authoritative. The IP leg is the whole action.
    Authority,
}

/// Where to send a magic packet, and to whom.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WolTarget {
    pub mac: Mac,
    /// The broadcast destination. Numeric `addr:port`, never a name — a magic
    /// packet is a broadcast, and a DNS name would resolve to one host.
    pub broadcast: SocketAddr,
}

/// The IP leg as configured. Either half may be absent; both usually are.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct IpConfig {
    pub avr: Option<Avr>,
    pub tv_wol: Option<WolTarget>,
}

impl IpConfig {
    /// Whether there is an IP leg at all.
    ///
    /// **This is what makes failing over to IP possible.** With nothing
    /// configured there is nowhere to fail over *to*, so [`crate::failover`]
    /// keeps reporting `cec` as active and names the degradation in its reason,
    /// rather than announcing a backend that does not exist.
    #[must_use]
    pub const fn is_configured(&self) -> bool {
        self.avr.is_some() || self.tv_wol.is_some()
    }
}

/// The IP steps of one action, in order. Empty for every verb that has no IP
/// expression.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IpPlan {
    pub role: IpRole,
    /// The magic packet to send, if any. Sent [`WOL_PACKET_REPEATS`] times.
    pub wol: Option<WolTarget>,
    /// Telnet control lines, in order, over **one** connection.
    pub commands: Vec<String>,
    /// Where those lines go. `None` when there are none.
    pub endpoint: Option<AvrEndpoint>,
}

impl IpPlan {
    /// Nothing to do.
    ///
    /// Not `const`: `Vec::is_empty` is only const from Rust 1.87 and this crate
    /// records a 1.85 floor.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.wol.is_none() && self.commands.is_empty()
    }
}

/// **PURE**: what the IP leg does for one action, given the config and the role.
///
/// The exhaustive match is the point: an action added to [`Action`] without a
/// decision here does not compile.
#[must_use]
pub fn plan_for(action: Action, config: &IpConfig, role: IpRole) -> IpPlan {
    let (wol, commands) = match action {
        // Cold wake: the magic packet first (the television may be at mains
        // standby, where CEC reaches nothing), then the receiver.
        Action::Wake => (
            config.tv_wol,
            config
                .avr
                .as_ref()
                .map(|a| a.wake_commands(role))
                .unwrap_or_default(),
        ),
        // Zone 2, every time. No magic packet: waking a device is not part of
        // switching it off.
        Action::Standby => (
            None,
            config
                .avr
                .as_ref()
                .map(Avr::standby_commands)
                .unwrap_or_default(),
        ),
        // Input switching and volume have no IP expression here. `SI<input>`
        // selects the receiver's source by a site-specific code, not by the
        // physical address these verbs name, so it cannot answer `input-select`;
        // and the receiver's volume is reachable over CEC whenever the receiver
        // is reachable at all.
        Action::InputClaim | Action::InputRelease | Action::InputSelect(_) => (None, Vec::new()),
    };
    IpPlan {
        role,
        wol,
        endpoint: config.avr.as_ref().map(|a| a.endpoint.clone()),
        commands,
    }
}

/// What a standby could NOT achieve over IP, if anything.
///
/// `Some(why)` when the IP leg is the authority and nothing in the plan powers
/// the main zone down. A `standby` that answered `ok` while the television
/// stayed on is exactly the dishonesty the rest of this crate refuses; the
/// caller turns this into an `error:` naming the cause and the setting that
/// would fix it.
#[must_use]
pub fn standby_shortfall(config: &IpConfig, role: IpRole) -> Option<String> {
    if role != IpRole::Authority {
        return None;
    }
    match &config.avr {
        Some(a) if a.main_power => None,
        Some(_) => Some(
            "the CEC backend is not authoritative and [avr].main_power is off, so nothing \
             powered the main zone down"
                .to_string(),
        ),
        None => Some(
            "the CEC backend is not authoritative and no [avr] is configured, so nothing \
             could be powered down at all"
                .to_string(),
        ),
    }
}

/// Everything this crate does on a network.
///
/// One method per wire, and both take a **whole** unit of work: the telnet
/// method takes the entire command list because the receiver's control port
/// accepts one client, so a connection is opened, drained of one list, and
/// closed. There is no "open" / "send" / "close" triple a caller could
/// interleave.
#[async_trait::async_trait]
pub trait IpWire: Send + Sync {
    /// Broadcast one magic packet.
    async fn wake_on_lan(&self, packet: &[u8], to: SocketAddr) -> Result<(), String>;

    /// Open one telnet connection, write every line in order, close.
    async fn avr_commands(&self, endpoint: &AvrEndpoint, commands: &[String])
        -> Result<(), String>;
}

/// What the IP leg actually did.
///
/// **A sent magic packet is not a woken television.** Nothing acknowledges one,
/// so `wol_packets` counts what left this host and claims nothing about what
/// received it. The telnet half is different: a write that completed means a
/// connection was accepted, which is real evidence about the receiver.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct IpReport {
    /// Magic packets that left this host. Acknowledged by nobody.
    pub wol_packets: usize,
    /// Control lines the receiver's socket accepted.
    pub commands: Vec<String>,
    /// Every failure, in order. Empty means every configured step completed.
    pub errors: Vec<String>,
}

impl IpReport {
    /// Whether every configured step completed.
    #[must_use]
    pub fn ok(&self) -> bool {
        self.errors.is_empty()
    }

    /// Whether anything was attempted at all.
    #[must_use]
    pub fn attempted(&self) -> bool {
        self.wol_packets > 0 || !self.commands.is_empty() || !self.errors.is_empty()
    }

    /// The failures as one line, for a reply or a log.
    #[must_use]
    pub fn why(&self) -> String {
        self.errors.join("; ")
    }
}

/// Run an IP plan over `wire`.
///
/// Ordered: the magic packet first, then the telnet lines — the wake sequence
/// #191 used, and the order a cold chain needs (the receiver may itself have to
/// be woken before it will accept a connection).
///
/// **Every step is attempted even after an earlier one fails.** A television
/// that is already on and a receiver that is unplugged are independent facts,
/// and the Zone-2 command is worth sending whether or not the magic packet
/// went out.
pub async fn execute(wire: &dyn IpWire, plan: &IpPlan) -> IpReport {
    let mut report = IpReport::default();
    if let Some(target) = plan.wol {
        let packet = wol::magic_packet(target.mac);
        for _ in 0..WOL_PACKET_REPEATS {
            match wire.wake_on_lan(&packet, target.broadcast).await {
                Ok(()) => report.wol_packets += 1,
                Err(e) => {
                    report
                        .errors
                        .push(format!("wake-on-lan to {}: {e}", target.broadcast));
                    // One failed send is enough: the second would fail the same
                    // way and would double the message.
                    break;
                }
            }
        }
    }
    if !plan.commands.is_empty() {
        let Some(endpoint) = &plan.endpoint else {
            // Unreachable from `plan_for`, which only produces commands from a
            // configured receiver — and reported rather than asserted, because a
            // panic here would take a long-running daemon down.
            report
                .errors
                .push("telnet commands were planned with no receiver endpoint".to_string());
            return report;
        };
        match wire.avr_commands(endpoint, &plan.commands).await {
            Ok(()) => report.commands = plan.commands.clone(),
            Err(e) => report.errors.push(format!("avr {endpoint}: {e}")),
        }
    }
    report
}

/// **PURE**: what an action carried out over the IP leg alone amounts to.
///
/// Only ever called on the [`IpRole::Authority`] path — when CEC is
/// authoritative its outcome is the action's outcome, and a receiver that did
/// not answer its telnet port is a warning in the journal rather than a failed
/// `wake`.
///
/// `why_ip` is [`crate::failover::Failover::reason`], so every reply from this
/// path names the observation that put the daemon on it.
#[must_use]
pub fn judge(
    action: Action,
    config: &IpConfig,
    report: &IpReport,
    why_ip: &str,
) -> crate::backend::ActionOutcome {
    use crate::backend::ActionOutcome;

    if !report.ok() {
        return ActionOutcome::Failed(format!(
            "the IP leg is carrying this action ({why_ip}) and it failed: {}",
            report.why()
        ));
    }
    if !report.attempted() {
        return ActionOutcome::Failed(format!(
            "the IP leg is carrying this action ({why_ip}) and it has no step for {}",
            match action {
                Action::Wake => "wake",
                Action::Standby => "standby",
                Action::InputClaim => "input-claim",
                Action::InputRelease => "input-release",
                Action::InputSelect(_) => "input-select",
            }
        ));
    }
    if action == Action::Standby {
        if let Some(shortfall) = standby_shortfall(config, IpRole::Authority) {
            // The Zone-2 command really did go out, and the main room really is
            // still on. Reporting `ok` here would be the `cec-health` failure
            // shape one layer up: a success for something that did not happen.
            return ActionOutcome::Failed(format!(
                "{shortfall} ({why_ip}). Sent: {}",
                if report.commands.is_empty() {
                    "nothing".to_string()
                } else {
                    report.commands.join(" ")
                }
            ));
        }
    }
    if action == Action::Wake && report.commands.is_empty() {
        // A magic packet and nothing else. Nothing acknowledges one, so this is
        // reported as what it is rather than as a woken television.
        tracing::info!(
            "wake over the IP leg sent {} magic packet(s) and nothing else; nothing \
             acknowledges a magic packet, so this is not evidence the television woke",
            report.wol_packets
        );
    }
    ActionOutcome::Done
}

/// The answer to a volume verb while the IP leg is carrying actions.
///
/// **A failure, not a refusal.** A refusal means the daemon deliberately
/// declined something it could have done; this is a thing it cannot do at all
/// from where it is. The Denon control protocol does have volume commands, but
/// they are absolute levels on a receiver-specific scale, and this daemon has
/// never read that scale — it reads CEC's 0–100 `<Report Audio Status>`. Mapping
/// one onto the other unmeasured would be inventing a number, which is the one
/// thing this crate refuses to do everywhere else.
#[must_use]
pub fn volume_unreachable(why_ip: &str) -> crate::backend::ActionOutcome {
    crate::backend::ActionOutcome::Failed(format!(
        "the IP leg is carrying actions ({why_ip}); the receiver's volume is only reachable \
         over CEC, so there is nothing to send"
    ))
}

/// The real wires: a broadcast UDP socket and a TCP control connection.
///
/// Both bounded by [`NET_TIMEOUT`]. Nothing here may await indefinitely — this
/// runs inside a client's request on the same daemon whose isolation depends on
/// bounded waits.
#[derive(Debug, Default, Clone, Copy)]
pub struct NetWire;

#[async_trait::async_trait]
impl IpWire for NetWire {
    async fn wake_on_lan(&self, packet: &[u8], to: SocketAddr) -> Result<(), String> {
        use tokio::net::UdpSocket;
        // The unspecified address, so the kernel picks the egress interface.
        let socket = UdpSocket::bind((std::net::Ipv4Addr::UNSPECIFIED, 0))
            .await
            .map_err(|e| format!("binding a UDP socket: {e}"))?;
        socket
            .set_broadcast(true)
            .map_err(|e| format!("enabling SO_BROADCAST: {e}"))?;
        match tokio::time::timeout(NET_TIMEOUT, socket.send_to(packet, to)).await {
            Ok(Ok(_)) => Ok(()),
            Ok(Err(e)) => Err(e.to_string()),
            Err(_) => Err(format!("timed out after {NET_TIMEOUT:?}")),
        }
    }

    async fn avr_commands(
        &self,
        endpoint: &AvrEndpoint,
        commands: &[String],
    ) -> Result<(), String> {
        use tokio::io::AsyncWriteExt;
        use tokio::net::TcpStream;

        // Resolved here rather than at config time: a name that does not resolve
        // must not stop this daemon from starting.
        let addrs: Vec<SocketAddr> = match tokio::time::timeout(
            NET_TIMEOUT,
            tokio::net::lookup_host((endpoint.host.as_str(), endpoint.port)),
        )
        .await
        {
            Ok(Ok(iter)) => iter.collect(),
            Ok(Err(e)) => return Err(format!("resolving {endpoint}: {e}")),
            Err(_) => return Err(format!("resolving {endpoint} timed out")),
        };
        let addr = addrs
            .first()
            .copied()
            .ok_or_else(|| format!("{endpoint} resolved to no address"))?;

        let mut stream = match tokio::time::timeout(NET_TIMEOUT, TcpStream::connect(addr)).await {
            Ok(Ok(s)) => s,
            Ok(Err(e)) => {
                return Err(format!(
                    "connecting: {e} (a receiver in standby powers its NIC down unless \
                     network control in standby is enabled in its menu)"
                ))
            }
            Err(_) => return Err(format!("connecting timed out after {NET_TIMEOUT:?}")),
        };
        for command in commands {
            let line = format!("{command}{}", avr::TERMINATOR);
            match tokio::time::timeout(NET_TIMEOUT, stream.write_all(line.as_bytes())).await {
                Ok(Ok(())) => {}
                Ok(Err(e)) => return Err(format!("writing {command:?}: {e}")),
                Err(_) => return Err(format!("writing {command:?} timed out")),
            }
        }
        // Best-effort: the lines are already written, and a receiver that closes
        // on us after acting is not a failure of the action.
        let _ = tokio::time::timeout(NET_TIMEOUT, stream.flush()).await;
        Ok(())
    }
}

#[cfg(test)]
pub(crate) mod testing {
    use super::*;
    use std::sync::Mutex;

    /// A recording stand-in for the network. **No socket, no packet, no
    /// connection** — the AVR and the television are live equipment in somebody's
    /// living room, and the receiver's control port accepts one client.
    #[derive(Default)]
    pub(crate) struct FakeWire {
        pub(crate) packets: Mutex<Vec<(Vec<u8>, SocketAddr)>>,
        pub(crate) sessions: Mutex<Vec<(AvrEndpoint, Vec<String>)>>,
        pub(crate) fail_wol: Mutex<bool>,
        pub(crate) fail_avr: Mutex<bool>,
    }

    impl FakeWire {
        pub(crate) fn sessions(&self) -> Vec<(AvrEndpoint, Vec<String>)> {
            self.sessions.lock().unwrap().clone()
        }

        pub(crate) fn packet_count(&self) -> usize {
            self.packets.lock().unwrap().len()
        }
    }

    #[async_trait::async_trait]
    impl IpWire for FakeWire {
        async fn wake_on_lan(&self, packet: &[u8], to: SocketAddr) -> Result<(), String> {
            if *self.fail_wol.lock().unwrap() {
                return Err("no route to the broadcast address".to_string());
            }
            self.packets.lock().unwrap().push((packet.to_vec(), to));
            Ok(())
        }

        async fn avr_commands(
            &self,
            endpoint: &AvrEndpoint,
            commands: &[String],
        ) -> Result<(), String> {
            if *self.fail_avr.lock().unwrap() {
                return Err("connection refused".to_string());
            }
            self.sessions
                .lock()
                .unwrap()
                .push((endpoint.clone(), commands.to_vec()));
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::testing::FakeWire;
    use super::*;
    use crate::state::PhysAddr;

    fn config() -> IpConfig {
        IpConfig {
            avr: Some(Avr {
                endpoint: AvrEndpoint {
                    host: "192.0.2.10".to_string(),
                    port: avr::DEFAULT_PORT,
                },
                input: Some("GAME".to_string()),
                main_power: false,
                zone2_off: true,
            }),
            tv_wol: Some(WolTarget {
                mac: Mac::parse("aa:bb:cc:dd:ee:ff").unwrap(),
                broadcast: "255.255.255.255:9".parse().unwrap(),
            }),
        }
    }

    #[test]
    fn an_unconfigured_ip_leg_plans_nothing_for_any_action() {
        let empty = IpConfig::default();
        assert!(!empty.is_configured());
        for action in [
            Action::Wake,
            Action::Standby,
            Action::InputClaim,
            Action::InputRelease,
            Action::InputSelect("1.0.0.0".parse::<PhysAddr>().unwrap()),
        ] {
            for role in [IpRole::Complement, IpRole::Authority] {
                assert!(
                    plan_for(action, &empty, role).is_empty(),
                    "{action:?} {role:?}"
                );
            }
        }
    }

    /// **The rule: the cold-path steps are planned with a perfectly healthy CEC
    /// bus.** A wake carries the magic packet; a standby carries Zone 2.
    #[test]
    fn the_cold_path_steps_are_planned_even_when_cec_is_authoritative() {
        let plan = plan_for(Action::Wake, &config(), IpRole::Complement);
        assert!(plan.wol.is_some(), "a cold television needs a magic packet");
        assert_eq!(plan.commands, vec!["SIGAME".to_string()]);

        let plan = plan_for(Action::Standby, &config(), IpRole::Complement);
        assert_eq!(plan.commands, vec!["Z2OFF".to_string()]);
        assert!(plan.wol.is_none(), "switching off does not wake anything");
    }

    /// Input switching and volume have no IP expression, in either role.
    #[test]
    fn the_input_and_volume_verbs_have_no_ip_leg() {
        for action in [
            Action::InputClaim,
            Action::InputRelease,
            Action::InputSelect("1.0.0.0".parse::<PhysAddr>().unwrap()),
        ] {
            for role in [IpRole::Complement, IpRole::Authority] {
                assert!(
                    plan_for(action, &config(), role).is_empty(),
                    "{action:?} {role:?}"
                );
            }
        }
    }

    /// **The rule: one telnet connection carries the whole command list, in
    /// order** — the receiver's control port accepts exactly one client.
    #[tokio::test]
    async fn one_session_carries_every_command_in_order() {
        let wire = FakeWire::default();
        let mut cfg = config();
        if let Some(a) = cfg.avr.as_mut() {
            a.main_power = true;
        }
        let plan = plan_for(Action::Standby, &cfg, IpRole::Complement);
        let report = execute(&wire, &plan).await;

        assert!(report.ok(), "{:?}", report.errors);
        let sessions = wire.sessions();
        assert_eq!(sessions.len(), 1, "one connection, not one per command");
        assert_eq!(
            sessions[0].1,
            vec!["Z2OFF".to_string(), "PWSTANDBY".to_string()]
        );
        assert_eq!(sessions[0].0.host, "192.0.2.10");
    }

    /// The magic packet goes out twice, and it is the canonical 102 bytes.
    #[tokio::test]
    async fn a_wake_broadcasts_the_magic_packet_twice() {
        let wire = FakeWire::default();
        let plan = plan_for(Action::Wake, &config(), IpRole::Authority);
        let report = execute(&wire, &plan).await;

        assert!(report.ok());
        assert_eq!(report.wol_packets, WOL_PACKET_REPEATS);
        assert_eq!(wire.packet_count(), 2);
        let packets = wire.packets.lock().unwrap();
        assert_eq!(packets[0].0.len(), wol::MAGIC_PACKET_LEN);
        assert_eq!(packets[0].1.port(), 9);
        // An authority wake powers the main zone without an opt-in.
        assert_eq!(
            wire.sessions()[0].1,
            vec!["PWON".to_string(), "SIGAME".to_string()]
        );
    }

    /// **The rule: an earlier failure does not cancel the later steps.** A
    /// television that is already on and a receiver that is unplugged are
    /// independent facts.
    #[tokio::test]
    async fn a_failed_magic_packet_does_not_cancel_the_telnet_half() {
        let wire = FakeWire::default();
        *wire.fail_wol.lock().unwrap() = true;
        let plan = plan_for(Action::Wake, &config(), IpRole::Complement);
        let report = execute(&wire, &plan).await;

        assert!(!report.ok());
        assert_eq!(report.wol_packets, 0);
        assert!(report.why().contains("wake-on-lan"), "{report:?}");
        assert_eq!(wire.sessions().len(), 1, "the receiver was still told");
        assert_eq!(report.commands, vec!["SIGAME".to_string()]);
    }

    /// A receiver that refuses the connection is reported, not swallowed — the
    /// standby-in-standby NIC precondition is exactly this case.
    #[tokio::test]
    async fn a_refused_receiver_is_reported() {
        let wire = FakeWire::default();
        *wire.fail_avr.lock().unwrap() = true;
        let plan = plan_for(Action::Standby, &config(), IpRole::Complement);
        let report = execute(&wire, &plan).await;
        assert!(!report.ok());
        assert!(report.why().contains("connection refused"), "{report:?}");
        assert!(report.commands.is_empty());
    }

    /// **The rule: an IP standby that cannot power the main zone down says so.**
    ///
    /// Reachable exactly when the adapter has failed over and the operator has
    /// not opted into telnet main-zone power — the common configuration, since
    /// `main_power` defaults off.
    #[test]
    fn an_authority_standby_that_cannot_power_the_main_zone_down_names_the_shortfall() {
        let cfg = config();
        assert_eq!(standby_shortfall(&cfg, IpRole::Complement), None);
        let why = standby_shortfall(&cfg, IpRole::Authority).expect("a shortfall");
        assert!(why.contains("main_power"), "{why}");

        let mut opted_in = config();
        if let Some(a) = opted_in.avr.as_mut() {
            a.main_power = true;
        }
        assert_eq!(standby_shortfall(&opted_in, IpRole::Authority), None);

        let none = IpConfig::default();
        assert!(standby_shortfall(&none, IpRole::Authority)
            .expect("a shortfall")
            .contains("no [avr]"));
    }

    /// **The rule: an IP-carried action that failed is an `error:`, and one
    /// that could not achieve its purpose is too.**
    #[tokio::test]
    async fn an_ip_carried_action_is_judged_on_what_it_achieved() {
        use crate::backend::ActionOutcome;

        // A wake that got its packets and its PWON out.
        let wire = FakeWire::default();
        let plan = plan_for(Action::Wake, &config(), IpRole::Authority);
        let report = execute(&wire, &plan).await;
        assert_eq!(
            judge(Action::Wake, &config(), &report, "the adapter is degraded"),
            ActionOutcome::Done
        );

        // A standby with `main_power` off: Zone 2 went off, the main room did
        // not, and that is an error naming the setting.
        let wire = FakeWire::default();
        let plan = plan_for(Action::Standby, &config(), IpRole::Authority);
        let report = execute(&wire, &plan).await;
        let ActionOutcome::Failed(why) = judge(
            Action::Standby,
            &config(),
            &report,
            "the adapter is degraded",
        ) else {
            panic!("a standby that did not power the main zone down is not a success");
        };
        assert!(why.contains("main_power"), "{why}");
        assert!(
            why.contains("Z2OFF"),
            "the reply must say what DID go out: {why}"
        );
        assert!(why.contains("the adapter is degraded"), "{why}");

        // …and with the opt-in, the same standby is a success.
        let mut opted_in = config();
        if let Some(a) = opted_in.avr.as_mut() {
            a.main_power = true;
        }
        let wire = FakeWire::default();
        let plan = plan_for(Action::Standby, &opted_in, IpRole::Authority);
        let report = execute(&wire, &plan).await;
        assert_eq!(
            judge(Action::Standby, &opted_in, &report, "degraded"),
            ActionOutcome::Done
        );

        // A wire failure is a failure, and it names the leg.
        let wire = FakeWire::default();
        *wire.fail_avr.lock().unwrap() = true;
        let plan = plan_for(Action::Standby, &opted_in, IpRole::Authority);
        let report = execute(&wire, &plan).await;
        let ActionOutcome::Failed(why) = judge(Action::Standby, &opted_in, &report, "degraded")
        else {
            panic!("a refused receiver is not a success");
        };
        assert!(why.contains("connection refused"), "{why}");
    }

    /// A verb with no IP expression, on the IP path, fails naming the verb —
    /// it does not quietly answer `ok`.
    #[tokio::test]
    async fn a_verb_with_no_ip_step_fails_rather_than_pretending() {
        use crate::backend::ActionOutcome;
        let wire = FakeWire::default();
        let plan = plan_for(Action::InputClaim, &config(), IpRole::Authority);
        let report = execute(&wire, &plan).await;
        let ActionOutcome::Failed(why) = judge(Action::InputClaim, &config(), &report, "degraded")
        else {
            panic!("input-claim has no IP expression");
        };
        assert!(why.contains("input-claim"), "{why}");
    }

    #[test]
    fn the_network_timeout_is_bounded() {
        assert!(NET_TIMEOUT <= Duration::from_secs(5));
    }
}
