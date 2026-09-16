//! Unix-socket IPC server.
//!
//! Same shape as `core/src/ipc.rs`, which is v1's shape: `LinesCodec` with a
//! 4096-byte cap, one tokio task per connection, accept errors logged and never
//! fatal, the socket bound 0600 under a tightened umask so it is private from
//! the instant it exists.
//!
//! The socket path is config/env driven and **must not collide with v1's or the
//! core's** (§11: they share no socket, prefix or unit name), which
//! [`crate::config::socket_path`] guarantees.
//!
//! Backend work is behind the [`AvBackend`] trait for the reason that trait's
//! docs give: it makes the whole request/reply surface testable end-to-end with
//! no `/dev/cecN`.
//!
//! # A note for the callers, not for this file
//!
//! The isolation this daemon gets from the session is topological (see
//! `core/units/tv-shell-v2-cec.service`), and it is only real if **every caller
//! bounds its connect and read timeouts** and renders a degraded state on
//! expiry. A caller that awaits this socket indefinitely is the mechanism by
//! which a wedged CEC backend eventually does reach the television. That rule
//! lives in the callers — the shell's Session QAM and the panel — and cannot be
//! enforced from here.

use std::os::unix::fs::PermissionsExt;
use std::sync::Arc;

use anyhow::{Context, Result};
use futures::{SinkExt, StreamExt};
use tokio::net::{UnixListener, UnixStream};
use tokio_util::codec::{Framed, LinesCodec};

use crate::action::Action;
use crate::backend::{ActionOutcome, AvBackend};
use crate::protocol::{self, Command};

/// Bind the socket (removing any stale file), chmod 0600, and serve forever.
pub async fn serve(sock_path: String, backend: Arc<dyn AvBackend>) -> Result<()> {
    let listener = bind(&sock_path)?;
    tracing::info!("listening on {sock_path}");
    loop {
        match listener.accept().await {
            Ok((stream, _addr)) => {
                let backend = Arc::clone(&backend);
                tokio::spawn(async move {
                    if let Err(e) = handle_client(stream, backend).await {
                        tracing::debug!("client connection ended: {e}");
                    }
                });
            }
            // Never fatal: one bad accept must not take the control surface down.
            Err(e) => tracing::warn!("accept error: {e}"),
        }
    }
}

/// Bind the listener with the v1 umask trick.
fn bind(sock_path: &str) -> Result<UnixListener> {
    // Stale socket: unconditional removal, error ignored — a leftover file from
    // a killed daemon must not stop the next one from starting.
    let _ = std::fs::remove_file(sock_path);
    // Create the socket private from the instant it exists. Binding then
    // chmod'ing leaves a TOCTOU window in which the socket carries umask-
    // dependent (possibly world-accessible) permissions and another local
    // process could connect. A 0o177 umask makes the kernel create the node
    // 0o600 atomically at bind; the explicit set_permissions below is then a
    // belt-and-braces assertion, since a umask can only clear bits.
    //
    // SAFETY: `umask` is a plain process-global setter that cannot fail. It is
    // process-global, though, and NOTHING here excludes other threads — tokio's
    // workers are already running by the time `serve` is called, so a file
    // another thread creates inside this window really does inherit 0o177. Two
    // reasons that is acceptable rather than merely tolerated: the window is a
    // single `bind()` call wide, and it fails CLOSED — the only effect is
    // over-restrictive permissions on an unrelated file, never over-permissive
    // ones, so it cannot leak anything.
    //
    // "Cannot leak anything" is not the same as "is harmless", and the core
    // crate's own suite proved the difference: a *directory* created by another
    // thread inside this window comes out `0o600`, with no `x` bit, and nothing
    // can then be unlinked inside it — which surfaced as an `EACCES` flake in a
    // module that touches no permissions at all. So tests here bind plain paths
    // under `/tmp` and create no scratch directories concurrently.
    let prev_umask = unsafe { libc::umask(0o177) };
    let bind_result = UnixListener::bind(sock_path);
    unsafe {
        libc::umask(prev_umask);
    }
    let listener = bind_result.with_context(|| format!("binding unix socket at {sock_path}"))?;
    std::fs::set_permissions(sock_path, std::fs::Permissions::from_mode(0o600))
        .with_context(|| format!("chmod 0o600 on {sock_path}"))?;
    Ok(listener)
}

/// One command per line, one reply per command, until the client goes away.
async fn handle_client(stream: UnixStream, backend: Arc<dyn AvBackend>) -> Result<()> {
    let mut framed = Framed::new(stream, LinesCodec::new_with_max_length(protocol::MAX_LINE));
    while let Some(line) = framed.next().await {
        let line = line.context("reading command line")?;
        let reply = dispatch(&backend, Command::parse(&line)).await;
        framed.send(reply).await?;
    }
    Ok(())
}

/// Resolve a command to its reply line.
///
/// The `match` is exhaustive over [`Command`] on purpose: the closed vocabulary
/// is enforced by the compiler, so a verb added to the enum without an answer
/// here does not build (§13, 2026-09-07 — no flat string vocabulary).
pub async fn dispatch(backend: &Arc<dyn AvBackend>, cmd: Command) -> String {
    match cmd {
        Command::Ping => protocol::resp_ok(),
        // Answered from a snapshot, on the reactor: it reads no device, so it
        // cannot hang when the device is the thing being diagnosed.
        Command::AvState => protocol::resp_json(&backend.snapshot().await),
        // Answered from the recorded facts, for the same reason: a health verb
        // that had to touch the adapter to answer could not report a wedged
        // adapter. See [`crate::health`].
        Command::AvHealth => protocol::resp_json(&backend.health().await),
        Command::Wake => act(backend, Action::Wake).await,
        Command::Standby => act(backend, Action::Standby).await,
        Command::InputClaim => act(backend, Action::InputClaim).await,
        Command::InputRelease => act(backend, Action::InputRelease).await,
        Command::InputSelect(addr) => act(backend, Action::InputSelect(addr)).await,
        Command::InputSelectUsage => protocol::resp_usage(protocol::INPUT_SELECT_USAGE),
        Command::Volume(action) => outcome(backend.volume(action).await),
        // Deliberately a bus read rather than a cached one — see
        // [`AvBackend::volume_state`]. `av-state` is the verb that must answer
        // when the device is the thing being diagnosed.
        Command::VolumeState => protocol::resp_json(&backend.volume_state().await),
        Command::VolumeUsage => protocol::resp_usage(protocol::VOLUME_USAGE),
        // A read of a decision already made. No adapter, no network: it answers
        // while the adapter is the thing being diagnosed, which is when a caller
        // most wants to know which backend is carrying actions.
        Command::Backend => protocol::resp_json(&backend.backend().await),
        // A pin to a backend this box does not have is an `error:`, not an `ok`:
        // accepting it and acting over CEC anyway would be a control that
        // reports an effect nothing applies.
        Command::BackendPin(pin) => match backend.pin_backend(pin).await {
            Ok(()) => protocol::resp_ok(),
            Err(why) => protocol::resp_error(&why),
        },
        Command::BackendPinUsage => protocol::resp_usage(protocol::BACKEND_PIN_USAGE),
        Command::Unknown => protocol::resp_unknown(),
    }
}

/// The one place an [`ActionOutcome`] becomes a reply line.
///
/// Three outcomes, three tokens. Collapsing `Refused` into `ok` is what v1 did
/// and it is the confusion this design removes; collapsing it into `error:`
/// would report a fault where there is none.
async fn act(backend: &Arc<dyn AvBackend>, action: Action) -> String {
    outcome(backend.act(action).await)
}

/// Three outcomes, three tokens. The only place the mapping is written.
fn outcome(outcome: ActionOutcome) -> String {
    match outcome {
        ActionOutcome::Done => protocol::resp_ok(),
        ActionOutcome::Refused(why) => protocol::resp_refused(&why),
        ActionOutcome::Failed(why) => protocol::resp_error(&why),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::action::{CecTx, StandbyTarget};
    use crate::backend::testing::{FakeAvr, FakeBackend};
    use crate::state::{BusObservation, PhysAddr, PowerState};
    use crate::volume::{VolumeKey, VolumeTx};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    fn fake() -> Arc<FakeBackend> {
        Arc::new(FakeBackend::new())
    }

    async fn reply(b: &Arc<dyn AvBackend>, line: &str) -> String {
        dispatch(b, Command::parse(line)).await
    }

    #[tokio::test]
    async fn dispatch_covers_the_verb_table() {
        let b: Arc<dyn AvBackend> = fake();
        assert_eq!(reply(&b, "ping").await, "ok");

        let state = reply(&b, "av-state").await;
        let json: serde_json::Value = serde_json::from_str(&state).unwrap();
        assert_eq!(json["backend"], serde_json::json!("cec"));
        assert_eq!(json["device"], serde_json::json!("/dev/cec0"));
        assert_eq!(json["physAddr"], serde_json::json!("2.5.0.0"));
    }

    /// **`av-state` reports `unknown` for what it has not observed, rather than
    /// an error or a default.**
    ///
    /// "The bus has said nothing" is a state this verb exists to report; an
    /// error reply would make a correctly-running daemon on a quiet bus look
    /// broken, which is the `cec-health` failure one layer down.
    #[tokio::test]
    async fn av_state_answers_on_a_silent_bus_without_inventing_anything() {
        let b: Arc<dyn AvBackend> = fake();
        let r = reply(&b, "av-state").await;
        assert!(!r.starts_with("error:"), "{r}");
        let json: serde_json::Value = serde_json::from_str(&r).unwrap();
        assert_eq!(json["weAreSource"], serde_json::Value::Null);
        assert_eq!(json["volume"], serde_json::Value::Null);
        assert_eq!(json["observedAt"], serde_json::Value::Null);
    }

    /// An observation made by the rx loop reaches the wire.
    ///
    /// This is the reachability check on the `weAreSource` rule: the real path
    /// can produce `true`, not just the unit test poking the field.
    #[tokio::test]
    async fn an_observed_active_source_reaches_the_reply() {
        let backend = fake();
        backend.observe(
            BusObservation::ActiveSource("2.5.0.0".parse::<PhysAddr>().unwrap()),
            1_700_000_000_000,
        );
        let b: Arc<dyn AvBackend> = backend;
        let json: serde_json::Value = serde_json::from_str(&reply(&b, "av-state").await).unwrap();
        assert_eq!(json["activeSource"], serde_json::json!("2.5.0.0"));
        assert_eq!(json["weAreSource"], serde_json::json!(true));
        assert_eq!(json["observedAt"], serde_json::json!(1_700_000_000_000u64));
    }

    #[tokio::test]
    async fn unknown_verbs_are_unknown() {
        let b: Arc<dyn AvBackend> = fake();
        assert_eq!(reply(&b, "frobnicate").await, "unknown");
        assert_eq!(reply(&b, "av-stateX").await, "unknown");
        assert_eq!(reply(&b, "backendX").await, "unknown");
        assert_eq!(reply(&b, "backend cec").await, "unknown");
    }

    // -----------------------------------------------------------------------
    // The backend verbs, and the IP leg, end to end over the seam.
    // -----------------------------------------------------------------------

    fn with_ip() -> Arc<FakeBackend> {
        Arc::new(FakeBackend::with_ip_leg())
    }

    /// **The rule: with a healthy adapter, `cec` is authoritative — and on a box
    /// with no IP leg it is the only backend there is.**
    #[tokio::test]
    async fn backend_reports_cec_while_the_adapter_is_healthy() {
        let b: Arc<dyn AvBackend> = fake();
        let json: serde_json::Value = serde_json::from_str(&reply(&b, "backend").await).unwrap();
        assert_eq!(json["active"], serde_json::json!("cec"));
        assert_eq!(json["available"], serde_json::json!(["cec"]));
        assert_eq!(json["pin"], serde_json::json!("auto"));
        assert!(!json["reason"].as_str().unwrap().is_empty());

        // And with an IP leg configured, it is listed as available without
        // being active.
        let b: Arc<dyn AvBackend> = with_ip();
        let json: serde_json::Value = serde_json::from_str(&reply(&b, "backend").await).unwrap();
        assert_eq!(json["active"], serde_json::json!("cec"));
        assert_eq!(json["available"], serde_json::json!(["cec", "ip"]));
    }

    /// **THE LOAD-BEARING TEST OF THIS STEP: a wedged adapter flips the backend
    /// to `ip`, the reply names the observation, and `standby` still reaches the
    /// receiver over telnet.**
    ///
    /// The failover is reached the real way — a degraded health verdict that
    /// holds past the hysteresis, folded through the same `Failover` the kernel
    /// backend uses — not by poking a field.
    ///
    /// Mutation-check (run 2026-09-14): make `failover::desired` return
    /// `Backend::Cec` for a degraded adapter and this fails on the first
    /// assertion; drop the `plan_for` call from the fake's `act` and it fails on
    /// the telnet assertion.
    #[tokio::test]
    async fn a_wedged_adapter_moves_the_backend_to_ip_and_standby_still_reaches_the_receiver() {
        let backend = Arc::new(FakeBackend::with_ip_leg().with_avr_main_power());
        let b: Arc<dyn AvBackend> = Arc::clone(&backend) as Arc<dyn AvBackend>;

        backend.fail_over_to_ip();
        let json: serde_json::Value = serde_json::from_str(&reply(&b, "backend").await).unwrap();
        assert_eq!(json["active"], serde_json::json!("ip"));
        let reason = json["reason"].as_str().unwrap();
        assert!(
            reason.contains("degraded"),
            "the reply must name the observation that caused it: {reason}"
        );

        // Standby now goes over telnet — and it is NOT gated on the CEC
        // ownership proof, which a dead bus can never supply.
        assert_eq!(reply(&b, "standby").await, "ok");
        let sessions = backend.ip_sessions();
        assert_eq!(sessions.len(), 1, "one connection: {sessions:?}");
        assert_eq!(
            sessions[0].1,
            vec!["Z2OFF".to_string(), "PWSTANDBY".to_string()]
        );
        // Nothing reached the CEC bus.
        assert!(backend.transmits().is_empty(), "{:?}", backend.transmits());
    }

    /// **The rule: an IP standby that cannot power the main zone down is an
    /// `error:`, never an `ok`.**
    ///
    /// This is the DEFAULT configuration — `main_power` is opt-in — so it is the
    /// common case, not an edge one. The Zone-2 command really did go out and
    /// the reply says so.
    #[tokio::test]
    async fn an_ip_standby_that_leaves_the_main_zone_on_says_so() {
        let backend = with_ip();
        let b: Arc<dyn AvBackend> = Arc::clone(&backend) as Arc<dyn AvBackend>;
        backend.fail_over_to_ip();

        let r = reply(&b, "standby").await;
        assert!(r.starts_with("error:"), "{r}");
        assert!(r.contains("main_power"), "{r}");
        assert_ne!(r, "ok");
        // Zone 2 still went off: the shortfall is the main zone, not the action.
        assert_eq!(backend.ip_sessions()[0].1, vec!["Z2OFF".to_string()]);
    }

    /// **THE CAPABILITY-COMPLEMENT RULE, at the IPC surface: the IP steps run
    /// with a perfectly healthy CEC bus, and they run BEFORE the CEC steps.**
    ///
    /// A `wake` broadcasts the magic packet (a television at mains standby hears
    /// no CEC at all) and a `standby` sends `Z2OFF` (no CEC equivalent exists),
    /// while `backend` still reports `cec`.
    ///
    /// Mutation-check (run 2026-09-14): make `ip::plan_for` return an empty plan
    /// unless the role is `Authority` — the "IP is only a fallback" reading of
    /// §13 Q7 — and both halves of this fail.
    #[tokio::test]
    async fn the_cold_path_steps_run_with_a_healthy_cec_bus() {
        let backend = with_ip();
        let b: Arc<dyn AvBackend> = Arc::clone(&backend) as Arc<dyn AvBackend>;

        assert_eq!(reply(&b, "wake").await, "ok");
        assert_eq!(
            backend.wol_packets(),
            2,
            "a cold television needs the packet"
        );
        assert_eq!(
            backend.ip_sessions()[0].1,
            vec!["SIGAME".to_string()],
            "no PWON: CEC is authoritative and powers the main zone"
        );
        // …and the CEC steps still happened.
        assert_eq!(
            backend.transmits(),
            vec![
                CecTx::ImageViewOn,
                CecTx::ActiveSource("2.5.0.0".parse().unwrap())
            ]
        );
        let json: serde_json::Value = serde_json::from_str(&reply(&b, "backend").await).unwrap();
        assert_eq!(json["active"], serde_json::json!("cec"));

        assert_eq!(reply(&b, "standby").await, "ok");
        assert_eq!(
            backend.ip_sessions()[1].1,
            vec!["Z2OFF".to_string()],
            "Zone 2 has no CEC equivalent, so it goes on every standby"
        );
    }

    /// A `standby` refused by the CEC ownership gate transmits nothing on the
    /// bus — but the Zone-2 command has already gone, because it is not part of
    /// what the gate protects.
    ///
    /// The gate exists to stop this box powering off a television somebody else
    /// is watching. Zone 2 is a different room.
    #[tokio::test]
    async fn a_refused_cec_standby_still_switches_zone_two_off() {
        let backend = with_ip();
        backend.observe(
            BusObservation::ActiveSource("1.0.0.0".parse::<PhysAddr>().unwrap()),
            1_700_000_000_000,
        );
        let b: Arc<dyn AvBackend> = Arc::clone(&backend) as Arc<dyn AvBackend>;

        let r = reply(&b, "standby").await;
        assert!(r.starts_with("refused:"), "{r}");
        assert!(backend.transmits().is_empty());
        assert_eq!(backend.ip_sessions()[0].1, vec!["Z2OFF".to_string()]);
    }

    /// **The rule: while the IP leg is carrying actions, a volume verb reports
    /// that there is nothing to send — it does not transmit into a bus the
    /// daemon has just concluded it cannot use.**
    #[tokio::test]
    async fn volume_over_the_ip_leg_reports_that_there_is_nothing_to_send() {
        let backend = with_ip();
        let b: Arc<dyn AvBackend> = Arc::clone(&backend) as Arc<dyn AvBackend>;
        backend.fail_over_to_ip();

        let r = reply(&b, "volume up").await;
        assert!(r.starts_with("error:"), "{r}");
        assert!(r.contains("only reachable over CEC"), "{r}");
        assert!(
            backend.volume_transmits().is_empty(),
            "{:?}",
            backend.volume_transmits()
        );
    }

    /// `backend-pin` overrides the decision; `auto` gives it back.
    #[tokio::test]
    async fn backend_pin_overrides_the_decision_and_auto_returns_it() {
        let backend = with_ip();
        let b: Arc<dyn AvBackend> = Arc::clone(&backend) as Arc<dyn AvBackend>;
        backend.fail_over_to_ip();

        assert_eq!(reply(&b, "backend-pin cec").await, "ok");
        let json: serde_json::Value = serde_json::from_str(&reply(&b, "backend").await).unwrap();
        assert_eq!(json["active"], serde_json::json!("cec"));
        assert_eq!(json["pin"], serde_json::json!("cec"));
        assert!(json["reason"].as_str().unwrap().contains("pinned to cec"));

        assert_eq!(reply(&b, "backend-pin auto").await, "ok");
        let json: serde_json::Value = serde_json::from_str(&reply(&b, "backend").await).unwrap();
        assert_eq!(json["active"], serde_json::json!("ip"));
    }

    /// **The rule: pinning to a backend this box does not have is an `error:`,
    /// not an `ok` that is then ignored.**
    #[tokio::test]
    async fn pinning_to_an_absent_backend_is_an_error() {
        let b: Arc<dyn AvBackend> = fake();
        let r = reply(&b, "backend-pin ip").await;
        assert!(r.starts_with("error:"), "{r}");
        assert!(r.contains("no IP leg is configured"), "{r}");
        assert_ne!(r, "ok");
        // The pin did not take effect.
        let json: serde_json::Value = serde_json::from_str(&reply(&b, "backend").await).unwrap();
        assert_eq!(json["pin"], serde_json::json!("auto"));
    }

    /// A missing or unknown `backend-pin` argument is a usage error and changes
    /// nothing — never `unknown`, and never a silent default to `auto`.
    #[tokio::test]
    async fn a_missing_backend_pin_argument_is_a_usage_error() {
        let backend = with_ip();
        let b: Arc<dyn AvBackend> = Arc::clone(&backend) as Arc<dyn AvBackend>;
        assert_eq!(reply(&b, "backend-pin cec").await, "ok");
        for line in ["backend-pin", "backend-pin libcec", "backend-pin cec ip"] {
            let r = reply(&b, line).await;
            assert_eq!(r, "error:usage: backend-pin cec|ip|auto", "{line} -> {r}");
            assert_ne!(r, "unknown", "{line}");
        }
        let json: serde_json::Value = serde_json::from_str(&reply(&b, "backend").await).unwrap();
        assert_eq!(
            json["pin"],
            serde_json::json!("cec"),
            "a usage error must not clear the operator's pin"
        );
    }

    // -----------------------------------------------------------------------
    // av-health, end to end over the seam.
    // -----------------------------------------------------------------------

    /// **THE RULE, at the IPC surface: `av-health` publishes what was observed
    /// and when, and `unknown` never reaches a client as `healthy`.**
    ///
    /// All three states are reached the real way — the backend's own health
    /// machine, driven by the same calls the kernel backend makes: the open-time
    /// facts, a `PollResult::StateChange` from the receive loop, and a
    /// `CEC_ADAP_G_CAPS` probe from the watchdog loop.
    ///
    /// Mutation-check (run 2026-09-14): make `health::classify`'s
    /// `(true, Unknown)` arm return `Healthy` and the middle assertion fails.
    #[tokio::test]
    async fn av_health_publishes_the_tri_state_and_never_renders_unknown_as_healthy() {
        let backend = fake();
        let b: Arc<dyn AvBackend> = Arc::clone(&backend) as Arc<dyn AvBackend>;

        let json: serde_json::Value = serde_json::from_str(&reply(&b, "av-health").await).unwrap();
        assert_eq!(json["state"], serde_json::json!("healthy"));

        // The kernel says the adapter was reconfigured.
        backend.note_state_change();
        let json: serde_json::Value = serde_json::from_str(&reply(&b, "av-health").await).unwrap();
        assert_eq!(json["state"], serde_json::json!("unknown"));
        assert_ne!(json["state"], serde_json::json!("healthy"));

        // The fd stops answering.
        backend.probe_fd(false);
        let json: serde_json::Value = serde_json::from_str(&reply(&b, "av-health").await).unwrap();
        assert_eq!(json["state"], serde_json::json!("degraded"));
        assert!(
            json["reason"].as_str().unwrap().contains("CEC_ADAP_G_CAPS"),
            "the reason must name the observation: {json}"
        );
    }

    /// **The rule: the four facts are AGES, and a fact that has not happened is
    /// `null` — not `0`, which would read as "just now".**
    ///
    /// A silent bus is the normal state in this deployment (everything can be off),
    /// and it is reported as a silence, not as a fault.
    #[tokio::test]
    async fn av_health_reports_ages_and_a_silent_bus_is_not_a_fault() {
        let backend = fake();
        let b: Arc<dyn AvBackend> = Arc::clone(&backend) as Arc<dyn AvBackend>;

        let json: serde_json::Value = serde_json::from_str(&reply(&b, "av-health").await).unwrap();
        for field in ["lastTxOk", "lastRxMs", "busActivityMs"] {
            assert_eq!(json[field], serde_json::Value::Null, "{field}: {json}");
            assert_ne!(json[field], serde_json::json!(0), "{field}");
        }
        // Nothing has been heard, and the verdict is still healthy.
        assert_eq!(json["state"], serde_json::json!("healthy"));

        // A transmit the bus accepted, and a message heard, both become ages.
        assert_eq!(reply(&b, "input-claim").await, "ok");
        backend.note_rx();
        backend.advance(2_500);
        let json: serde_json::Value = serde_json::from_str(&reply(&b, "av-health").await).unwrap();
        assert_eq!(json["lastTxOk"], serde_json::json!(2_500));
        assert_eq!(json["lastRxMs"], serde_json::json!(2_500));
        // Still no pin monitor, so still no electrical-activity age — and the
        // reason says so rather than leaving a reader to guess.
        assert_eq!(json["busActivityMs"], serde_json::Value::Null);
        assert!(json["reason"]
            .as_str()
            .unwrap()
            .contains("CEC_CAP_MONITOR_PIN absent"));
    }

    /// **A refused action is not a transmit**, so it does not move the last-tx
    /// age — the same zero-transmit rule the refusal reply carries, seen from
    /// the health side.
    #[tokio::test]
    async fn a_refused_action_does_not_move_the_last_transmit_age() {
        let backend = fake();
        backend.observe(
            BusObservation::ActiveSource("1.0.0.0".parse::<PhysAddr>().unwrap()),
            1_700_000_000_000,
        );
        let b: Arc<dyn AvBackend> = Arc::clone(&backend) as Arc<dyn AvBackend>;

        assert!(reply(&b, "standby").await.starts_with("refused:"));
        let json: serde_json::Value = serde_json::from_str(&reply(&b, "av-health").await).unwrap();
        assert_eq!(json["lastTxOk"], serde_json::Value::Null);
    }

    /// **THE WATCHDOG RULE, through the same backend the IPC surface answers
    /// from: the feed follows fact 1, not the published verdict.**
    ///
    /// A degraded-but-answering daemon must keep feeding, or every television
    /// standby becomes a restart loop — v1's "recovered a daemon that was never
    /// broken" with a new mechanism.
    ///
    /// Mutation-check (run 2026-09-14): make `should_feed_watchdog` return
    /// `self.state == HealthState::Healthy` and the last assertion fails; make
    /// it return `true` unconditionally and the middle one does.
    #[tokio::test]
    async fn the_watchdog_feed_follows_the_fd_and_not_the_verdict() {
        let backend = fake();
        let b: Arc<dyn AvBackend> = Arc::clone(&backend) as Arc<dyn AvBackend>;
        assert!(backend.feeds_watchdog());

        backend.probe_fd(false);
        assert!(!backend.feeds_watchdog(), "a wedged fd must stop the feed");
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&reply(&b, "av-health").await).unwrap()
                ["state"],
            serde_json::json!("degraded")
        );

        // Answering again, but unaddressed: degraded, and still fed.
        backend.probe_fd(true);
        backend.note_state_change();
        assert!(
            backend.feeds_watchdog(),
            "an adapter that answers must keep the daemon alive whatever the verdict"
        );
    }

    // -----------------------------------------------------------------------
    // Power and input switching, end to end over the seam.
    // -----------------------------------------------------------------------

    /// `wake` is One Touch Play then the claim, in that order.
    #[tokio::test]
    async fn wake_powers_the_chain_on_then_claims_the_display() {
        let backend = fake();
        // The TV answers the power read-back, so `tvPower` stops being unknown.
        backend.set_power_reply(Some(PowerState::On));
        let b: Arc<dyn AvBackend> = Arc::clone(&backend) as Arc<dyn AvBackend>;

        assert_eq!(reply(&b, "wake").await, "ok");
        assert_eq!(
            backend.transmits(),
            vec![
                CecTx::ImageViewOn,
                CecTx::ActiveSource("2.5.0.0".parse().unwrap())
            ]
        );

        // The claim reaches the published state: our own `<Active Source>` never
        // comes back through the receive loop, so without the local record this
        // would still read `null`.
        let json: serde_json::Value = serde_json::from_str(&reply(&b, "av-state").await).unwrap();
        assert_eq!(json["activeSource"], serde_json::json!("2.5.0.0"));
        assert_eq!(json["weAreSource"], serde_json::json!(true));
        assert_eq!(
            json["displayOwnership"]["state"],
            serde_json::json!("owned-by-us")
        );
        // Read back, not assumed: `tvPower` is the TV's own answer.
        assert_eq!(json["tvPower"], serde_json::json!("on"));
        // And a claim of ours is NOT evidence the bus announces ownership.
        assert_eq!(
            json["displayOwnership"]["everObserved"],
            serde_json::json!(false)
        );
    }

    /// A TV that does not answer the power read-back leaves the power
    /// `unknown` — never `on` because two frames were ACKed.
    #[tokio::test]
    async fn a_silent_power_read_back_leaves_the_power_unknown() {
        let backend = fake();
        let b: Arc<dyn AvBackend> = Arc::clone(&backend) as Arc<dyn AvBackend>;
        assert_eq!(reply(&b, "wake").await, "ok");
        let json: serde_json::Value = serde_json::from_str(&reply(&b, "av-state").await).unwrap();
        assert_eq!(json["tvPower"], serde_json::Value::Null);
    }

    /// **THE LOAD-BEARING NEGATIVE TEST, at the IPC surface: `standby` while
    /// another device holds active source refuses and transmits ZERO
    /// messages.**
    ///
    /// Asserted on the transmits produced through the backend seam, not on which
    /// readers were called — v1's tests drew that line and this mirrors it. The
    /// ownership state asserted here is reached the REAL way: an `<Active
    /// Source>` broadcast folded by the receive path, not a field poked
    /// directly.
    ///
    /// Mutation-check (run 2026-09-14): delete the `owns_display` gate from
    /// `action::plan`'s `Standby` arm and this fails on both assertions.
    #[tokio::test]
    async fn standby_refuses_and_transmits_nothing_when_someone_else_holds_the_display() {
        let backend = fake();
        // The Apple TV takes the screen — exactly as the rx loop would report
        // it.
        backend.observe(
            BusObservation::ActiveSource("1.0.0.0".parse::<PhysAddr>().unwrap()),
            1_700_000_000_000,
        );
        let b: Arc<dyn AvBackend> = Arc::clone(&backend) as Arc<dyn AvBackend>;

        let r = reply(&b, "standby").await;
        assert!(r.starts_with("refused:"), "{r}");
        assert!(
            r.contains("1.0.0.0"),
            "the refusal must name the owner: {r}"
        );
        assert!(
            backend.transmits().is_empty(),
            "a refused standby must put NOTHING on the bus, got {:?}",
            backend.transmits()
        );
        // A refusal is neither a success nor a fault.
        assert_ne!(r, "ok");
        assert!(!r.starts_with("error:"));
    }

    /// The same for a bus that has said nothing at all — the daemon-started-
    /// mid-session case, which is the common one.
    #[tokio::test]
    async fn standby_refuses_on_a_silent_bus() {
        let backend = fake();
        let b: Arc<dyn AvBackend> = Arc::clone(&backend) as Arc<dyn AvBackend>;
        let r = reply(&b, "standby").await;
        assert!(r.starts_with("refused:"), "{r}");
        assert!(backend.transmits().is_empty());
    }

    /// And the positive case, reached the real way: we claim the display, then
    /// standby proceeds and addresses two named devices — never a broadcast.
    #[tokio::test]
    async fn standby_proceeds_once_we_positively_hold_the_display() {
        let backend = fake();
        let b: Arc<dyn AvBackend> = Arc::clone(&backend) as Arc<dyn AvBackend>;
        assert_eq!(reply(&b, "input-claim").await, "ok");

        assert_eq!(reply(&b, "standby").await, "ok");
        assert_eq!(
            backend.transmits(),
            vec![
                CecTx::ActiveSource("2.5.0.0".parse().unwrap()),
                CecTx::Standby(StandbyTarget::Tv),
                CecTx::Standby(StandbyTarget::AudioSystem),
            ]
        );
    }

    /// `input-claim` / `input-release` — jedwards1230/tv-shell#372's ask, in a
    /// v2 shape.
    #[tokio::test]
    async fn claim_and_release_move_the_published_ownership() {
        let backend = fake();
        let b: Arc<dyn AvBackend> = Arc::clone(&backend) as Arc<dyn AvBackend>;

        assert_eq!(reply(&b, "input-claim").await, "ok");
        let json: serde_json::Value = serde_json::from_str(&reply(&b, "av-state").await).unwrap();
        assert_eq!(
            json["displayOwnership"]["state"],
            serde_json::json!("owned-by-us")
        );

        assert_eq!(reply(&b, "input-release").await, "ok");
        assert_eq!(
            backend.transmits(),
            vec![
                CecTx::ActiveSource("2.5.0.0".parse().unwrap()),
                CecTx::InactiveSource("2.5.0.0".parse().unwrap()),
            ]
        );
        // Released goes to UNKNOWN, never to "nobody": the message says who let
        // go, never who has it now.
        let json: serde_json::Value = serde_json::from_str(&reply(&b, "av-state").await).unwrap();
        assert_eq!(json["activeSource"], serde_json::Value::Null);
        assert_eq!(json["weAreSource"], serde_json::Value::Null);
        assert_eq!(
            json["displayOwnership"]["state"],
            serde_json::json!("unknown")
        );
    }

    /// `input-select` hands the display to another device — a `<Set Stream
    /// Path>`, which the libcec path could not express at all.
    #[tokio::test]
    async fn input_select_broadcasts_a_stream_path_to_the_named_address() {
        let backend = fake();
        let b: Arc<dyn AvBackend> = Arc::clone(&backend) as Arc<dyn AvBackend>;
        assert_eq!(reply(&b, "input-select 1.0.0.0").await, "ok");
        assert_eq!(
            backend.transmits(),
            vec![CecTx::SetStreamPath("1.0.0.0".parse().unwrap())]
        );
        // No guess is recorded: the selected device confirms with its own
        // `<Active Source>`, which the receive loop folds.
        let json: serde_json::Value = serde_json::from_str(&reply(&b, "av-state").await).unwrap();
        assert_eq!(json["activeSource"], serde_json::Value::Null);
    }

    /// **The rule: a malformed body is a usage error and transmits nothing.**
    ///
    /// Distinct from `unknown` (the client knows the verb) and from any silent
    /// default (a `<Set Stream Path>` to a nonexistent port fails silently).
    #[tokio::test]
    async fn a_malformed_input_select_is_a_usage_error_and_transmits_nothing() {
        let backend = fake();
        let b: Arc<dyn AvBackend> = Arc::clone(&backend) as Arc<dyn AvBackend>;
        for line in [
            "input-select",
            "input-select nonsense",
            "input-select 1.0.0",
        ] {
            let r = reply(&b, line).await;
            assert!(r.starts_with("error:usage: input-select"), "{line} -> {r}");
            assert_ne!(r, "unknown", "{line}");
        }
        assert!(backend.transmits().is_empty());
    }

    /// A bus that NAKs the transmit is a FAILURE, not a success — `ok` here
    /// means the frame was accepted on the bus, not that it left the adapter.
    #[tokio::test]
    async fn a_naked_transmit_is_an_error_not_an_ok() {
        let backend = fake();
        backend.fail_transmits();
        let b: Arc<dyn AvBackend> = Arc::clone(&backend) as Arc<dyn AvBackend>;
        let r = reply(&b, "wake").await;
        assert!(r.starts_with("error:"), "{r}");
        assert_ne!(r, "ok");
    }

    // -----------------------------------------------------------------------
    // Volume, end to end over the seam.
    // -----------------------------------------------------------------------

    /// A volume step that the AVR acts on answers `ok`, and the level it
    /// reported back reaches `av-state`.
    ///
    /// `volume` and `muted` were `null` in every step before this one; this is
    /// the reachability check that the real path now populates them.
    #[tokio::test]
    async fn a_volume_step_the_avr_acts_on_is_ok_and_reaches_av_state() {
        let backend = fake();
        let b: Arc<dyn AvBackend> = Arc::clone(&backend) as Arc<dyn AvBackend>;

        assert_eq!(reply(&b, "volume up").await, "ok");
        let json: serde_json::Value = serde_json::from_str(&reply(&b, "av-state").await).unwrap();
        assert_eq!(json["volume"], serde_json::json!(41));
        assert_eq!(json["muted"], serde_json::json!(false));

        assert_eq!(reply(&b, "volume down").await, "ok");
        assert_eq!(reply(&b, "volume down").await, "ok");
        let json: serde_json::Value = serde_json::from_str(&reply(&b, "av-state").await).unwrap();
        assert_eq!(json["volume"], serde_json::json!(39));
    }

    /// **THE LOAD-BEARING NEGATIVE TEST OF THIS STEP, at the IPC surface: an
    /// AVR whose selected input is not ours reports a FAILURE, never `ok`.**
    ///
    /// Every frame is accepted by the bus here — the AVR ACKs and ignores, which
    /// is V2_DESIGN §8's constraint — so a daemon judging success from the
    /// transmit would answer `ok` while the volume did not move. Asserted on the
    /// reply AND on the transmits produced through the seam, so it is clear the
    /// press really was sent and the failure is a judgement, not a refusal.
    ///
    /// Mutation-check (run 2026-09-14): make `volume::perform_level` return
    /// `Done` without consulting `judge_level` and this fails, together with
    /// `volume::an_avr_on_another_input_is_a_failure_and_never_an_ok`.
    #[tokio::test]
    async fn a_volume_step_an_avr_ignores_is_an_error_and_never_an_ok() {
        let backend = fake();
        backend.set_avr(FakeAvr {
            acts: false,
            ..FakeAvr::default()
        });
        let b: Arc<dyn AvBackend> = Arc::clone(&backend) as Arc<dyn AvBackend>;

        let r = reply(&b, "volume up").await;
        assert!(r.starts_with("error:"), "{r}");
        assert!(r.contains("non-selected input"), "{r}");
        assert_ne!(r, "ok");
        assert!(!r.starts_with("refused:"));
        assert!(
            backend
                .volume_transmits()
                .contains(&VolumeTx::KeyPressAndRelease(VolumeKey::VolumeUp)),
            "the press really was sent: {:?}",
            backend.volume_transmits()
        );
    }

    /// **The system-audio-mode step is not optional.**
    ///
    /// An AVR out of system-audio mode ignores every volume UI command, so the
    /// request goes out before the press — and the whole ordered sequence is
    /// asserted, not just its presence.
    ///
    /// Mutation-check (run 2026-09-14): delete the `ensure_system_audio_mode`
    /// call from `volume::execute` and this fails on the sequence comparison.
    #[tokio::test]
    async fn a_volume_action_requests_system_audio_mode_before_pressing_anything() {
        let backend = fake();
        backend.set_avr(FakeAvr {
            system_audio_mode: false,
            ..FakeAvr::default()
        });
        let b: Arc<dyn AvBackend> = Arc::clone(&backend) as Arc<dyn AvBackend>;

        assert_eq!(reply(&b, "volume up").await, "ok");
        assert_eq!(
            backend.volume_transmits(),
            vec![
                VolumeTx::SystemAudioModeQuery,
                VolumeTx::SystemAudioModeRequest("2.5.0.0".parse().unwrap()),
                VolumeTx::AudioStatusQuery,
                VolumeTx::KeyPressAndRelease(VolumeKey::VolumeUp),
                VolumeTx::AudioStatusQuery,
            ]
        );
    }

    /// `mute` / `unmute` converge instead of toggling, and a call that finds the
    /// state already correct transmits no key at all.
    #[tokio::test]
    async fn the_mute_verbs_converge_and_do_not_toggle() {
        let backend = fake();
        let b: Arc<dyn AvBackend> = Arc::clone(&backend) as Arc<dyn AvBackend>;

        assert_eq!(reply(&b, "volume mute").await, "ok");
        assert_eq!(reply(&b, "volume mute").await, "ok");
        assert_eq!(
            backend
                .volume_transmits()
                .iter()
                .filter(|tx| matches!(tx, VolumeTx::KeyPressAndRelease(_)))
                .count(),
            1,
            "the second `mute` must transmit no toggle: {:?}",
            backend.volume_transmits()
        );
        let json: serde_json::Value = serde_json::from_str(&reply(&b, "av-state").await).unwrap();
        assert_eq!(json["muted"], serde_json::json!(true));

        assert_eq!(reply(&b, "volume unmute").await, "ok");
        let json: serde_json::Value = serde_json::from_str(&reply(&b, "av-state").await).unwrap();
        assert_eq!(json["muted"], serde_json::json!(false));
    }

    /// `volume-state` names where its values came from, and says `null`
    /// throughout when it knows nothing.
    #[tokio::test]
    async fn volume_state_reports_the_avr_and_names_its_source() {
        let backend = fake();
        let b: Arc<dyn AvBackend> = Arc::clone(&backend) as Arc<dyn AvBackend>;

        let json: serde_json::Value =
            serde_json::from_str(&reply(&b, "volume-state").await).unwrap();
        assert_eq!(json["level"], serde_json::json!(40));
        assert_eq!(json["muted"], serde_json::json!(false));
        assert_eq!(json["source"], serde_json::json!("avr-report"));

        // A receiver that answers nothing, and a bus that has said nothing:
        // every field is null, and NOT `0` / `false`.
        let silent = fake();
        silent.set_avr(FakeAvr {
            report: None,
            ..FakeAvr::default()
        });
        let b: Arc<dyn AvBackend> = silent;
        let json: serde_json::Value =
            serde_json::from_str(&reply(&b, "volume-state").await).unwrap();
        for field in ["level", "muted", "source", "observedAt"] {
            assert_eq!(json[field], serde_json::Value::Null, "{field}: {json}");
            assert_ne!(json[field], serde_json::json!(0), "{field}");
            assert_ne!(json[field], serde_json::json!(false), "{field}");
        }
    }

    /// And the fallback: a silent AVR, but something heard earlier on the bus.
    ///
    /// Reached the real way — a `<Report Audio Status>` folded by the receive
    /// path — so the `observed` source is a state the daemon can actually be in.
    #[tokio::test]
    async fn volume_state_falls_back_to_what_the_bus_said_and_labels_it() {
        let backend = fake();
        backend.set_avr(FakeAvr {
            report: None,
            ..FakeAvr::default()
        });
        backend.observe(
            BusObservation::AudioStatus {
                volume: crate::state::Observation::Known(12),
                muted: true,
            },
            1_699_000_000_000,
        );
        let b: Arc<dyn AvBackend> = backend;
        let json: serde_json::Value =
            serde_json::from_str(&reply(&b, "volume-state").await).unwrap();
        assert_eq!(json["level"], serde_json::json!(12));
        assert_eq!(json["muted"], serde_json::json!(true));
        assert_eq!(json["source"], serde_json::json!("observed"));
        assert_eq!(json["observedAt"], serde_json::json!(1_699_000_000_000u64));
    }

    /// **The rule: `volume` with a missing or unknown argument is a usage error
    /// and transmits nothing.**
    ///
    /// Distinct from `unknown` (the client knows the verb) and from any silent
    /// default (a typo must not report `ok` for a change nobody asked for).
    #[tokio::test]
    async fn a_missing_or_unknown_volume_argument_is_a_usage_error_and_transmits_nothing() {
        let backend = fake();
        let b: Arc<dyn AvBackend> = Arc::clone(&backend) as Arc<dyn AvBackend>;
        for line in ["volume", "volume louder", "volume UP", "volume up down"] {
            let r = reply(&b, line).await;
            assert_eq!(
                r, "error:usage: volume up|down|mute|unmute",
                "{line} -> {r}"
            );
            assert_ne!(r, "unknown", "{line}");
            assert_ne!(r, "ok", "{line}");
        }
        assert!(
            backend.volume_transmits().is_empty(),
            "{:?}",
            backend.volume_transmits()
        );
    }

    /// A volume action with no readable address of our own refuses — and a
    /// refusal is zero transmits, here as everywhere else.
    #[tokio::test]
    async fn a_volume_action_refuses_when_our_own_address_is_unknown() {
        let backend = Arc::new(FakeBackend::without_our_address());
        let b: Arc<dyn AvBackend> = Arc::clone(&backend) as Arc<dyn AvBackend>;
        let r = reply(&b, "volume up").await;
        assert!(r.starts_with("refused:"), "{r}");
        assert!(r.contains("CEC_ADAP_G_PHYS_ADDR"), "{r}");
        assert!(
            backend.volume_transmits().is_empty(),
            "a refusal must put NOTHING on the bus, got {:?}",
            backend.volume_transmits()
        );
    }

    #[tokio::test]
    async fn no_reply_ever_contains_a_newline() {
        let b: Arc<dyn AvBackend> = fake();
        for line in [
            "ping",
            "av-state",
            "av-health",
            "frobnicate",
            "",
            "av-state x",
            "standby",
            "input-select",
            "input-select ???",
            "volume",
            "volume up",
            "volume ???",
            "volume-state",
            "backend",
            "backend-pin",
            "backend-pin ???",
            "backend-pin ip",
        ] {
            let r = reply(&b, line).await;
            assert!(!r.contains('\n'), "{line} -> {r:?}");
            assert!(!r.contains('\r'), "{line} -> {r:?}");
        }
    }

    async fn send_line(stream: &mut UnixStream, line: &str) -> String {
        stream
            .write_all(format!("{line}\n").as_bytes())
            .await
            .unwrap();
        // Replies are newline-framed; read to the first '\n' so a long JSON
        // reply is never truncated mid-document by a fixed-size read.
        let mut acc = Vec::new();
        let mut byte = [0u8; 1];
        loop {
            let n = stream.read(&mut byte).await.unwrap();
            if n == 0 || byte[0] == b'\n' {
                break;
            }
            acc.push(byte[0]);
        }
        String::from_utf8_lossy(&acc).trim_end().to_string()
    }

    #[tokio::test]
    async fn end_to_end_over_a_real_socket() {
        // Deliberately a short `/tmp` path and not a deep scratch one: this
        // binds a real Unix-domain socket and `sockaddr_un::sun_path` caps the
        // path at ~104 bytes. Same exception v1 and the core document.
        let sock = std::path::PathBuf::from("/tmp")
            .join(format!("tv-cec-ipc-test-{}.sock", std::process::id()))
            .to_string_lossy()
            .to_string();
        let backend: Arc<dyn AvBackend> = fake();
        let server = tokio::spawn(serve(sock.clone(), backend));

        for _ in 0..100 {
            if std::path::Path::new(&sock).exists() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }

        let mut s = UnixStream::connect(&sock).await.unwrap();
        assert_eq!(send_line(&mut s, "ping").await, "ok");
        assert_eq!(send_line(&mut s, "nope").await, "unknown");
        // Several commands on one connection, as v1 allows.
        let state = send_line(&mut s, "av-state").await;
        assert!(state.starts_with('{'), "{state}");

        // The socket must be private the moment it exists.
        let mode = std::fs::metadata(&sock).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "socket must be 0600");

        server.abort();
        let _ = std::fs::remove_file(&sock);
    }

    #[tokio::test]
    async fn a_stale_socket_file_does_not_stop_a_restart() {
        let sock = std::path::PathBuf::from("/tmp")
            .join(format!("tv-cec-stale-{}.sock", std::process::id()))
            .to_string_lossy()
            .to_string();
        std::fs::write(&sock, b"not a socket").unwrap();
        let listener = bind(&sock).expect("a stale file must be removed, not fatal");
        drop(listener);
        let _ = std::fs::remove_file(&sock);
    }
}
