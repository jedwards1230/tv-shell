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
//! # The thread budget, and why it is written down
//!
//! The unit ships `TasksMax=` as a resource fence. A tokio multi-threaded
//! runtime built with the default settings spawns **one worker per CPU**, so the
//! daemon's thread count was a property of whichever box it ran on and the
//! fence's was a constant — two numbers that could never be checked against each
//! other, and on a 16-CPU host they did not agree.
//!
//! What that cost: the runtime's own workers plus the main thread exhausted the
//! cgroup's pid limit, and the next thread the daemon asked for was the tokio
//! blocking-pool thread that backs the `tokio::fs` open inside
//! `AsyncDevice::open`. `clone(2)` returned `EAGAIN`, which tokio classifies as
//! a *temporary* spawn failure — so it did not fail the task, it left it queued
//! for a thread that could never arrive. The first `.await` in startup never
//! returned. Nothing was logged, because nothing had got far enough to log, and
//! `Type=notify` turned a silent hang into a start-job timeout and a restart
//! loop.
//!
//! So the budget is fixed here, in constants, and [`THREAD_BUDGET`] is asserted
//! against the shipped unit's `TasksMax=` by a test. This daemon relays a
//! handful of ioctls and serves one socket; it has no use for a worker per CPU,
//! and a CPU-derived thread count is exactly what it must not have.
//!
//! **This file decides nothing about health.** It performs the probe and asks
//! [`tv_shell_cec::health::Health::should_feed_watchdog`], which is gated on
//! fact 1 — `CEC_ADAP_G_CAPS` round-tripping on our own file descriptor, a pure
//! ioctl that touches the bus not at all — and deliberately not on the derived
//! verdict. The reasoning for that split, and for the other three facts, lives
//! with the state machine.

use std::path::Path;
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

/// Worker threads for the runtime, fixed rather than derived from the CPU count.
///
/// Two, not one: the IPC server and the receive loop are both long-lived, and a
/// single worker makes a slow reply on one of them a stall on the other. Two is
/// also small enough that the budget below stays true on any host, which is the
/// property that matters — see the module docs.
const WORKER_THREADS: usize = 2;

/// Ceiling on tokio's blocking pool, which defaults to 512.
///
/// The daemon's only blocking work is the `tokio::fs` open of the device node at
/// startup. Left at the default the pool is an unbounded hole in the budget, and
/// an unbounded hole is not a budget.
const MAX_BLOCKING_THREADS: usize = 2;

/// Every OS thread this daemon can have at once.
///
/// The main thread, the runtime's workers, the blocking pool at its ceiling, and
/// the two threads `linux-cec` dedicates to the device and to the poller — it
/// relays each ioctl to a thread of its own rather than polling an fd. A test
/// asserts the shipped unit's `TasksMax=` leaves room for all of them.
pub const THREAD_BUDGET: usize = 1 + WORKER_THREADS + MAX_BLOCKING_THREADS + 2;

/// How long the adapter may take to open before startup gives up and says so.
///
/// A bound rather than a plain `.await`, because **the failure this daemon
/// actually suffered was a hang, not an error**: a blocking-pool thread that
/// could not spawn left the very first await queued forever, and a queued await
/// logs nothing. Whatever the next cause of a stuck startup turns out to be,
/// this converts it into a message and a non-zero exit, which `Restart=always`
/// retries — instead of a blank journal and a start-job timeout.
///
/// Well under the unit's `TimeoutStartSec=`, so the daemon is always the one
/// that reports the failure rather than the service manager; a test asserts it.
const OPEN_TIMEOUT: Duration = Duration::from_secs(8);

/// Fallback feed interval when `WATCHDOG_USEC` is unset — i.e. when the binary
/// is run by hand. Nothing is listening then, so the value only bounds a
/// harmless no-op.
const DEFAULT_WATCHDOG_PERIOD: Duration = Duration::from_secs(15);

/// The whole argument surface, in one place so the error path and `--help`
/// cannot drift apart.
const USAGE: &str = "tv-shell-cec [--check-config] [--version|-V] [--help|-h]";

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
    /// Load and validate the config, report, and exit — without opening an
    /// adapter, binding a socket or transmitting anything.
    CheckConfig,
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
        Some("--check-config") => Cli::CheckConfig,
        Some(other) => Cli::Unknown(other.to_string()),
    }
}

/// `--check-config`: judge the file the daemon would read, with the parser the
/// daemon uses, and say so.
///
/// This exists so a configuration-management run can validate `cec.toml`
/// **before** it is moved into place — the shape `tv-shell-core
/// write-session-env` already provides for `core.toml` and Ansible's
/// `template:` consumes as `validate:`. The alternative is what this daemon's
/// strictness would otherwise guarantee: `deny_unknown_fields` at every level
/// plus [`CecConfig::validate`] means a typo'd key is not a defaulted value but
/// a daemon that refuses to start, and a file deployed by a config run is then
/// judged for the first time at the next session start, by which time the run
/// that wrote it has reported success and gone.
///
/// It opens no adapter, binds no socket and transmits nothing, so it is safe to
/// run against a live deployment and on a box that has no adapter at all.
fn check_config() -> ExitCode {
    // `config_path()` then `load_from`, which is exactly `CecConfig::load()`'s
    // body — spelled out so the path that gets REPORTED is the same one that was
    // read, rather than a second resolution that could disagree with the first.
    let path = config::config_path();
    match check_config_at(&path) {
        // STDOUT and `println!`, not the tracing layer, for the same reason
        // `--version` uses it: this line is quoted verbatim in a config run's
        // output, and a timestamp and a level in front of it are noise there.
        Ok(report) => {
            println!("{report}");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("{e}");
            ExitCode::FAILURE
        }
    }
}

/// The pure half of [`check_config`]: everything but the env read and the
/// printing, so a test can exercise the whole path a `validate:` takes.
///
/// Returns the success line. It says **whether** each optional leg is
/// configured and never **what** it is configured with: the file it judges
/// carries a receiver's address and a television's MAC, and the output of a
/// config-management run is not where either belongs.
fn check_config_at(path: &Path) -> anyhow::Result<String> {
    // A MISSING FILE IS A SUCCESS. All-defaults is a valid configuration — the
    // daemon starts on it — so a check that failed on absence would refuse the
    // state of every fresh install.
    let config = CecConfig::load_from(path).map_err(|e| anyhow::anyhow!("config: {e}"))?;
    config.validate()?;
    // `validate` already parsed this; asked again for the answer rather than
    // unwrapping a `Result` whose Ok-ness is an invariant of the line above.
    let ip = config.ip()?;
    let leg = |present: bool| if present { "configured" } else { "absent" };
    Ok(format!(
        "{}: config ok; [avr] {}, [tv] wake-on-lan {}",
        path.display(),
        leg(ip.avr.is_some()),
        leg(ip.tv_wol.is_some()),
    ))
}

fn main() -> ExitCode {
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
        // Also before the runtime, and for the same stated reason as the two
        // above: checking a config file must not need one. This is what an
        // Ansible `template:` names as its `validate:` command, so it runs
        // wherever the file is staged — as an unprivileged user, with no
        // adapter, possibly on a box where /dev/cec0 does not exist.
        Cli::CheckConfig => return check_config(),
        Cli::Unknown(other) => {
            tracing::error!("unknown argument {other:?}; usage: {USAGE}");
            return ExitCode::FAILURE;
        }
    }

    // Before the runtime, because building it is the first thing that can fail
    // for want of a thread — and it must not fail silently.
    check_thread_budget();

    // Built by hand rather than with `#[tokio::main]`, because the attribute's
    // default is a worker per CPU and this daemon's thread count has to be a
    // constant the unit's `TasksMax=` can be checked against. See the module
    // docs for what the CPU-derived default cost.
    //
    // After the argument dispatch above on purpose: `--version` and `--help`
    // must not need a runtime, and a refused argument must not need one either.
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .worker_threads(WORKER_THREADS)
        .max_blocking_threads(MAX_BLOCKING_THREADS)
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            tracing::error!("cannot build the tokio runtime: {e}");
            return ExitCode::FAILURE;
        }
    };
    runtime.block_on(run())
}

async fn run() -> ExitCode {
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

    let backend = match tokio::time::timeout(OPEN_TIMEOUT, open_backend(&config)).await {
        Err(_elapsed) => {
            // Deliberately its own arm, with its own message. A startup that
            // stops making progress used to be indistinguishable from a startup
            // that was merely slow, and the service manager reported it with a
            // line that named nothing.
            tracing::error!(
                "opening the CEC adapter made no progress within {OPEN_TIMEOUT:?}; \
                 giving up rather than hanging the start job. If the thread budget \
                 warning above fired, that is the cause."
            );
            notifier.status("adapter open timed out");
            return ExitCode::FAILURE;
        }
        Ok(Ok(b)) => b,
        Ok(Err(e)) => {
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

/// Say, before the runtime is built, whether the cgroup can hold the daemon.
///
/// The one line this daemon most needed and did not have. Its first hardware run
/// failed because the cgroup's pid limit was below the thread count the runtime
/// wanted, and the shape of that failure is the problem: the thread that could
/// not be created was one tokio asks for on behalf of a blocking task, tokio
/// treats `EAGAIN` as *temporary* rather than fatal, and so the task waited for
/// a thread that would never exist. No error was returned to anyone. Nothing was
/// logged. The journal held nothing but the service manager's own timeout.
///
/// So this reads the limit and says so up front, while there is still a thread
/// to say it with. It never refuses to start: the budget is a ceiling the daemon
/// may not reach, `pids.max` can legitimately be `max`, and a daemon that
/// declines to run on a limit it might have fitted inside is worse than one that
/// warns and tries.
fn check_thread_budget() {
    let Some(limit) = cgroup_pids_max() else {
        tracing::debug!("no cgroup pid limit found; thread budget is {THREAD_BUDGET}");
        return;
    };
    if limit < THREAD_BUDGET {
        tracing::warn!(
            "this cgroup allows {limit} tasks but the daemon needs up to \
             {THREAD_BUDGET} threads; if startup stops here, raise TasksMax= in the unit. \
             A thread that cannot be created does not fail loudly, it waits."
        );
    } else {
        tracing::debug!("cgroup allows {limit} tasks; thread budget is {THREAD_BUDGET}");
    }
}

/// This process's `pids.max`, or `None` when there is no finite limit to read.
///
/// cgroup v2 only, which is what a `systemd --user` service runs under. Every
/// failure is a `None` rather than an error: this is advice, and advice that
/// fails to load must not be able to stop the daemon.
fn cgroup_pids_max() -> Option<usize> {
    let own = std::fs::read_to_string("/proc/self/cgroup").ok()?;
    let path = own.lines().find_map(|l| l.strip_prefix("0::"))?.trim();
    let raw = std::fs::read_to_string(format!("/sys/fs/cgroup{path}/pids.max")).ok()?;
    parse_pids_max(&raw)
}

/// Parse a `pids.max` value: a number, or the literal `max` for "no limit".
fn parse_pids_max(raw: &str) -> Option<usize> {
    match raw.trim() {
        "max" => None,
        n => n.parse().ok(),
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
        // Near-misses of the config check are refused too, and by the same rule:
        // a `validate:` command that was silently the SERVE path would open the
        // adapter on the box staging a config file.
        for near in [
            "--check",
            "--checkconfig",
            "--check-config=x",
            "check-config",
        ] {
            assert_eq!(
                parse_args(&args(&[near])),
                Cli::Unknown(near.to_string()),
                "{near} must be refused, not treated as --check-config"
            );
        }
    }

    /// The `validate:` arm. It has exactly one spelling, because the one thing
    /// naming it is a config-management template, not a human at a prompt.
    #[test]
    fn the_config_check_has_one_spelling_and_takes_no_argument() {
        assert_eq!(parse_args(&args(&["--check-config"])), Cli::CheckConfig);
        // The path comes from `$TV_SHELL_CEC_CONFIG` — the same resolution the
        // daemon uses — so a trailing word is an argument nobody reads, and an
        // ignored path would check a DIFFERENT file from the one named.
        assert_eq!(
            parse_args(&args(&["--check-config", "/tmp/cec.toml"])),
            Cli::CheckConfig
        );
    }

    /// **The rule: the check refuses what the daemon would refuse.**
    ///
    /// This is the whole value of the flag — a document the daemon would abort
    /// on must fail HERE, where the config run can still decline to install it,
    /// rather than at the next session start.
    #[test]
    fn the_config_check_takes_the_same_path_a_validate_would() {
        let dir = std::env::temp_dir().join(format!("tv-cec-check-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();

        // Absent ⇒ ok. All-defaults is a valid configuration, so a fresh install
        // must not be reported as a broken one.
        let missing = dir.join("missing.toml");
        let ok = check_config_at(&missing).unwrap();
        assert!(ok.contains("config ok"), "{ok}");
        assert!(ok.contains("[avr] absent"), "{ok}");
        assert!(ok.contains("[tv] wake-on-lan absent"), "{ok}");

        // A typo'd key: `deny_unknown_fields` is what makes this a startup
        // failure rather than a silently-defaulted value, and the check inherits
        // it because it uses the same parser.
        let typo = dir.join("typo.toml");
        std::fs::write(&typo, "[device]\nphysaddr = \"1.0.0.0\"\n").unwrap();
        let e = check_config_at(&typo).unwrap_err().to_string();
        assert!(e.starts_with("config:"), "{e}");
        assert!(e.contains("physaddr"), "{e}");

        // A well-formed document with a bad VALUE fails too, naming the key —
        // parsing and validation are two gates and the check runs both.
        let bad = dir.join("bad.toml");
        std::fs::write(&bad, "[device]\nphys_addr = \"25.0.0\"\n").unwrap();
        let e = check_config_at(&bad).unwrap_err().to_string();
        assert!(e.contains("[device] phys_addr"), "{e}");

        // A valid document with both optional legs set reports them as present —
        // and reports WHETHER, never WHAT.
        let full = dir.join("full.toml");
        std::fs::write(
            &full,
            "[avr]\nhost = \"192.0.2.10\"\n[tv]\nwol_mac = \"aa:bb:cc:dd:ee:ff\"\n",
        )
        .unwrap();
        let ok = check_config_at(&full).unwrap();
        assert!(ok.contains("[avr] configured"), "{ok}");
        assert!(ok.contains("[tv] wake-on-lan configured"), "{ok}");
        assert!(
            !ok.contains("192.0.2.10"),
            "the report must not echo the file: {ok}"
        );
        assert!(
            !ok.contains("aa:bb:cc:dd:ee:ff"),
            "the report must not echo the file: {ok}"
        );

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_usage_line_names_every_flag_the_parser_accepts() {
        for flag in ["--version", "-V", "--help", "-h", "--check-config"] {
            assert!(USAGE.contains(flag), "usage omits {flag}: {USAGE}");
        }
    }

    /// The unit's resource fence must leave room for every thread the daemon can
    /// have.
    ///
    /// **This is the regression test for the bug that made the first hardware
    /// run fail**, and it is a consistency check rather than a runtime one on
    /// purpose: the defect was two numbers in two files that nothing compared —
    /// a `TasksMax=` constant in the unit and a worker count derived from the
    /// host's CPUs. Reproducing the failure itself needs a cgroup and a box with
    /// enough CPUs to exhaust it; keeping the two numbers honest needs neither,
    /// and it is the half that can drift silently.
    ///
    /// It fails if someone lowers `TasksMax=`, raises a thread constant, or adds
    /// a component that spawns threads without saying so in [`THREAD_BUDGET`].
    #[test]
    fn the_unit_allows_every_thread_the_daemon_can_have() {
        let unit = include_str!("../../core/units/tv-shell-v2-cec.service");
        let tasks_max: usize = unit
            .lines()
            .find_map(|l| l.strip_prefix("TasksMax="))
            .expect("the unit must set TasksMax=")
            .trim()
            .parse()
            .expect("TasksMax= must be a plain number");
        assert!(
            tasks_max >= THREAD_BUDGET,
            "TasksMax={tasks_max} is below the daemon's thread budget of {THREAD_BUDGET}; \
             the runtime cannot start and the failure is a silent hang, not an error"
        );
    }

    /// **The sandbox permits the socket the daemon actually binds.**
    ///
    /// `ProtectSystem=strict` mounts the whole hierarchy read-only except a few
    /// kernel paths, and `$XDG_RUNTIME_DIR` is not among them — so the unit as
    /// first written forbade the one file the daemon must create. It started
    /// cleanly, sent `READY=1`, and then died on `EROFS` binding its socket.
    ///
    /// This is the shape of bug the test exists for: a hardening directive that
    /// is correct in isolation and forbids something the daemon needs. Nothing
    /// compared the two, because the requirement lives in Rust and the
    /// permission lives in a unit file.
    #[test]
    fn the_sandbox_permits_the_socket_the_daemon_binds() {
        let unit = include_str!("../../core/units/tv-shell-v2-cec.service");
        let directive = |key: &str| {
            unit.lines()
                .filter_map(|l| l.trim().strip_prefix(key))
                .next()
                .map(str::trim)
                .map(str::to_string)
        };

        // Only meaningful while something is taking the filesystem away. If the
        // hardening is ever dropped, there is nothing to grant back.
        let read_only = directive("ProtectSystem=").as_deref() == Some("strict");
        if !read_only {
            return;
        }
        let writable = directive("ReadWritePaths=").unwrap_or_default();
        assert!(
            writable.split_whitespace().any(|p| p == "%t"),
            "ProtectSystem=strict makes $XDG_RUNTIME_DIR read-only, but the daemon binds \
             its socket there; the unit must carry ReadWritePaths=%t (got {writable:?})"
        );
    }

    /// The socket stays directly in `$XDG_RUNTIME_DIR`, which is what `%t`
    /// grants.
    ///
    /// The pair to the test above: `ReadWritePaths=%t` grants that one
    /// directory and not a subdirectory of it, so moving the socket deeper
    /// silently reintroduces the same `EROFS`. Whoever moves it has to come
    /// here and change the unit too.
    #[test]
    fn the_socket_sits_directly_in_the_runtime_directory() {
        // The REAL resolver, not a rebuilt copy of it — a test that reassembles
        // the same format string would pass no matter what the daemon does.
        // `$TV_SHELL_CEC_SOCK` is a developer affordance that may point
        // anywhere, so the assertion is about the default the unit must match.
        if std::env::var_os(tv_shell_cec::config::SOCKET_PATH_ENV).is_some() {
            return;
        }
        let path = tv_shell_cec::config::socket_path();
        let parent = std::path::Path::new(&path)
            .parent()
            .expect("the socket path has a directory");
        let runtime_dir = format!("/run/user/{}", unsafe { libc::getuid() });
        assert_eq!(
            parent,
            std::path::Path::new(&runtime_dir),
            "the socket moved out of $XDG_RUNTIME_DIR; ReadWritePaths=%t no longer covers it"
        );
    }

    /// Every address family the daemon opens is permitted.
    ///
    /// `RestrictAddressFamilies=` is the other directive on this unit that can
    /// forbid real behaviour, and unlike the socket bind it would not fail at
    /// startup — it would fail the first time someone asked for a wake, which
    /// is the worst moment to discover it.
    ///
    /// The list is what the code uses, not what the unit happens to say:
    /// `AF_UNIX` for the IPC socket and sd_notify, `AF_INET`/`AF_INET6` for the
    /// Wake-on-LAN broadcast and the AVR control connection, and `AF_NETLINK`
    /// because `getaddrinfo` reaches for it when resolving the AVR's hostname.
    #[test]
    fn the_sandbox_permits_every_address_family_the_daemon_opens() {
        let unit = include_str!("../../core/units/tv-shell-v2-cec.service");
        let allowed: Vec<&str> = unit
            .lines()
            .filter_map(|l| l.trim().strip_prefix("RestrictAddressFamilies="))
            .flat_map(str::split_whitespace)
            .collect();
        assert!(
            !allowed.is_empty(),
            "the unit must state RestrictAddressFamilies= explicitly"
        );
        for family in ["AF_UNIX", "AF_INET", "AF_INET6", "AF_NETLINK"] {
            assert!(
                allowed.contains(&family),
                "the daemon opens {family} sockets but the unit forbids them: {allowed:?}"
            );
        }
    }

    /// The daemon gives up on a stuck open before the service manager gives up
    /// on the daemon.
    ///
    /// Which of the two reports the failure decides whether anyone can read the
    /// cause: the daemon names what it was doing, while the service manager can
    /// only say the start job timed out. That is the whole difference between
    /// the journal from the first hardware run and a useful one.
    ///
    /// `TimeoutStartSec=` is read from the shipped unit rather than assumed.
    /// It is set explicitly there for this reason — left unset it inherits a
    /// manager default that varies by distribution and is invisible in the unit.
    #[test]
    fn the_daemon_reports_a_stuck_open_before_systemd_times_out() {
        let unit = include_str!("../../core/units/tv-shell-v2-cec.service");
        let start_timeout = unit
            .lines()
            .find_map(|l| l.strip_prefix("TimeoutStartSec="))
            .expect("the unit must set TimeoutStartSec= rather than inherit a manager default")
            .trim()
            .strip_suffix('s')
            .and_then(|n| n.parse::<u64>().ok())
            .expect("TimeoutStartSec= must be a plain number of seconds");
        assert!(
            OPEN_TIMEOUT < Duration::from_secs(start_timeout),
            "OPEN_TIMEOUT={OPEN_TIMEOUT:?} is not inside TimeoutStartSec={start_timeout}s, \
             so systemd reports the failure first and the cause goes unlogged"
        );
    }

    /// **No operator-facing message carries a run of stray spaces.**
    ///
    /// A long literal wrapped across source lines keeps the indentation of every
    /// continuation line unless the line ends in a `\`. The compiler is happy,
    /// the source reads correctly, and the message an operator actually sees has
    /// eighteen spaces in the middle of a sentence. That is a poor bug to ship in
    /// any message and a worse one here, because every message in this file is
    /// read at the moment the daemon will not start — the failed-startup text is
    /// the only thing standing between a blank journal and a diagnosis.
    ///
    /// Scanning the source is the point rather than a shortcut: the defect is
    /// created between source and literal, it is invisible in a review diff, and
    /// it cannot be reached at runtime for the arms that need hardware to fire.
    /// This fails on every message in the file, including ones not yet written.
    #[test]
    fn no_message_in_this_file_has_mangled_whitespace() {
        let source = include_str!("main.rs");
        // Built rather than written, so this test's own needle is not a literal
        // containing the run it searches for.
        let run = " ".repeat(3);
        for (n, line) in source.lines().enumerate() {
            // Only inside a literal, and only runs the wrap bug produces. Source
            // indentation itself is leading whitespace, which this skips.
            let Some(quote) = line.find('"') else {
                continue;
            };
            let body = &line[quote + 1..];
            assert!(
                !body.contains(&run),
                "line {} looks like a literal wrapped without a trailing `\\`, so the \
                 rendered message carries stray spaces: {}",
                n + 1,
                line.trim()
            );
        }
    }

    /// `pids.max` is a number or the word `max`, and `max` is not a limit.
    ///
    /// Worth pinning because the wrong reading is the dangerous one: parsing
    /// `max` as a failure would make the check silent on exactly the hosts that
    /// have no limit, and parsing it as zero would make it warn on all of them
    /// until the warning got ignored.
    #[test]
    fn an_unlimited_pids_max_is_not_a_tiny_one() {
        assert_eq!(parse_pids_max("max\n"), None);
        assert_eq!(parse_pids_max("16\n"), Some(16));
        assert_eq!(parse_pids_max("  24  "), Some(24));
        assert_eq!(parse_pids_max("nonsense"), None);
    }
}
