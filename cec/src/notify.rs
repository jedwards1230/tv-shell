//! The `sd_notify` transport — `READY=1` and `WATCHDOG=1`.
//!
//! The unit is `Type=notify`, which means **systemd waits for `READY=1` before
//! the start job completes**; a daemon that never sends it hangs the unit at
//! start. It is also `WatchdogSec=`, which means a daemon that never sends
//! `WATCHDOG=1` is SIGABRTed and restarted on that interval. Both halves are
//! therefore load-bearing from this first step: shipping either directive
//! without its message would ship a unit that self-kills or never comes up.
//!
//! # This module is the transport, and only the transport
//!
//! **It decides nothing.** *When* to feed the watchdog is a health question, and
//! `health.rs` owns it from step 6 — the plan's rule is that the feed happens
//! only when `CEC_ADAP_G_CAPS` round-trips (fact 1), so that a wedged fd stops
//! the feed and systemd restarts the daemon on a bounded timer with no polling
//! script and no bus side effects. Until `health.rs` lands, `main` applies that
//! one rule directly. The split is kept because the alternative — a transport
//! that also decides — is how `cec-health` ended up reporting health it could
//! not know.
//!
//! No `sd-notify` crate: the protocol is one datagram of ASCII to the socket
//! named by `$NOTIFY_SOCKET`, and a dependency for that would be a dependency
//! whose transitive graph has to be re-argued against the workspace's
//! no-system-C-libraries rule for no gain.

use std::io;
use std::os::unix::net::{SocketAddr, UnixDatagram};

/// The environment variable systemd sets for a `Type=notify` service.
pub const NOTIFY_SOCKET_ENV: &str = "NOTIFY_SOCKET";

/// A connection to the service manager, or the honest absence of one.
///
/// Absent is the normal case when the binary is run from a shell, and it must
/// stay a no-op rather than an error: the daemon is expected to be runnable by
/// hand for development, exactly as the core is.
#[derive(Debug)]
pub struct Notifier {
    socket: Option<(UnixDatagram, SocketAddr)>,
}

impl Notifier {
    /// Open the notify socket named by `$NOTIFY_SOCKET`, if there is one.
    ///
    /// A malformed or unopenable socket degrades to "not under systemd" with a
    /// warning rather than failing startup. The daemon is more useful running
    /// and un-notified than not running at all — and under a real
    /// `Type=notify` unit the missing `READY=1` is loud all by itself.
    #[must_use]
    pub fn from_env() -> Notifier {
        let Some(name) = std::env::var_os(NOTIFY_SOCKET_ENV) else {
            tracing::debug!("{NOTIFY_SOCKET_ENV} is unset; not running under a notify unit");
            return Notifier { socket: None };
        };
        match Self::open(&name.to_string_lossy()) {
            Ok(socket) => Notifier {
                socket: Some(socket),
            },
            Err(e) => {
                tracing::warn!("cannot open {NOTIFY_SOCKET_ENV}={name:?}: {e}");
                Notifier { socket: None }
            }
        }
    }

    fn open(name: &str) -> io::Result<(UnixDatagram, SocketAddr)> {
        let addr = notify_address(name)?;
        let socket = UnixDatagram::unbound()?;
        // Non-blocking on purpose: this is sent from an async task, and a full
        // socket buffer must return `WouldBlock` for us to drop the datagram
        // rather than stall the reactor. A dropped `WATCHDOG=1` costs one
        // interval; a stalled reactor costs the daemon.
        socket.set_nonblocking(true)?;
        Ok((socket, addr))
    }

    /// Tell the service manager the daemon is up and serving.
    pub fn ready(&self) {
        self.send("READY=1\n");
    }

    /// Feed the watchdog. Call only when the liveness condition actually holds.
    pub fn watchdog(&self) {
        self.send("WATCHDOG=1\n");
    }

    /// Publish a one-line human status, shown by `systemctl status`.
    pub fn status(&self, text: &str) {
        // Newlines would split one datagram into extra protocol lines, so the
        // same one-line rule the IPC replies follow applies here.
        self.send(&format!("STATUS={}\n", crate::protocol::sanitize_ipc(text)));
    }

    fn send(&self, message: &str) {
        let Some((socket, addr)) = &self.socket else {
            return;
        };
        if let Err(e) = socket.send_to_addr(message.as_bytes(), addr) {
            // Never fatal. Losing a notification is a supervision problem, and
            // systemd's own response to a missed `WATCHDOG=1` is the correct
            // one; taking the daemon down here would pre-empt it with a worse
            // failure.
            tracing::debug!("sd_notify {message:?} failed: {e}");
        }
    }
}

/// Resolve `$NOTIFY_SOCKET` to a socket address.
///
/// Pure (no I/O), so the two forms are testable. systemd names the socket either
/// as a filesystem path or — with a leading `@` — in the abstract namespace, and
/// a daemon that handles only one of them silently fails to notify on the other.
/// That failure is not silent under `Type=notify`: it hangs the unit at start.
pub fn notify_address(name: &str) -> io::Result<SocketAddr> {
    match name.strip_prefix('@') {
        // The abstract namespace is a Linux extension, and so is the std API for
        // it. This daemon only runs on Linux (there is no `/dev/cecN` anywhere
        // else), so the other arm is a clear error rather than a silent skip.
        #[cfg(target_os = "linux")]
        Some(abstract_name) => {
            use std::os::linux::net::SocketAddrExt;
            SocketAddr::from_abstract_name(abstract_name.as_bytes())
        }
        #[cfg(not(target_os = "linux"))]
        Some(_) => Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "abstract unix sockets are a Linux extension",
        )),
        None if name.starts_with('/') => SocketAddr::from_pathname(name),
        None => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "{NOTIFY_SOCKET_ENV}={name:?} is neither an absolute path nor an abstract \
                 name (a leading '@')"
            ),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **Both forms systemd uses resolve.**
    ///
    /// The abstract form is the one that is easy to miss, and missing it does
    /// not degrade — it hangs the unit's start job, because `Type=notify` waits
    /// for a `READY=1` that never arrives.
    #[test]
    fn both_notify_socket_forms_resolve() {
        // The abstract-namespace accessors are a Linux extension, like the
        // namespace itself, so the whole assertion is gated the way the
        // implementation arm is.
        #[cfg(target_os = "linux")]
        {
            use std::os::linux::net::SocketAddrExt;

            let path = notify_address("/run/user/1000/systemd/notify").unwrap();
            assert!(path.as_pathname().is_some());
            assert!(path.as_abstract_name().is_none());

            let abstract_addr = notify_address("@a/b/systemd/notify").unwrap();
            assert_eq!(
                abstract_addr.as_abstract_name().unwrap(),
                b"a/b/systemd/notify"
            );
            assert!(abstract_addr.as_pathname().is_none());
        }
        #[cfg(not(target_os = "linux"))]
        {
            let path = notify_address("/run/user/1000/systemd/notify").unwrap();
            assert!(path.as_pathname().is_some());
        }
    }

    #[test]
    fn a_relative_name_is_refused_rather_than_guessed_at() {
        for name in ["", "notify", "run/systemd/notify"] {
            assert!(
                notify_address(name).is_err(),
                "{name:?} must not resolve to a socket"
            );
        }
    }

    /// With no `$NOTIFY_SOCKET` every call is a silent no-op, so the binary
    /// stays runnable from a shell.
    #[test]
    fn a_notifier_with_no_socket_is_a_no_op() {
        let n = Notifier { socket: None };
        n.ready();
        n.watchdog();
        n.status("nothing to see here");
    }

    /// **`READY=1` actually leaves the process and arrives at a listener.**
    ///
    /// Every other test in this module checks a part: that both socket forms
    /// resolve, that a relative name is refused, that an absent socket is a
    /// no-op. None of them proves a datagram is sent, which is the thing
    /// `Type=notify` waits on — and a daemon that resolves the address perfectly
    /// and then sends nothing hangs its unit exactly as loudly as one that never
    /// tried. So this stands up a real listener on a real socket and reads the
    /// bytes back.
    ///
    /// It does NOT reproduce the failure seen on the first hardware run, and
    /// should not be read as covering it: that daemon never reached its
    /// `ready()` call at all. The test that guards *that* is the thread-budget
    /// assertion in `main.rs`.
    #[test]
    fn ready_is_a_datagram_that_actually_arrives() {
        let dir = std::env::temp_dir().join(format!(
            "tv-shell-cec-notify-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::create_dir_all(&dir).expect("a temp dir");
        let path = dir.join("notify");
        let listener = UnixDatagram::bind(&path).expect("binding the fake notify socket");
        listener
            .set_read_timeout(Some(std::time::Duration::from_secs(5)))
            .expect("a bounded read, so a lost datagram fails rather than hangs");

        let notifier = Notifier {
            socket: Some(
                Notifier::open(path.to_str().expect("a utf-8 temp path"))
                    .expect("opening the fake notify socket"),
            ),
        };
        notifier.ready();
        notifier.watchdog();

        let mut buf = [0u8; 64];
        let len = listener
            .recv(&mut buf)
            .expect("READY=1 must arrive; a Type=notify unit hangs at start without it");
        assert_eq!(&buf[..len], b"READY=1\n");

        let len = listener.recv(&mut buf).expect("WATCHDOG=1 must arrive too");
        assert_eq!(&buf[..len], b"WATCHDOG=1\n");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A status line is one line, whatever it is handed.
    #[test]
    fn a_status_message_cannot_forge_extra_protocol_lines() {
        assert_eq!(
            crate::protocol::sanitize_ipc("a\nWATCHDOG=1"),
            "a WATCHDOG=1"
        );
    }
}
