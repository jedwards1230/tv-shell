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

use crate::backend::AvBackend;
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
        Command::Unknown => protocol::resp_unknown(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::testing::FakeBackend;
    use crate::state::{BusObservation, PhysAddr};
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
        // The verbs steps 4-7 add. Until they transmit something, `unknown` is
        // the honest answer — a stub `ok` would tell a caller the television was
        // woken when nothing happened.
        for later in ["wake", "standby", "input-claim", "volume up", "av-health"] {
            assert_eq!(reply(&b, later).await, "unknown", "{later}");
        }
    }

    #[tokio::test]
    async fn no_reply_ever_contains_a_newline() {
        let b: Arc<dyn AvBackend> = fake();
        for line in ["ping", "av-state", "frobnicate", "", "av-state x"] {
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
