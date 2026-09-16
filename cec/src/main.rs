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
//! 6. Probe the adapter on a timer, record what it observed, and feed
//!    `WATCHDOG=1` while — and only while — the health machine says to.
//! 7. Serve until a signal.
//!
//! # The watchdog, stated plainly
//!
//! The unit ships `WatchdogSec=30s`, so a daemon that does not feed it is
//! SIGABRTed and restarted every 30 s. **That is the only supervisor on this
//! box with restart authority over this daemon** (V2_DESIGN §9), and it is what
//! retires the Ansible CEC watchdog rather than merely disabling it: no polling
//! script, no `cec-health` probe with bus side effects, no second mechanism that
//! can "recover" a daemon that was never broken.
//!
//! **This file decides nothing about health.** It performs the probe and asks
//! [`tv_shell_cec::health::Health::should_feed_watchdog`], which is gated on
//! fact 1 — `CEC_ADAP_G_CAPS` round-tripping on our own file descriptor, a pure
//! ioctl that touches the bus not at all — and deliberately not on the derived
//! verdict. The reasoning for that split, and for the other three facts, lives
//! with the state machine.

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

/// The whole argument surface, in one place so the error path and `--help`
/// cannot drift apart.
const USAGE: &str = "tv-shell-cec [--version|-V] [--help|-h]";

/// What the command line asked for.
///
/// A pure decision, split out of `main` so the dispatch is testable without a
/// process: `main` is `#[tokio::main]` and opens an adapter, so nothing about
/// it can be exercised from a unit test.
#[derive(Debug, PartialEq, Eq)]
enum Cli {
    /// No arguments: open the adapter and serve. What the unit does.
    Serve,
    /// Print the version line and exit 0.
    Version,
    /// Print the usage line and exit 0.
    Help,
    /// Anything else, carried so the message can name it.
    Unknown(String),
}

/// Parse the argument list (already stripped of argv[0]).
///
/// UNKNOWN ARGUMENTS ARE REFUSED rather than ignored. Silently serving despite
/// an argument nobody understood is how a typo'd flag becomes a daemon running
/// with settings the operator believes it has.
///
/// This is safe for the unit: `tv-shell-v2-cec.service` starts
/// `@TV_SHELL_V2_PREFIX@/bin/tv-shell-cec` with NO arguments at all, so the
/// only path systemd ever takes is `Serve`. A refusal can only be reached by a
/// human typing one.
fn parse_args(args: &[String]) -> Cli {
    match args.first().map(String::as_str) {
        None => Cli::Serve,
        Some("--version" | "-V") => Cli::Version,
        Some("--help" | "-h") => Cli::Help,
        Some(other) => Cli::Unknown(other.to_string()),
    }
}

#[tokio::main]
async fn main() -> ExitCode {
    init_tracing();

    match parse_args(&std::env::args().skip(1).collect::<Vec<_>>()) {
        Cli::Serve => {}
        // STDOUT, not the tracing layer: `init_tracing` writes to stderr with a
        // timestamp and a level in front of every line, and `--version` is the
        // one output an operator pipes or diffs against the core's.
        Cli::Version => {
            println!("{}", tv_shell_cec::version::version_string());
            return ExitCode::SUCCESS;
        }
        Cli::Help => {
            println!("{USAGE}");
            return ExitCode::SUCCESS;
        }
        Cli::Unknown(other) => {
            tracing::error!("unknown argument {other:?}; usage: {USAGE}");
            return ExitCode::FAILURE;
        }
    }

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

    let watchdog = watchdog_feed(notifier, backend.probe);

    let outcome = tokio::select! {
        result = server => match result {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                tracing::error!("ipc server: {e:#}");
                ExitCode::FAILURE
            }
        },
        () = watchdog => {
            // Only reachable when the probe is absent, which cannot
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
    /// Observe the health facts and answer "should `WATCHDOG=1` be sent?".
    ///
    /// The **decision** is the health machine's; this closure only carries the
    /// backend that can observe. `None` for a backend with no device, which
    /// cannot happen with a real one.
    probe: Option<Box<dyn Fn() -> ProbeFuture + Send + Sync>>,
}

type ProbeFuture = std::pin::Pin<Box<dyn std::future::Future<Output = bool> + Send>>;

#[cfg(target_os = "linux")]
async fn open_backend(config: &CecConfig) -> anyhow::Result<Opened> {
    use tv_shell_cec::kernel::{follower, KernelBackend};

    let backend = Arc::new(KernelBackend::open(config).await?);
    let device = backend.device();
    let observations = backend.observations();

    let health = backend.health();
    let failover = backend.failover();

    let prober = Arc::clone(&backend);
    Ok(Opened {
        ipc: Arc::clone(&backend) as Arc<dyn AvBackend>,
        receive_loop: Some(Box::pin(follower::run(
            device,
            observations,
            health,
            failover,
        ))),
        probe: Some(Box::new(move || {
            let prober = Arc::clone(&prober);
            Box::pin(async move { prober.probe().await })
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

/// Probe the adapter on a timer, and feed `WATCHDOG=1` while the health machine
/// says to.
///
/// When it says not to, the feed simply **stops**. This function does not exit,
/// kill anything, or log once a second: systemd's `WatchdogSec=` already owns
/// the response, and a second mechanism with restart authority over the same
/// process is the §9 "only one supervisor" rule being broken. A SIGABRT from the
/// service manager, on a bounded timer, is the whole answer to a hung backend.
///
/// The probe itself is where the loop can hang — an ioctl that never returns —
/// and that is the case this whole arrangement exists for: no feed goes out, and
/// systemd restarts the unit.
async fn watchdog_feed(
    notifier: Notifier,
    probe: Option<Box<dyn Fn() -> ProbeFuture + Send + Sync>>,
) {
    let Some(probe) = probe else {
        return;
    };
    let period = watchdog_period();
    tracing::debug!(
        "probing the adapter every {period:?}; feeding the watchdog while fact 1 holds"
    );
    let mut healthy = true;
    loop {
        tokio::time::sleep(period).await;
        if probe().await {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn args(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn no_arguments_serves_which_is_what_the_unit_does() {
        // The load-bearing case. `ExecStart=` passes nothing, so any parse that
        // made the empty list an error would be a unit that never starts.
        assert_eq!(parse_args(&args(&[])), Cli::Serve);
    }

    #[test]
    fn both_version_spellings_are_accepted() {
        assert_eq!(parse_args(&args(&["--version"])), Cli::Version);
        assert_eq!(parse_args(&args(&["-V"])), Cli::Version);
        // Lowercase `-v` is conventionally verbosity, not version, and this
        // daemon's verbosity is `RUST_LOG`. It must NOT be a silent alias.
        assert_eq!(
            parse_args(&args(&["-v"])),
            Cli::Unknown("-v".to_string()),
            "-v must be refused, not quietly treated as --version"
        );
    }

    #[test]
    fn an_unknown_argument_is_refused_and_named() {
        // Refused, not ignored: a daemon that serves anyway is a daemon running
        // with settings the operator believes it has.
        assert_eq!(
            parse_args(&args(&["--adapter=/dev/cec1"])),
            Cli::Unknown("--adapter=/dev/cec1".to_string())
        );
    }

    #[test]
    fn the_usage_line_names_every_flag_the_parser_accepts() {
        for flag in ["--version", "-V", "--help", "-h"] {
            assert!(USAGE.contains(flag), "usage omits {flag}: {USAGE}");
        }
    }
}
