//! game-shell input daemon.
//!
//! Grabs a gamepad exclusively via `EVIOCGRAB`, emits keyboard/mouse events via
//! uinput, and serves a newline-delimited IPC protocol on a Unix socket
//! (see `docs/IPC_PROTOCOL.md`). Formerly a drop-in replacement for the
//! now-deleted `gamepad-input.py`, keeping its wire protocol so the QML shell
//! is unchanged.
//!
//! Runtime topology: the IPC server and signal handling run on a multi-thread
//! tokio runtime; the input subsystem runs on its own OS thread with a
//! current-thread runtime, keeping real-time input timing off the IPC
//! scheduler. The two communicate over an `mpsc` control channel and a
//! `broadcast` event bus.

// Daemon modules live in the library crate (`game_shell_input`); this binary
// only wires them together. (lib+bin split — see lib.rs — so the cross-platform
// modules aren't dead-code on non-Linux hosts where `main` is cfg-excluded.)
#[cfg(target_os = "linux")]
use game_shell_input::{
    bluetooth, http, hyprland, input, ipc, network, power, protocol, session, session_env, state,
    watch,
};

#[cfg(target_os = "linux")]
use game_shell_input::ipc::{ControllerDbState, SharedControllerDbState};

#[cfg(target_os = "linux")]
fn main() -> anyhow::Result<()> {
    use std::sync::Arc;
    use tokio::sync::{broadcast, mpsc, Notify};

    // Load daemon.env before anything else so GAME_SHELL_HTTP_BIND and
    // other vars are available even when the session wrapper didn't source
    // the file (#165).
    session_env::load_daemon_env();

    init_tracing();

    let uid = unsafe { libc::getuid() };
    let sock_path = std::env::var("GAME_SHELL_SOCK")
        .unwrap_or_else(|_| format!("/run/user/{uid}/game-shell-input.sock"));

    let (events_tx, _events_rx) = broadcast::channel::<protocol::Event>(256);
    let (control_tx, control_rx) = mpsc::channel::<state::Control>(64);

    // Dedicated channel for the file-watch actor to signal the input runtime
    // of external settings.json changes. A separate Notify (rather than a
    // broadcast receiver) avoids adding a permanent receiver on the global
    // Event bus that would defeat receiver_count()==0 fast-paths (#163).
    let config_changed = Arc::new(Notify::new());

    // Input subsystem on a dedicated OS thread with its own current-thread
    // runtime (isolated timing).
    let input_events = events_tx.clone();
    let input_config_changed = Arc::clone(&config_changed);
    let input_thread = std::thread::Builder::new()
        .name("input".into())
        .spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("build input runtime");
            rt.block_on(input::run(control_rx, input_events, input_config_changed));
        })?;

    // Re-exec notification: the HTTP /dev/restart-daemon endpoint sets this
    // flag and wakes the main select! so the process can re-exec itself
    // (#167). The Notify is woken by the HTTP handler; the AtomicBool
    // survives past the runtime shutdown so the final re-exec check after
    // input_thread.join() can read it without holding an Arc<Notify>.
    let reexec_notify = std::sync::Arc::new(tokio::sync::Notify::new());
    let reexec_flag = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    // Keep a handle outside the runtime closure for the post-join re-exec check
    // (the `async move` block below moves `reexec_flag` into the HTTP task).
    let reexec_flag_check = reexec_flag.clone();

    // Main runtime: IPC server + signal handling + Phase 3 D-Bus actors.
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    rt.block_on(async move {
        // Spawn the Phase 3 D-Bus actors on the IPC runtime. Each owns its own
        // connection and pushes events onto the shared broadcast bus. They log
        // and never panic the daemon if BlueZ/NetworkManager/logind/UPower are
        // absent, so spawning them unconditionally is safe.
        let dbus = spawn_dbus_actors(&events_tx, &control_tx);

        // Spawn the file-watch actor. It inotify-watches settings.json for
        // external edits and signals the input runtime via config_changed.
        // Fire-and-forget like the D-Bus actors — it logs and degrades
        // gracefully if inotify fails.
        {
            let watch_config_changed = Arc::clone(&config_changed);
            tokio::spawn(async move {
                watch::run(watch_config_changed).await;
            });
        }

        // logind session watcher: releases the gamepad grab while our session is
        // backgrounded (VT-switched away) and re-grabs on return. Fire-and-forget
        // like the D-Bus actors — logs and degrades gracefully if logind is
        // absent (grab simply stays held, the pre-feature behaviour).
        {
            let session_control_tx = control_tx.clone();
            tokio::spawn(async move {
                if let Err(e) = session::run(session_control_tx).await {
                    tracing::warn!("logind session actor exited: {e}");
                }
            });
        }

        // Initialize the controller DB state once at startup. The IPC server
        // shares it across connections (Arc<RwLock<_>>).
        let db_state: SharedControllerDbState =
            std::sync::Arc::new(tokio::sync::RwLock::new(ControllerDbState::initial()));

        let ipc_task = tokio::spawn(ipc::serve(
            sock_path,
            control_tx.clone(),
            events_tx.clone(),
            dbus,
            db_state,
        ));

        // LAN HTTP control bridge (#151): opt-in via GAME_SHELL_HTTP_BIND.
        // When the env var is unset (the default), no socket is opened and no
        // control surface is exposed. When set, it must be a `host:port`
        // address the operator has bound to a trusted LAN interface.
        if let Ok(bind_str) = std::env::var("GAME_SHELL_HTTP_BIND") {
            match bind_str.parse::<std::net::SocketAddr>() {
                Ok(addr) => {
                    let token = std::env::var("GAME_SHELL_HTTP_TOKEN").ok();
                    // Read auth-enabled flag here so it is logged once at startup
                    // alongside the bind address, before the task is spawned.
                    let auth_enabled = http::read_auth_enabled();
                    tokio::spawn(http::serve(
                        addr,
                        token,
                        auth_enabled,
                        control_tx.clone(),
                        events_tx.clone(),
                        reexec_notify.clone(),
                        reexec_flag.clone(),
                    ));
                }
                Err(e) => {
                    tracing::warn!(
                        "GAME_SHELL_HTTP_BIND={bind_str:?} is not a valid host:port address: {e}"
                    );
                }
            }
        }

        tokio::select! {
            _ = wait_for_signal() => {
                tracing::info!("signal received, shutting down");
            }
            _ = reexec_notify.notified() => {
                tracing::info!("re-exec requested, shutting down for restart");
            }
        }

        let _ = control_tx.send(state::Control::Shutdown).await;
        ipc_task.abort();
    });

    // Let the input thread reset stick state and close uinput devices.
    // Gamepad grabs are released in input::run's shutdown path before this
    // returns, so the re-exec'd daemon can re-grab cleanly.
    let _ = input_thread.join();

    // Re-exec if the HTTP /dev/restart-daemon endpoint requested it.
    // Performed AFTER input_thread.join() so pads are released before the
    // new process image starts grabbing them again.
    if reexec_flag_check.load(std::sync::atomic::Ordering::Acquire) {
        use std::os::unix::process::CommandExt;
        // Re-exec the canonical install-path binary, NOT current_exe(): a fresh
        // `/dev/build` replaces the binary inode, so /proc/self/exe resolves to
        // "…/game-shell-input (deleted)" and exec()ing that path fails ENOENT.
        // The install path always points at the just-built binary.
        let exe = session_env::input_bin();
        let err = std::process::Command::new(&exe)
            .args(std::env::args_os().skip(1))
            .exec();
        // exec() only returns on error.
        return Err(anyhow::anyhow!("re-exec failed ({}): {err}", exe.display()));
    }

    Ok(())
}

/// Build the Phase 3 D-Bus actor channels and spawn each actor on the current
/// (IPC) runtime. Returns the [`ipc::DbusSenders`] the IPC server uses to route
/// Bluetooth/Network/Power commands. Channels use the same size (64) as the
/// input control channel. Each actor logs and exits cleanly if its service is
/// absent; the IPC side degrades those commands to `error:*`.
///
/// `control_tx` is a clone of the input runtime's control channel. It is handed
/// to the CEC actor (under `--features cec`) so CEC remote keypresses can be
/// injected as `Control::Key` nav events — gated by `GAME_SHELL_CEC_LIFECYCLE`.
#[cfg(target_os = "linux")]
fn spawn_dbus_actors(
    events_tx: &tokio::sync::broadcast::Sender<protocol::Event>,
    control_tx: &tokio::sync::mpsc::Sender<state::Control>,
) -> ipc::DbusSenders {
    use tokio::sync::mpsc;

    let (bt_tx, bt_rx) = mpsc::channel(64);
    let (net_tx, net_rx) = mpsc::channel(64);
    let (power_tx, power_rx) = mpsc::channel(64);
    let (hypr_tx, hypr_rx) = mpsc::channel(64);
    // HDMI-CEC actor (#94): only spawned when the daemon is built `--features
    // cec`. The module/channel/spawn are all gated so the default build links no
    // libcec.
    #[cfg(feature = "cec")]
    let (cec_tx, cec_rx) = mpsc::channel(64);
    {
        let events_tx = events_tx.clone();
        tokio::spawn(async move {
            if let Err(e) = bluetooth::run(bt_rx, events_tx).await {
                tracing::warn!("bluetooth actor exited: {e}");
            }
        });
    }
    {
        let events_tx = events_tx.clone();
        tokio::spawn(async move {
            if let Err(e) = network::run(net_rx, events_tx).await {
                tracing::warn!("network actor exited: {e}");
            }
        });
    }
    {
        let events_tx = events_tx.clone();
        // Hand the power actor a clone of the CEC channel so logind
        // PrepareForSleep can drive the CEC lifecycle (standby on suspend, wake
        // on resume). Only present under `--features cec`; the CEC actor no-ops
        // these unless GAME_SHELL_CEC_LIFECYCLE is enabled.
        #[cfg(feature = "cec")]
        let power_cec_tx = Some(cec_tx.clone());
        tokio::spawn(async move {
            #[cfg(feature = "cec")]
            let result = power::run(power_rx, events_tx, power_cec_tx).await;
            #[cfg(not(feature = "cec"))]
            let result = power::run(power_rx, events_tx).await;
            if let Err(e) = result {
                tracing::warn!("power actor exited: {e}");
            }
        });
    }
    {
        let events_tx = events_tx.clone();
        tokio::spawn(async move {
            if let Err(e) = hyprland::run(hypr_rx, events_tx).await {
                tracing::warn!("hyprland actor exited: {e}");
            }
        });
    }
    #[cfg(feature = "cec")]
    {
        let events_tx = events_tx.clone();
        // Clone of the input control channel so the CEC actor can inject remote
        // keypresses as nav `Control::Key` events (gated by the lifecycle flag).
        let cec_control_tx = control_tx.clone();
        tokio::spawn(async move {
            if let Err(e) = game_shell_input::cec::run(cec_rx, events_tx, cec_control_tx).await {
                tracing::warn!("cec actor exited: {e}");
            }
        });
    }
    // On the default (no `cec` feature) build, `control_tx` is only used by the
    // CEC actor spawn above, so discard it explicitly to avoid an unused-arg
    // warning under clippy `-D warnings`.
    #[cfg(not(feature = "cec"))]
    let _ = control_tx;

    ipc::DbusSenders {
        bt: Some(bt_tx),
        net: Some(net_tx),
        power: Some(power_tx),
        hypr: Some(hypr_tx),
        #[cfg(feature = "cec")]
        cec: Some(cec_tx),
    }
}

#[cfg(target_os = "linux")]
fn init_tracing() {
    use tracing_subscriber::{fmt, EnvFilter};
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    fmt()
        .with_env_filter(filter)
        .with_target(false)
        .without_time()
        .init();
}

#[cfg(target_os = "linux")]
async fn wait_for_signal() {
    use tokio::signal::unix::{signal, SignalKind};
    let mut term = signal(SignalKind::terminate()).expect("install SIGTERM handler");
    let mut intr = signal(SignalKind::interrupt()).expect("install SIGINT handler");
    tokio::select! {
        _ = term.recv() => {}
        _ = intr.recv() => {}
    }
}

#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("game-shell-input only runs on Linux (requires evdev/uinput).");
    std::process::exit(1);
}
