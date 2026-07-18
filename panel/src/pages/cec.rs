//! `/cec` — HDMI-CEC topology (`cec-scan`/`cec-device`), input switching
//! (`cec-active-source`, per-device `cec-power-on`/`-off`), and the
//! transmit-wedge health/recovery flow (`cec-health`/`cec-test` plus an
//! escalating "Recover CEC" ladder: test → restart daemon → full reboot).
//!
//! **Feature-gated.** CEC is Linux-only and requires the daemon to be built
//! `--features cec`; on a default build every `cec-*` command replies
//! `error:unsupported on this platform` (feature/platform off) or
//! `error:libcec unavailable` (adapter absent/wedged). Both map to an honest
//! "not available" banner here — never a raw failure banner — via
//! [`cec_unavailable_reason`].
//!
//! Degradation: `GET /cec` makes exactly one eager IPC call (`cec-health`,
//! explicitly read-only and cheap per `docs/IPC_PROTOCOL.md`); topology/scan
//! and switching are lazy, htmx-triggered, mirroring `pages::tools`'s
//! no-page-load-IPC philosophy for anything that touches the bus. Always 200,
//! never a 500.

use askama::Template;
use axum::extract::State;
use axum::response::{Html, IntoResponse};
use axum::Form;
use serde::Deserialize;
use serde_json::Value;

use crate::bridge::BridgeError;
use crate::state::{AppState, SharedState};

// ---------------------------------------------------------------------------
// Shared small helpers (own copy per page — see `pages::controllers`'s doc
// comment for why).
// ---------------------------------------------------------------------------

fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn pretty_json_text(s: &str) -> String {
    match serde_json::from_str::<Value>(s) {
        Ok(v) => serde_json::to_string_pretty(&v).unwrap_or_else(|_| s.to_string()),
        Err(_) => s.to_string(),
    }
}

fn pretty_block(reply: &str) -> String {
    format!("<pre>{}</pre>", esc(&pretty_json_text(reply)))
}

#[derive(Template)]
#[template(path = "cec_result.html")]
struct CecResultTemplate {
    ok: bool,
    body_html: String,
}

fn result_html(ok: bool, body_html: &str) -> String {
    let tmpl = CecResultTemplate {
        ok,
        body_html: body_html.to_string(),
    };
    tmpl.render()
        .unwrap_or_else(|e| format!("<p class=\"banner banner-error\">render error: {e}</p>"))
}

fn error_result(msg: &str) -> String {
    result_html(false, &format!("<pre>{}</pre>", esc(msg)))
}

/// Map the two well-known "CEC isn't there" error strings
/// (`docs/IPC_PROTOCOL.md` § HDMI-CEC Commands) to an honest, non-alarming
/// message. `None` for any other (genuine) failure, which callers still
/// render as a normal error.
fn cec_unavailable_reason(msg: &str) -> Option<&'static str> {
    if msg.contains("unsupported on this platform") {
        Some(
            "CEC not available in this daemon build (built without --features cec, \
             or running on a non-Linux host).",
        )
    } else if msg.contains("libcec unavailable") {
        Some(
            "CEC adapter unavailable right now — see the Health panel above for the \
             specific cause and recovery steps.",
        )
    } else {
        None
    }
}

/// Send a `cec-*` command and render the reply, mapping the two "CEC isn't
/// there" errors to an honest info banner instead of a failure banner.
async fn run_cec(state: &AppState, line: &str) -> String {
    match state.ipc.command(line).await {
        Ok(reply) => result_html(true, &pretty_block(&reply)),
        Err(e) => {
            let msg = e.to_string();
            match cec_unavailable_reason(&msg) {
                Some(friendly) => result_html(
                    true,
                    &format!("<p class=\"banner banner-warn\">{}</p>", esc(friendly)),
                ),
                None => error_result(&msg),
            }
        }
    }
}

/// `<addr>` must be a decimal logical address in `0..=15`
/// (`docs/IPC_PROTOCOL.md` § `cec-device`/`cec-power-on`/`cec-power-off`).
fn validate_addr(s: &str) -> Result<u8, String> {
    let n: i64 = s
        .trim()
        .parse()
        .map_err(|_| format!("addr must be an integer 0-15: {s:?}"))?;
    if !(0..=15).contains(&n) {
        return Err(format!("addr must be between 0 and 15 (got {n})"));
    }
    Ok(n as u8)
}

/// Friendly names for the 16 CEC logical addresses (CEC 1.4 device-type
/// table), used as a fallback when `cecDeviceNames` has no override for that
/// address. Mirrors what `AVControlSettings.qml` derives client-side.
fn default_device_name(addr: i64) -> &'static str {
    match addr {
        0 => "TV",
        1 => "Recorder 1",
        2 => "Recorder 2",
        3 => "Tuner 1",
        4 => "Playback 1",
        5 => "Audio System",
        6 => "Tuner 2",
        7 => "Tuner 3",
        8 => "Playback 2",
        9 => "Recorder 3",
        10 => "Tuner 4",
        11 => "Playback 3",
        12 => "Reserved 1",
        13 => "Reserved 2",
        14 => "Free Use",
        15 => "Unregistered/Broadcast",
        _ => "Unknown",
    }
}

// ---------------------------------------------------------------------------
// Health & wedge recovery — its own `#cec-health` partial, re-rendered after
// Test / restart-daemon so the panel always reflects the current state.
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct CecHealthJson {
    transmit: String,
    reason: Option<String>,
    since: i64,
    #[serde(rename = "lastError")]
    last_error: Option<String>,
}

struct HealthView {
    banner_class: &'static str,
    headline: String,
    detail: String,
    recommend_test: bool,
    recommend_restart: bool,
    recommend_reboot: bool,
    /// `false` only for the `no_libcec` case (a build/platform gap, not a
    /// runtime fault) — hides the recovery buttons entirely in favor of a
    /// plain explanatory note, since restarting the daemon or rebooting the
    /// box can't fix a missing feature flag.
    cec_available: bool,
}

fn since_ago(ms: i64) -> String {
    if ms <= 0 {
        return String::new();
    }
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(ms);
    let delta_s = (now_ms - ms).max(0) / 1000;
    let ago = if delta_s < 60 {
        format!("{delta_s}s ago")
    } else if delta_s < 3600 {
        format!("{}m ago", delta_s / 60)
    } else {
        format!("{}h ago", delta_s / 3600)
    };
    format!(" (since {ago})")
}

/// Classify a `cec-health`/`cec-test` reply into a display-ready view.
/// Mirrors the `transmit`/`reason` state table in `docs/IPC_PROTOCOL.md`
/// § `cec-health`.
fn classify_health(h: &CecHealthJson) -> HealthView {
    let since_txt = since_ago(h.since);
    match (h.transmit.as_str(), h.reason.as_deref()) {
        ("ok", _) => HealthView {
            banner_class: "banner-ok",
            headline: "CEC: healthy".to_string(),
            detail: format!("Last transmit succeeded{since_txt}."),
            recommend_test: false,
            recommend_restart: false,
            recommend_reboot: false,
            cec_available: true,
        },
        ("failing", _) => HealthView {
            banner_class: "banner-error",
            headline: "CEC: transmit wedge".to_string(),
            detail: format!(
                "The adapter can receive but every transmit is failing{since_txt}.{}",
                h.last_error
                    .as_deref()
                    .map(|e| format!(" Last error: {e}"))
                    .unwrap_or_default()
            ),
            recommend_test: true,
            recommend_restart: true,
            recommend_reboot: false,
            cec_available: true,
        },
        ("unavailable", Some("no_libcec")) => HealthView {
            banner_class: "banner-warn",
            headline: "CEC not available in this daemon build".to_string(),
            detail: "Built without --features cec, or running on a non-Linux host.".to_string(),
            recommend_test: false,
            recommend_restart: false,
            recommend_reboot: false,
            cec_available: false,
        },
        ("unavailable", Some("no_adapter")) => HealthView {
            banner_class: "banner-error",
            headline: "No CEC adapter detected".to_string(),
            detail: "libcec found zero adapters — check the physical HDMI-CEC adapter and cable, \
                      then re-test."
                .to_string(),
            recommend_test: true,
            recommend_restart: false,
            recommend_reboot: true,
            cec_available: true,
        },
        ("unavailable", Some("adapter_open_failed")) => HealthView {
            banner_class: "banner-error",
            headline: "CEC adapter not responding (hardware wedge)".to_string(),
            detail: "An adapter is present but the libcec open handshake failed. Re-seat the \
                      adapter, then restart the daemon."
                .to_string(),
            recommend_test: true,
            recommend_restart: true,
            recommend_reboot: true,
            cec_available: true,
        },
        ("unavailable", reason) => HealthView {
            banner_class: "banner-error",
            headline: "CEC unavailable".to_string(),
            detail: format!("reason: {}", reason.unwrap_or("unknown")),
            recommend_test: true,
            recommend_restart: true,
            recommend_reboot: true,
            cec_available: true,
        },
        (other, _) => HealthView {
            banner_class: "banner-warn",
            headline: format!("CEC: {other}"),
            detail: format!(
                "No transmit has been attempted yet{since_txt}. Run Test CEC to probe the bus."
            ),
            recommend_test: true,
            recommend_restart: false,
            recommend_reboot: false,
            cec_available: true,
        },
    }
}

#[derive(Template)]
#[template(path = "cec_health.html")]
struct CecHealthTemplate {
    view: HealthView,
    action_ok: bool,
    action_msg: String,
}

/// Run `cmd` (`cec-health` or `cec-test`, same reply shape), classify it, and
/// render the health section, optionally with an `action_msg` banner from
/// whatever triggered the re-render (e.g. a restart-daemon attempt).
async fn render_health_view(state: &AppState, cmd: &str, action: Option<(bool, String)>) -> String {
    let view = match state.ipc.command_json::<CecHealthJson>(cmd).await {
        Ok(h) => classify_health(&h),
        Err(e) => HealthView {
            banner_class: "banner-error",
            headline: "CEC health check failed".to_string(),
            detail: e.to_string(),
            recommend_test: true,
            recommend_restart: false,
            recommend_reboot: false,
            cec_available: true,
        },
    };
    let (action_ok, action_msg) = action.unwrap_or((true, String::new()));
    let tmpl = CecHealthTemplate {
        view,
        action_ok,
        action_msg,
    };
    tmpl.render()
        .unwrap_or_else(|e| format!("<p class=\"banner banner-error\">render error: {e}</p>"))
}

async fn render_health_section(state: &AppState) -> String {
    render_health_view(state, "cec-health", None).await
}

/// `POST /cec/test` — step 1 of the recovery ladder: `cec-test`, a
/// side-effect-free poll probe that also refreshes the health state.
pub async fn test(State(state): State<SharedState>) -> impl IntoResponse {
    Html(render_test(&state).await)
}

pub async fn render_test(state: &AppState) -> String {
    render_health_view(state, "cec-test", None).await
}

/// `true` only when the bridge has no base URL / can't be reached at all —
/// as opposed to being reachable but the operation itself failing. Mirrors
/// `pages::dev`'s private `bridge_unavailable` (duplicated here rather than
/// imported — `pages::dev` is owned by M1, not this page).
fn bridge_unavailable(e: &BridgeError) -> bool {
    matches!(e, BridgeError::NotConfigured | BridgeError::Unreachable(_))
}

/// `POST /cec/recover/restart-daemon` — step 2 of the recovery ladder:
/// restart the input daemon (bridge first, falling back to direct exec, same
/// tier logic as `pages::dev::restart_daemon` — CEC re-initializes on daemon
/// start), then re-fetch and show the fresh health state.
pub async fn recover_restart_daemon(State(state): State<SharedState>) -> impl IntoResponse {
    Html(render_recover_restart_daemon(&state).await)
}

pub async fn render_recover_restart_daemon(state: &AppState) -> String {
    let (ok, text) = match state.bridge.restart_daemon().await {
        Ok(body) => (true, format!("Bridge — restart-daemon: ok\n{body}")),
        Err(e) if bridge_unavailable(&e) => match state.recovery.restart_daemon().await {
            Ok(body) => (true, format!("Direct exec — restart-daemon: ok\n{body}")),
            Err(e2) => (false, format!("Direct exec — restart-daemon failed: {e2}")),
        },
        Err(e) => (false, format!("Bridge — restart-daemon failed: {e}")),
    };
    let msg = if ok {
        format!(
            "{text}\n\nCEC re-initializes on daemon start — Test CEC in a few seconds to confirm."
        )
    } else {
        text
    };
    render_health_view(state, "cec-health", Some((ok, msg))).await
}

// ---------------------------------------------------------------------------
// Topology — GET-lazy `cec-scan` / `cec-device`, merged with the
// `cecDeviceNames` friendly-name overrides from `get-config`.
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct CecDeviceJson {
    #[serde(rename = "logicalAddress")]
    logical_address: i64,
    #[serde(rename = "powerStatus")]
    power_status: String,
}

fn friendly_name(names: &Value, addr: i64) -> String {
    names
        .get(addr.to_string())
        .and_then(Value::as_str)
        .map(str::to_string)
        .unwrap_or_else(|| default_device_name(addr).to_string())
}

/// `POST /cec/scan` — `cec-scan`, rendered as a table with per-device
/// power-on/standby actions. Friendly names come from `cecDeviceNames`
/// (`get-config`), falling back to the CEC 1.4 device-type default.
pub async fn scan(State(state): State<SharedState>) -> impl IntoResponse {
    Html(render_scan(&state).await)
}

pub async fn render_scan(state: &AppState) -> String {
    let names = state
        .ipc
        .get_config()
        .await
        .ok()
        .and_then(|cfg| cfg.get("cecDeviceNames").cloned())
        .unwrap_or_else(|| Value::Object(Default::default()));

    match state
        .ipc
        .command_json::<Vec<CecDeviceJson>>("cec-scan")
        .await
    {
        Ok(devices) => {
            if devices.is_empty() {
                return result_html(
                    true,
                    "<p class=\"muted\">No CEC devices found on the bus.</p>",
                );
            }
            let mut html = String::from(
                r#"<table class="tools-table"><thead><tr><th>Addr</th><th>Name</th><th>Power</th><th>Actions</th></tr></thead><tbody>"#,
            );
            for d in &devices {
                let name = friendly_name(&names, d.logical_address);
                html.push_str(&format!(
                    "<tr><td>{addr}</td><td>{name}</td><td>{power}</td><td>\
                     <form hx-post=\"/cec/power-on\" hx-target=\"#cec-result\" hx-swap=\"innerHTML\" class=\"inline-form\" \
                     hx-confirm=\"Power on {name_attr} (addr {addr})?\">\
                     <input type=\"hidden\" name=\"addr\" value=\"{addr}\"><button type=\"submit\">Power on</button></form>\
                     <form hx-post=\"/cec/power-off\" hx-target=\"#cec-result\" hx-swap=\"innerHTML\" class=\"inline-form\" \
                     hx-confirm=\"Send standby to {name_attr} (addr {addr})?\">\
                     <input type=\"hidden\" name=\"addr\" value=\"{addr}\"><button type=\"submit\">Standby</button></form>\
                     </td></tr>",
                    addr = d.logical_address,
                    name = esc(&name),
                    power = esc(&d.power_status),
                    name_attr = esc(&name),
                ));
            }
            html.push_str("</tbody></table>");
            result_html(true, &html)
        }
        Err(e) => {
            let msg = e.to_string();
            match cec_unavailable_reason(&msg) {
                Some(friendly) => result_html(
                    true,
                    &format!("<p class=\"banner banner-warn\">{}</p>", esc(friendly)),
                ),
                None => error_result(&msg),
            }
        }
    }
}

#[derive(Deserialize)]
pub struct AddrForm {
    addr: String,
}

/// `POST /cec/device` — `cec-device <addr>`.
pub async fn device(
    State(state): State<SharedState>,
    Form(form): Form<AddrForm>,
) -> impl IntoResponse {
    Html(render_device(&state, &form.addr).await)
}

pub async fn render_device(state: &AppState, addr: &str) -> String {
    match validate_addr(addr) {
        Ok(a) => run_cec(state, &format!("cec-device {a}")).await,
        Err(msg) => error_result(&msg),
    }
}

// ---------------------------------------------------------------------------
// Switching
// ---------------------------------------------------------------------------

/// `POST /cec/active-source` — `cec-active-source`: the "switch input"
/// primitive (there is no separate switch-input command).
pub async fn active_source(State(state): State<SharedState>) -> impl IntoResponse {
    Html(render_active_source(&state).await)
}

pub async fn render_active_source(state: &AppState) -> String {
    run_cec(state, "cec-active-source").await
}

/// `POST /cec/power-on` — `cec-power-on <addr>`.
pub async fn power_on(
    State(state): State<SharedState>,
    Form(form): Form<AddrForm>,
) -> impl IntoResponse {
    Html(render_power_on(&state, &form.addr).await)
}

pub async fn render_power_on(state: &AppState, addr: &str) -> String {
    match validate_addr(addr) {
        Ok(a) => run_cec(state, &format!("cec-power-on {a}")).await,
        Err(msg) => error_result(&msg),
    }
}

/// `POST /cec/power-off` — `cec-power-off <addr>`.
pub async fn power_off(
    State(state): State<SharedState>,
    Form(form): Form<AddrForm>,
) -> impl IntoResponse {
    Html(render_power_off(&state, &form.addr).await)
}

pub async fn render_power_off(state: &AppState, addr: &str) -> String {
    match validate_addr(addr) {
        Ok(a) => run_cec(state, &format!("cec-power-off {a}")).await,
        Err(msg) => error_result(&msg),
    }
}

// ---------------------------------------------------------------------------
// Page shell — GET /cec
// ---------------------------------------------------------------------------

#[derive(Template)]
#[template(path = "cec.html")]
struct CecTemplate {
    active: &'static str,
    health_section_html: String,
}

pub async fn page(State(state): State<SharedState>) -> impl IntoResponse {
    Html(render_page(&state).await)
}

pub async fn render_page(state: &AppState) -> String {
    let health_section_html = render_health_section(state).await;
    let tmpl = CecTemplate {
        active: "cec",
        health_section_html,
    };
    tmpl.render()
        .unwrap_or_else(|e| format!("<p class=\"banner banner-error\">render error: {e}</p>"))
}
