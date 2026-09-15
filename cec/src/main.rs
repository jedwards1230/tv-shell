//! `tv-shell-cec` entry point.
//!
//! Startup order, and why each step is where it is:
//!
//! 1. Load and **validate** config before anything uses a value from it — a bad
//!    `phys_addr` fails here, naming the key, not at the first ioctl.
//! 2. Open and configure the adapter, and read its topology back. The startup
//!    sequence itself transmits nothing (see [`tv_shell_cec::kernel::device`]);
//!    transmits happen only when a client asks for one.
//! 3. Bind the IPC socket and start serving.
//! 4. Start the receive loop.
//! 5. **Then** send `READY=1`. The unit is `Type=notify`, so this is what
//!    completes its start job — and it is sent last on purpose: a `READY=1` sent
//!    before the socket exists would tell systemd the daemon was serving while a
//!    client connecting on that promise still got `ENOENT`.
//! 6. Feed `WATCHDOG=1` while, and only while, the adapter fd answers.
//! 7. Serve until a signal.
//!
//! # The watchdog, stated plainly
//!
//! The unit ships `WatchdogSec=30s`, so a daemon that does not feed it is
//! SIGABRTed and restarted every 30 s. That means the feed cannot wait for step
//! 6 of the plan — shipping the directive without the message would be shipping
//! a unit that self-kills. So the minimal feed is here now, gated on exactly the
//! plan's fact 1: `CEC_ADAP_G_CAPS` round-trips. It is a pure ioctl on our own
//! file descriptor and touches the bus not at all.
//!
//! What is deliberately NOT here is the rest of step 6 — the health state
//! machine over the four observed facts, and the `av-health` verb that publishes
//! them. This feed answers one question ("is the fd answering?") and makes no
//! claim about the bus. That distinction is the whole lesson of v1's
//! `cec-health`, which inferred adapter health from the outcome of our own
//! transmits and was then inferred from a second time by a watchdog reading IPC
//! reachability — which is how a deliberately stopped daemon read as a wedged
//! adapter and got "recovered" three times.

use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use tv_shell_cec::backend::AvBackend;
use tv_shell_cec::config::{self, CecConfig};
use tv_shell_cec::ipc;
use tv_shell_cec::notify::Notifier;

/// systemd's own rule: feed at half the configured interval, so one missed or
/// delayed feed is not immediately fatal.
const WATCHDOG_DIVISOR: u32 = 2;

/// Fallback feed interval when `WATCHDOG_USEC` is unset — i.e. when the binary
/// is run by hand. Nothing is listening then, so the value only bounds a
/// harmless no-op.
const DEFAULT_WATCHDOG_PERIOD: Duration = Duration::from_secs(15);

#[tokio::main]
async fn main() -> ExitCode {
    init_tracing();

    let config = match CecConfig::load() {
        Ok(c) => c,
        Err(e) => {
            tracing::error!("{e}");
            return ExitCode::FAILURE;
        }
    };
    if let Err(e) = config.validate() {
        tracing::error!("{e}");
        return ExitCode::FAILURE;
    }

    let notifier = Notifier::from_env();

    let backend = match open_backend(&config).await {
        Ok(b) => b,
        Err(e) => {
            // Fatal, and loudly so. The unit is `ConditionPathExists=/dev/cec0`
            // gated, so reaching here means the node exists and the adapter
            // still would not configure — which is a real fault, not a box
            // without an adapter. `Restart=always` retries it on a bounded
            // timer, and nothing else in the session waits on this unit.
            tracing::error!("cannot open the CEC adapter: {e:#}");
            notifier.status(&format!("adapter unavailable: {e}"));
            return ExitCode::FAILURE;
        }
    };

    let sock_path = config::socket_path();
    let server = ipc::serve(sock_path.clone(), Arc::clone(&backend.ipc));

    // The receive loop. Spawned rather than selected on, because it is the
    // thing being supervised: if it returns, the daemon has stopped listening,
    // and the watchdog feed below is what turns that into a restart.
    if let Some(rx) = backend.receive_loop {
        tokio::spawn(rx);
    }

    // Everything is up: the socket is bound and the loop is running.
    notifier.ready();
    notifier.status("listening; power, input and volume verbs available");
    tracing::info!("ready; serving on {sock_path}");

    let watchdog = watchdog_feed(notifier, backend.liveness);

    let outcome = tokio::select! {
        result = server => match result {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                tracing::error!("ipc server: {e:#}");
                ExitCode::FAILURE
            }
        },
        () = watchdog => {
            // Only reachable when the liveness probe is absent, which cannot
            // happen with a real backend. Kept as a branch rather than an
            // `unreachable!()` so a future refactor cannot turn it into a panic.
            tracing::warn!("the watchdog feed ended; shutting down");
            ExitCode::FAILURE
        }
        signal = terminate() => {
            tracing::info!("{signal}; shutting down");
            ExitCode::SUCCESS
        }
    };

    // Unlink our own socket. Best-effort: the file may already be gone, and a
    // failure here must not turn a clean shutdown into a failed one.
    //
    // It matters more here than it looks. v1's daemon did NOT unlink its socket,
    // so a stale node outlived every stop — and the CEC watchdog's precondition
    // tested for that node's existence, read the resulting timeouts as
    // "unreachable", and "recovered" a daemon that was never broken. A daemon
    // that cleans up after itself is half of not repeating that.
    if let Err(e) = std::fs::remove_file(&sock_path) {
        if e.kind() != std::io::ErrorKind::NotFound {
            tracing::warn!("removing {sock_path}: {e}");
        }
    }
    outcome
}

/// What `main` needs out of whichever backend it opened.
struct Opened {
    /// The backend the IPC layer answers from.
    ipc: Arc<dyn AvBackend>,
    /// The receive loop to spawn, if this backend has one.
    receive_loop: Option<std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>>>,
    /// The watchdog's liveness probe — fact 1, and nothing else.
    liveness: Option<Box<dyn Fn() -> LivenessFuture + Send + Sync>>,
}

type LivenessFuture = std::pin::Pin<Box<dyn std::future::Future<Output = bool> + Send>>;

#[cfg(target_os = "linux")]
async fn open_backend(config: &CecConfig) -> anyhow::Result<Opened> {
    use tv_shell_cec::kernel::{follower, KernelBackend};

    let backend = Arc::new(KernelBackend::open(config).await?);
    let device = backend.device();
    let observations = backend.observations();

    let probe = Arc::clone(&backend);
    Ok(Opened {
        ipc: Arc::clone(&backend) as Arc<dyn AvBackend>,
        receive_loop: Some(Box::pin(follower::run(device, observations))),
        liveness: Some(Box::new(move || {
            let probe = Arc::clone(&probe);
            Box::pin(async move { probe.fd_is_alive().await })
        })),
    })
}

/// There is no CEC device outside Linux, and this daemon has nothing to say
/// without one.
///
/// It refuses to start rather than serving an `av-state` full of `null`s: a
/// reply that looks like a quiet bus, from a daemon that has no bus at all, is
/// exactly the kind of confident-and-wrong answer this crate exists to avoid.
#[cfg(not(target_os = "linux"))]
async fn open_backend(_config: &CecConfig) -> anyhow::Result<Opened> {
    anyhow::bail!("the kernel CEC API (/dev/cecN) exists only on Linux")
}

/// Feed `WATCHDOG=1` for as long as the adapter fd answers.
///
/// When the probe fails the feed simply **stops**. It does not exit, kill
/// anything, or log once a second: systemd's `WatchdogSec=` already owns the
/// response, and a second mechanism with restart authority over the same process
/// is the §9 "only one supervisor" rule being broken. A SIGABRT from the service
/// manager, on a bounded timer, is the whole answer to a hung backend.
async fn watchdog_feed(
    notifier: Notifier,
    liveness: Option<Box<dyn Fn() -> LivenessFuture + Send + Sync>>,
) {
    let Some(liveness) = liveness else {
        return;
    };
    let period = watchdog_period();
    tracing::debug!("feeding the systemd watchdog every {period:?} while the adapter fd answers");
    let mut healthy = true;
    loop {
        tokio::time::sleep(period).await;
        if liveness().await {
            if !healthy {
                tracing::info!("the adapter fd is answering again; resuming the watchdog feed");
                healthy = true;
            }
            notifier.watchdog();
        } else if healthy {
            // Once, on the edge. The daemon is about to be restarted by
            // systemd; a message per interval would just be noise in the
            // journal at the moment someone is reading it.
            tracing::error!(
                "the adapter fd has stopped answering; no longer feeding the watchdog, so \
                 systemd will restart this unit"
            );
            notifier.status("adapter fd not answering");
            healthy = false;
        }
    }
}

/// Half of `$WATCHDOG_USEC`, which systemd sets from the unit's `WatchdogSec=`.
///
/// Read from the environment rather than hardcoded, so the unit is the single
/// place the interval is written and changing it there is enough.
fn watchdog_period() -> Duration {
    let Ok(usec) = std::env::var("WATCHDOG_USEC") else {
        return DEFAULT_WATCHDOG_PERIOD;
    };
    match usec.parse::<u64>() {
        Ok(usec) if usec > 0 => Duration::from_micros(usec / u64::from(WATCHDOG_DIVISOR)),
        _ => {
            tracing::warn!("WATCHDOG_USEC={usec:?} is not a positive number of microseconds");
            DEFAULT_WATCHDOG_PERIOD
        }
    }
}

/// Resolve when the process is asked to stop, naming which signal did it.
async fn terminate() -> &'static str {
    use tokio::signal::unix::{signal, SignalKind};
    // A failure to install a handler is not worth aborting a working daemon
    // over, so it degrades to "this signal will not be caught".
    let mut sigterm = match signal(SignalKind::terminate()) {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!("cannot handle SIGTERM: {e}");
            return std::future::pending().await;
        }
    };
    let mut sigint = match signal(SignalKind::interrupt()) {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!("cannot handle SIGINT: {e}");
            sigterm.recv().await;
            return "SIGTERM";
        }
    };
    tokio::select! {
        _ = sigterm.recv() => "SIGTERM",
        _ = sigint.recv() => "SIGINT",
    }
}

/// Structured logs to stderr, filtered by `RUST_LOG` (default `info`).
///
/// journald capture comes from the unit, not from a journald layer here: the
/// daemon runs under `systemd --user` and stderr is already routed. Keeping the
/// binary journald-free means it also runs readably from a shell.
fn init_tracing() {
    use tracing_subscriber::EnvFilter;
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .init();
}
