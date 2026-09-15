//! Shared application state threaded through every axum handler via
//! `State<SharedState>`.

use std::sync::Arc;

use crate::bridge::DevBridge;
use crate::capabilities::CapabilitySnapshot;
use crate::config::AppConfig;
use crate::exec::Recovery;
use crate::transport::NodeTransport;
use crate::updates::UpdatesState;

/// The panel's shared state: resolved config plus the three data-tier
/// clients (node transport primary, HTTP bridge dev-ops, direct-exec
/// recovery) and the Updates feature's cache/job state.
pub struct AppState {
    pub cfg: AppConfig,
    /// What the node declared it can do, resolved ONCE before the router was
    /// built (`crate::capabilities::handshake`). Route registration and the
    /// nav both read it, so a link can never point at an unregistered route.
    /// Static by design — see [`crate::capabilities`].
    pub caps: CapabilitySnapshot,
    /// The node this panel speaks for. Held as a trait object so the pages
    /// depend on *what* a node can do, not on the Unix socket that happens to
    /// serve the local one — see [`crate::transport`].
    pub node: Arc<dyn NodeTransport>,
    /// The **v2 AV-control daemon** (`cec/`, `tv-shell-cec`) — a THIRD socket,
    /// separate from the v1 daemon's and the v2 core's (V2_DESIGN §11).
    ///
    /// Held as the same [`NodeTransport`] trait object because the framing is
    /// identical (one command line, one reply line, 4096-byte cap), but it is
    /// **not a node**: it speaks its own small vocabulary and answers
    /// `unknown` to `capabilities`, so only
    /// [`NodeTransport::command_timeout`] is ever called on it and the startup
    /// handshake never touches it. Its page is registered unconditionally and
    /// renders degraded when nothing answers — see [`crate::pages::av`].
    pub av: Arc<dyn NodeTransport>,
    /// The path [`AppState::av`] dials, kept beside it purely so the page can
    /// show which socket it looked at — a wrong path and a stopped daemon look
    /// identical otherwise.
    pub av_sock: std::path::PathBuf,
    /// The daemon's opt-in HTTP dev-ops tier, held as a trait object for the
    /// same reason — see [`crate::bridge::DevBridge`].
    pub bridge: Arc<dyn DevBridge>,
    pub recovery: Recovery,
    pub updates: UpdatesState,
}

/// `Arc`-wrapped state, cloned cheaply into every handler.
pub type SharedState = Arc<AppState>;
