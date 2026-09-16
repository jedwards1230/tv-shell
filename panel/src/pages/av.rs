//! `/devices/av` — the **v2** AV-control daemon (`cec/`, `tv-shell-cec`):
//! `av-health`, `av-state`, and a placeholder for the `backend` verb step 7 of
//! the plan for jedwards1230/tv-shell#504 fills in.
//!
//! **A different daemon from the CEC page next door.** Devices ▸ CEC drives
//! v1's libcec `cec-*` IPC on the v1 socket; this page reads v2's kernel-CEC
//! daemon on a THIRD socket (`tv-shell-v2-cec.sock`, V2_DESIGN §11). The two
//! are mutually exclusive on the hardware — one exclusive serial port, one
//! owner — so the pages sit beside each other exactly as the daemons do, and
//! neither is written in terms of the other.
//!
//! **Read-only, deliberately.** Every transmit verb this daemon has (`wake`,
//! `standby`, `input-claim`, `input-release`, `input-select`, the `volume`
//! family) puts a message on a shared bus that may carry other playback
//! devices. This page asks three questions and sends nothing, which is also why it
//! is a plain `GET` in the recovery tier with no mutating route to gate.
//!
//! # THE RULE THIS PAGE EXISTS TO OBEY
//!
//! **A caller of that socket must use a bounded connect+read timeout and render
//! a degraded state on expiry — never an indefinite await.** The v2 AV unit's
//! isolation from the session is topological (a bare `Wants=`, no ordering, no
//! failure propagation), and that isolation is only real if its callers treat
//! the socket as unreliable: a caller blocking forever on a wedged CEC backend
//! is the mechanism by which that backend eventually does reach the television.
//! systemd cannot enforce it; it lives here, in [`AV_TIMEOUT`], and it is
//! covered by a test that stands up a socket which accepts and never replies.
//!
//! # And the second rule, from the daemon's side
//!
//! **`unknown` is never rendered as healthy.** `av-health` publishes a
//! tri-state, and the `unknown` arm gets its own (non-ok) dot here. Rendering it
//! green would re-create v1's `cec-health` in the UI after the daemon had gone
//! to the trouble of not committing it.

use std::time::Duration;

use askama::Template;
use axum::extract::State;
use axum::response::{Html, IntoResponse};
use serde_json::Value;

use crate::capabilities::{Chrome, Gate};
use crate::state::{AppState, SharedState};
use crate::transport::{NodeTransport, TransportError};

/// The bound on every call to the v2 AV daemon — connect, write and read.
///
/// Short on purpose: this is a page load, and the answer to "the AV daemon is
/// wedged" is a degraded page now, not a correct page later. It is deliberately
/// well under [`crate::ipc::IpcTransport`]'s 3s default, which is why every call
/// here goes through `command_timeout` rather than `command`.
pub const AV_TIMEOUT: Duration = Duration::from_millis(800);

#[derive(Template)]
#[template(path = "av.html")]
struct AvTemplate {
    chrome: Chrome,
    /// The socket that was dialled, so a wrong path is visible rather than
    /// indistinguishable from a stopped daemon.
    sock: String,
    timeout_ms: u64,
    /// `None` when the daemon did not answer within [`AV_TIMEOUT`].
    health: Option<HealthView>,
    /// Why there is no health, when there is none.
    health_error: String,
    /// `av-state`, flattened to label/value rows.
    state_rows: Vec<Row>,
    state_error: String,
    /// What the `backend` verb answered: which backend is carrying actions,
    /// which exist, and why.
    backend: Option<BackendView>,
    /// Why there is no backend answer, when there is none.
    backend_error: String,
    /// Whether Devices ▸ CEC (v1) is registered on this node.
    ///
    /// The prose here names that page because the two daemons contend for one
    /// adapter, but the link may only be rendered when the route exists: this
    /// page is recovery tier and that one is behind `Feature::Cec`, so with the
    /// v1 daemon down or built without CEC the link would 404.
    cec_page: bool,
}

/// One `av-health` reply, prepared for rendering.
pub struct HealthView {
    /// The raw tri-state token: `healthy`, `degraded` or `unknown`.
    pub state: String,
    /// The dot class for that token. **Never `dot-ok` for `unknown`** — see
    /// [`dot_class`].
    pub dot_class: &'static str,
    pub since: String,
    pub last_tx: String,
    pub last_rx: String,
    pub bus_activity: String,
    pub reason: String,
}

/// One `backend` reply, prepared for rendering.
pub struct BackendView {
    /// Which backend is carrying actions: `cec` or `ip`.
    pub active: String,
    /// The dot class for it. **`ip` is not green**: the IP leg carrying actions
    /// means either that CEC is degraded or that an operator has overridden the
    /// decision, and neither is a steady state to render as fine.
    pub dot_class: &'static str,
    /// Every backend this box has, space-separated.
    pub available: String,
    /// The operator override in force, `auto` when there is none.
    pub pin: String,
    /// Whether an override is in force at all — the page says so explicitly,
    /// because a pinned backend is a decision a person made and forgot.
    pub pinned: bool,
    /// Why `active` is what it is. The daemon's own sentence, verbatim.
    pub reason: String,
}

/// One label/value row of the `av-state` table.
pub struct Row {
    pub label: &'static str,
    pub value: String,
}

pub async fn page(State(state): State<SharedState>) -> impl IntoResponse {
    Html(render_page(&state).await)
}

pub async fn render_page(state: &AppState) -> String {
    // Concurrently, so three bounded calls cost one timeout and not three. Each
    // still carries its own bound; nothing here can await indefinitely.
    let (health, av_state, backend) = tokio::join!(
        ask(state.av.as_ref(), "av-health"),
        ask(state.av.as_ref(), "av-state"),
        ask(state.av.as_ref(), "backend"),
    );
    render(
        &state.caps,
        &state.av_sock.display().to_string(),
        &health,
        &av_state,
        &backend,
    )
}

/// One bounded request. Every failure is a `String` to show the operator; none
/// is an error to propagate, because this page's job is to report what it found.
async fn ask(av: &dyn NodeTransport, line: &str) -> Result<String, String> {
    av.command_timeout(line, AV_TIMEOUT)
        .await
        .map_err(|e| describe(&e))
}

/// Transport failures in the operator's terms.
///
/// The distinction that matters is **timeout vs unreachable**: "nothing is
/// listening" is the normal state on a box that has not taken the operator step
/// yet, while "it accepted the connection and never answered" is a wedged
/// daemon, which is the case the watchdog is about to act on.
fn describe(e: &TransportError) -> String {
    match e {
        TransportError::Timeout => format!(
            "the v2 AV daemon accepted the connection but did not reply within {} ms — \
             it is wedged; systemd's WatchdogSec= should restart it shortly",
            AV_TIMEOUT.as_millis()
        ),
        TransportError::Unreachable => {
            "nothing is listening on the v2 AV socket — the daemon is not running. On a box \
             with no /dev/cec0 that is expected: the unit is ConditionPathExists-gated."
                .to_string()
        }
        other => other.to_string(),
    }
}

fn render(
    caps: &crate::capabilities::CapabilitySnapshot,
    sock: &str,
    health: &Result<String, String>,
    av_state: &Result<String, String>,
    backend: &Result<String, String>,
) -> String {
    let (health_view, health_error) = match health {
        Ok(reply) => match health_view(reply) {
            Some(v) => (Some(v), String::new()),
            None => (
                None,
                format!("the daemon answered `av-health` with something this panel could not read: {reply}"),
            ),
        },
        Err(why) => (None, why.clone()),
    };
    let (state_rows, state_error) = match av_state {
        Ok(reply) => match serde_json::from_str::<Value>(reply) {
            Ok(v) => (state_rows(&v), String::new()),
            Err(e) => (
                Vec::new(),
                format!("unreadable `av-state` reply ({e}): {reply}"),
            ),
        },
        Err(why) => (Vec::new(), why.clone()),
    };
    let (backend_view, backend_error) = match backend {
        Ok(reply) => match backend_view(reply) {
            Some(v) => (Some(v), String::new()),
            None => (
                None,
                format!(
                    "the daemon answered `backend` with something this panel could not read: \
                     {reply}"
                ),
            ),
        },
        Err(why) => (None, why.clone()),
    };
    let tmpl = AvTemplate {
        chrome: Chrome::new(caps, "devices.av"),
        sock: sock.to_string(),
        timeout_ms: AV_TIMEOUT.as_millis() as u64,
        health: health_view,
        health_error,
        state_rows,
        state_error,
        backend: backend_view,
        backend_error,
        cec_page: caps.allows(Gate::Cec),
    };
    tmpl.render()
        .unwrap_or_else(|e| format!("<p class=\"banner banner-error\">render error: {e}</p>"))
}

/// Parse a `backend` reply into its rendered form.
///
/// `None` when the reply is not a backend document — including the `unknown` a
/// daemon built before the verb existed would answer. **No value is invented**:
/// a page claiming "backend: cec" from a daemon that did not say so would be
/// asserting the very thing the verb exists to report.
fn backend_view(reply: &str) -> Option<BackendView> {
    let v: Value = serde_json::from_str(reply).ok()?;
    let active = v.get("active")?.as_str()?.to_string();
    let pin = v
        .get("pin")
        .and_then(Value::as_str)
        .unwrap_or("auto")
        .to_string();
    let available = match v.get("available") {
        Some(Value::Array(items)) if !items.is_empty() => items
            .iter()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>()
            .join(" "),
        _ => active.clone(),
    };
    Some(BackendView {
        dot_class: backend_dot_class(&active),
        pinned: pin != "auto",
        active,
        available,
        pin,
        reason: v
            .get("reason")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
    })
}

/// **The rule: only `cec` is green.**
///
/// The IP leg carrying actions is a working degraded mode, not a steady state —
/// it means the adapter stopped answering, or that somebody pinned it — and a
/// green dot would make the one page that can report a wedged adapter look
/// exactly like the page for a healthy one. A token this panel does not
/// recognise gets the same treatment, for the same reason it does in
/// [`dot_class`].
pub fn backend_dot_class(active: &str) -> &'static str {
    if active == "cec" {
        "dot-ok"
    } else {
        "dot-warn"
    }
}

/// Parse an `av-health` reply into its rendered form.
///
/// `None` when the reply is not a health document, so a protocol change shows as
/// "could not read this" rather than as a blank, healthy-looking panel.
fn health_view(reply: &str) -> Option<HealthView> {
    let v: Value = serde_json::from_str(reply).ok()?;
    let state = v.get("state")?.as_str()?.to_string();
    Some(HealthView {
        dot_class: dot_class(&state),
        state,
        since: age(v.get("sinceMs")),
        last_tx: age(v.get("lastTxOk")),
        last_rx: age(v.get("lastRxMs")),
        bus_activity: age(v.get("busActivityMs")),
        reason: v
            .get("reason")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
    })
}

/// **The rule, in one function: `unknown` is never `dot-ok`.**
///
/// A state token this panel does not recognise gets the same treatment as
/// `unknown` for the same reason — a value we cannot interpret is not evidence
/// of health.
pub fn dot_class(state: &str) -> &'static str {
    match state {
        "healthy" => "dot-ok",
        "degraded" => "dot-error",
        _ => "dot-warn",
    }
}

/// An age in ms as human text. `null` (or absent) renders as an em dash and
/// **never as "0 s"**, because "it has never happened" and "it happened just
/// now" are different facts — the same tri-state discipline the daemon applies
/// on the wire.
fn age(v: Option<&Value>) -> String {
    let Some(ms) = v.and_then(Value::as_u64) else {
        return "—".to_string();
    };
    if ms < 1_000 {
        return format!("{ms} ms");
    }
    let secs = ms / 1_000;
    if secs < 90 {
        return format!("{secs} s");
    }
    let mins = secs / 60;
    if mins < 90 {
        return format!("{mins} min");
    }
    format!("{} h {} min", mins / 60, mins % 60)
}

/// `av-state`, flattened into the table the page renders.
///
/// Hand-listed rather than iterating the JSON object so the labels can say what
/// a key means, and so a `null` renders as an explicit "unknown" rather than
/// disappearing — a missing row would read as "not applicable" when it means
/// "nothing has told us".
fn state_rows(v: &Value) -> Vec<Row> {
    let s = |key: &str| -> String {
        match v.get(key) {
            None | Some(Value::Null) => "unknown".to_string(),
            Some(Value::String(t)) => t.clone(),
            Some(Value::Array(items)) => {
                if items.is_empty() {
                    "none".to_string()
                } else {
                    items
                        .iter()
                        .map(|i| i.as_str().map_or_else(|| i.to_string(), str::to_string))
                        .collect::<Vec<_>>()
                        .join(" ")
                }
            }
            Some(other) => other.to_string(),
        }
    };
    vec![
        Row {
            label: "Device",
            value: s("device"),
        },
        Row {
            label: "Physical address (read back)",
            value: s("physAddr"),
        },
        Row {
            label: "Physical address (configured)",
            value: s("physAddrConfigured"),
        },
        Row {
            label: "Logical addresses",
            value: s("logAddrs"),
        },
        Row {
            label: "Capabilities",
            value: s("capabilities"),
        },
        Row {
            label: "Pin monitor (CEC_CAP_MONITOR_PIN)",
            value: s("monitorPin"),
        },
        Row {
            label: "TV power",
            value: s("tvPower"),
        },
        Row {
            label: "AVR power",
            value: s("avrPower"),
        },
        Row {
            label: "Active source",
            value: s("activeSource"),
        },
        Row {
            label: "We are the source",
            value: s("weAreSource"),
        },
        Row {
            label: "Display ownership",
            value: v
                .get("displayOwnership")
                .and_then(|o| o.get("state"))
                .and_then(Value::as_str)
                .unwrap_or("unknown")
                .to_string(),
        },
        Row {
            label: "Volume",
            value: s("volume"),
        },
        Row {
            label: "Muted",
            value: s("muted"),
        },
        Row {
            label: "Dropped messages",
            value: s("lostMessages"),
        },
    ]
}
