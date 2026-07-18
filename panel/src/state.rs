//! Shared application state threaded through every axum handler via
//! `State<SharedState>`.

use std::sync::Arc;

use crate::bridge::BridgeClient;
use crate::config::AppConfig;
use crate::exec::Recovery;
use crate::ipc::IpcClient;

/// The panel's shared state: resolved config plus the three data-tier
/// clients (IPC primary, HTTP bridge dev-ops, direct-exec recovery).
pub struct AppState {
    pub cfg: AppConfig,
    pub ipc: IpcClient,
    pub bridge: BridgeClient,
    pub recovery: Recovery,
}

/// `Arc`-wrapped state, cloned cheaply into every handler.
pub type SharedState = Arc<AppState>;
