//! Hermetic handler-level tests: build an [`AppState`] whose IPC client
//! points at a non-existent socket and whose bridge has no base URL, then
//! exercise the internal `render_*` functions the axum handlers wrap
//! (rather than the handlers/extractors directly). Asserts they degrade
//! gracefully — non-empty HTML with the expected degraded markers, never a
//! panic — with no real daemon or network involved.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixListener;

use crate::bridge::BridgeClient;
use crate::config::AppConfig;
use crate::exec::Recovery;
use crate::ipc::IpcClient;
use crate::pages;
use crate::state::AppState;

fn hermetic_state() -> Arc<AppState> {
    // `/tmp` directly (short and stable): the socket is never bound here —
    // only connected-to, and the connection is expected to fail — but a
    // short path keeps this consistent with the `ipc` module's own tests.
    let sock = std::path::PathBuf::from(format!(
        "/tmp/tvshp-hermetic-{}-{:?}.sock",
        std::process::id(),
        std::thread::current().id()
    ));
    Arc::new(AppState {
        cfg: AppConfig::default(),
        ipc: IpcClient::new(sock),
        bridge: BridgeClient::new(None, None),
        recovery: Recovery::new(),
    })
}

#[tokio::test]
async fn dashboard_tiles_degrades_when_daemon_unreachable() {
    let state = hermetic_state();
    let html = pages::dashboard::render_tiles(&state).await;
    assert!(!html.is_empty());
    assert!(
        html.contains("/dev"),
        "degraded dashboard must link to /dev for recovery: {html}"
    );
    assert!(
        html.to_lowercase().contains("unreachable"),
        "degraded dashboard must show an unreachable marker: {html}"
    );
}

#[tokio::test]
async fn logs_view_degrades_when_bridge_and_daemon_absent() {
    let state = hermetic_state();
    let html = pages::logs::render_view(&state, 50, None, false).await;
    assert!(!html.is_empty());
    assert!(
        html.to_lowercase().contains("bridge"),
        "log view must mention the unavailable HTTP bridge: {html}"
    );
}

#[tokio::test]
async fn dev_page_renders_with_daemon_down_banner() {
    let state = hermetic_state();
    let html = pages::dev::render_page(&state).await;
    assert!(!html.is_empty());
    assert!(
        html.contains("down"),
        "dev page must show the daemon as down when unreachable: {html}"
    );
}

// ---------------------------------------------------------------------------
// UI-polish pass: status humanizer, nav daemon dot, OOB refreshes
// ---------------------------------------------------------------------------

#[tokio::test]
async fn dashboard_tiles_humanizes_status_token() {
    let mut replies = HashMap::new();
    replies.insert("status", "connected:grabbed");
    let sock = spawn_canned_daemon("dashboard-status-humanize", replies);
    tokio::time::sleep(Duration::from_millis(20)).await;
    let state = state_for_socket(sock);
    let html = pages::dashboard::render_tiles(&state).await;
    assert!(
        html.contains("Connected · grabbed"),
        "expected the humanized status label: {html}"
    );
    assert!(
        html.contains("connected:grabbed"),
        "expected the raw token to remain visible for debugging: {html}"
    );
}

#[tokio::test]
async fn controllers_fleet_humanizes_status_token() {
    let mut replies = HashMap::new();
    replies.insert("status", "disconnected:grabbed");
    replies.insert("get-pads", "[]");
    replies.insert("get-bindings", "{}");
    replies.insert("get-config", "{}");
    replies.insert("controllerdb-status", "{}");
    let sock = spawn_canned_daemon("controllers-fleet-humanize", replies);
    tokio::time::sleep(Duration::from_millis(20)).await;
    let state = state_for_socket(sock);
    let html = pages::controllers::render_page(&state).await;
    assert!(
        html.contains("No controllers connected · grab armed"),
        "expected the humanized fleet status label: {html}"
    );
}

#[tokio::test]
async fn controllers_grab_includes_oob_fleet_refresh() {
    let mut replies = HashMap::new();
    replies.insert("grab", "ok");
    replies.insert("status", "connected:grabbed");
    replies.insert("get-pads", "[]");
    let sock = spawn_canned_daemon("controllers-grab-oob", replies);
    tokio::time::sleep(Duration::from_millis(20)).await;
    let state = state_for_socket(sock);
    let html = pages::controllers::render_grab(&state).await;
    assert!(
        html.contains(r#"id="controllers-fleet""#) && html.contains(r#"hx-swap-oob="true""#),
        "expected an out-of-band fleet refresh bolted onto the grab response: {html}"
    );
}

#[tokio::test]
async fn cec_active_source_includes_oob_health_refresh() {
    let mut replies = HashMap::new();
    replies.insert("cec-active-source", "ok");
    replies.insert(
        "cec-health",
        r#"{"transmit":"ok","reason":null,"since":1719500000000,"lastError":null}"#,
    );
    let sock = spawn_canned_daemon("cec-active-source-oob", replies);
    tokio::time::sleep(Duration::from_millis(20)).await;
    let state = state_for_socket(sock);
    let html = pages::cec::render_active_source(&state).await;
    assert!(
        html.contains(r#"id="cec-health""#) && html.contains(r#"hx-swap-oob="true""#),
        "expected an out-of-band health refresh bolted onto the active-source response: {html}"
    );
}

#[tokio::test]
async fn nav_dot_shows_ok_when_daemon_reachable() {
    let mut replies = HashMap::new();
    replies.insert("status", "connected:grabbed");
    let sock = spawn_canned_daemon("nav-dot-ok", replies);
    tokio::time::sleep(Duration::from_millis(20)).await;
    let state = state_for_socket(sock);
    let html = pages::nav::render_dot(&state).await;
    assert!(
        html.contains("dot-ok"),
        "expected a green dot when the daemon answers: {html}"
    );
}

#[tokio::test]
async fn nav_dot_shows_error_when_daemon_unreachable() {
    let state = hermetic_state();
    let html = pages::nav::render_dot(&state).await;
    assert!(
        html.contains("dot-error"),
        "expected a red dot when the daemon is unreachable: {html}"
    );
}

/// A minimal multi-connection fake daemon for `get-config`/`set-config`
/// round-trip tests. Unlike `ipc`'s private one-shot `spawn_fake_daemon`
/// (good for a single `IpcClient::command` call), the real Settings/Widgets
/// flows make TWO separate connections per page load or save (each
/// `IpcClient` request opens its own connection — see `ipc.rs`'s doc
/// comment), so this helper loops accepting connections indefinitely rather
/// than closing after one.
///
/// Replies:
/// - `get-config` → `canned_get_config` verbatim (the fixed document tests
///   assert against).
/// - `set-config <json>` → records the raw JSON body (everything after the
///   first space) into the returned `Arc<Mutex<Vec<String>>>`, in receipt
///   order, and replies `ok`.
/// - anything else → `error:unknown command` (shouldn't be hit by these
///   tests, but avoids a silent hang if it is).
///
/// Reusable as-is by the Widgets-page implementer for its own
/// `widgets`-subtree round-trip tests — just spawn it and point a fresh
/// `IpcClient` at the returned socket path.
pub fn spawn_config_daemon(
    name: &str,
    canned_get_config: &'static str,
) -> (std::path::PathBuf, Arc<Mutex<Vec<String>>>) {
    let sock = std::path::PathBuf::from(format!(
        "/tmp/tvshp-cfgd-{name}-{}-{}.sock",
        std::process::id(),
        config_daemon_uniquifier()
    ));
    let _ = std::fs::remove_file(&sock);
    let listener = UnixListener::bind(&sock).expect("bind fake config daemon socket");
    let received: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let received_for_task = Arc::clone(&received);
    tokio::spawn(async move {
        loop {
            let Ok((stream, _)) = listener.accept().await else {
                break;
            };
            let received = Arc::clone(&received_for_task);
            tokio::spawn(async move {
                let (read_half, mut write_half) = stream.into_split();
                let mut reader = BufReader::new(read_half);
                let mut line = String::new();
                if reader.read_line(&mut line).await.unwrap_or(0) == 0 {
                    return;
                }
                let line = line.trim_end();
                if line == "get-config" {
                    let _ = write_half
                        .write_all(format!("{canned_get_config}\n").as_bytes())
                        .await;
                } else if let Some(body) = line.strip_prefix("set-config ") {
                    received.lock().unwrap().push(body.to_string());
                    let _ = write_half.write_all(b"ok\n").await;
                } else {
                    let _ = write_half.write_all(b"error:unknown command\n").await;
                }
            });
        }
    });
    (sock, received)
}

fn config_daemon_uniquifier() -> u32 {
    use std::sync::atomic::{AtomicU32, Ordering};
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    COUNTER.fetch_add(1, Ordering::Relaxed)
}

/// Like [`spawn_config_daemon`], but answers an arbitrary map of exact
/// request-line → reply-line pairs instead of only understanding
/// `get-config`/`set-config`. Used by the Tools console's round-trip tests,
/// which exercise many distinct IPC commands against one fake daemon.
/// Requests not present in `replies` get `error:unknown command`.
pub fn spawn_canned_daemon(
    name: &str,
    replies: std::collections::HashMap<&'static str, &'static str>,
) -> std::path::PathBuf {
    let sock = std::path::PathBuf::from(format!(
        "/tmp/tvshp-canned-{name}-{}-{}.sock",
        std::process::id(),
        config_daemon_uniquifier()
    ));
    let _ = std::fs::remove_file(&sock);
    let listener = UnixListener::bind(&sock).expect("bind fake canned daemon socket");
    tokio::spawn(async move {
        loop {
            let Ok((stream, _)) = listener.accept().await else {
                break;
            };
            let replies = replies.clone();
            tokio::spawn(async move {
                let (read_half, mut write_half) = stream.into_split();
                let mut reader = BufReader::new(read_half);
                let mut line = String::new();
                if reader.read_line(&mut line).await.unwrap_or(0) == 0 {
                    return;
                }
                let line = line.trim_end();
                let reply = replies
                    .get(line)
                    .copied()
                    .unwrap_or("error:unknown command");
                let _ = write_half.write_all(format!("{reply}\n").as_bytes()).await;
            });
        }
    });
    sock
}

fn state_for_socket(sock: std::path::PathBuf) -> Arc<AppState> {
    Arc::new(AppState {
        cfg: AppConfig::default(),
        ipc: IpcClient::new(sock),
        bridge: BridgeClient::new(None, None),
        recovery: Recovery::new(),
    })
}

#[tokio::test]
async fn settings_page_renders_current_config() {
    let (sock, _received) = spawn_config_daemon(
        "settings-page",
        r#"{"themeMode":"light","rumbleEnabled":false}"#,
    );
    tokio::time::sleep(Duration::from_millis(20)).await;
    let state = state_for_socket(sock);
    let html = pages::settings::render_page(&state).await;
    assert!(!html.is_empty());
    assert!(
        html.contains("light"),
        "settings page must render the current themeMode value: {html}"
    );
}

#[tokio::test]
async fn settings_save_sends_expected_patch() {
    let (sock, received) = spawn_config_daemon("settings-save", "{}");
    tokio::time::sleep(Duration::from_millis(20)).await;
    let state = state_for_socket(sock);

    let mut form: HashMap<String, String> = HashMap::new();
    form.insert("themeMode".to_string(), "light".to_string());
    form.insert("rumbleEnabled".to_string(), "on".to_string()); // checked
                                                                // controllerDebug intentionally absent from the form -> must become
                                                                // explicit `false`, not be omitted.

    let html = pages::settings::render_save(&state, &form).await;
    assert!(
        html.to_lowercase().contains("saved"),
        "expected ok result: {html}"
    );

    let sent = received.lock().unwrap().clone();
    assert_eq!(
        sent.len(),
        1,
        "expected exactly one set-config call: {sent:?}"
    );
    let patch: serde_json::Value = serde_json::from_str(&sent[0]).unwrap();
    assert_eq!(patch["themeMode"], "light");
    assert_eq!(patch["rumbleEnabled"], true);
    assert_eq!(patch["controllerDebug"], false);
    assert!(
        patch.get("keyBindings").is_none(),
        "keyBindings must never appear in a Settings save patch: {patch}"
    );
    assert!(
        patch.get("perGameBindings").is_none(),
        "perGameBindings must never appear in a Settings save patch: {patch}"
    );
    assert!(
        patch.get("perPlayerBindings").is_none(),
        "perPlayerBindings must never appear in a Settings save patch: {patch}"
    );
    assert!(
        patch.get("widgets").is_none(),
        "widgets must never appear in a Settings save patch: {patch}"
    );
}

#[tokio::test]
async fn settings_page_renders_pretty_printed_raw_json() {
    let (sock, _received) = spawn_config_daemon(
        "settings-raw-pretty-render",
        r#"{"themeMode":"dark","rumbleEnabled":true}"#,
    );
    tokio::time::sleep(Duration::from_millis(20)).await;
    let state = state_for_socket(sock);
    let html = pages::settings::render_page(&state).await;
    assert!(
        html.contains("{\n"),
        "expected the raw JSON escape hatch to be pretty-printed (multi-line): {html}"
    );
}

#[tokio::test]
async fn settings_raw_pretty_input_is_sent_compact() {
    // The textarea round-trips a pretty-printed, multi-line document — the
    // set-config call it triggers must still be a single compact line.
    let (sock, received) = spawn_config_daemon("settings-raw-compact", "{}");
    tokio::time::sleep(Duration::from_millis(20)).await;
    let state = state_for_socket(sock);
    let pretty = "{\n  \"themeMode\": \"light\"\n}";
    let html = pages::settings::render_save_raw(&state, pretty).await;
    assert!(
        html.to_lowercase().contains("merged"),
        "expected an ok result: {html}"
    );
    let sent = received.lock().unwrap().clone();
    assert_eq!(
        sent.len(),
        1,
        "expected exactly one set-config call: {sent:?}"
    );
    assert_eq!(
        sent[0], r#"{"themeMode":"light"}"#,
        "raw JSON must be compacted to a single line before set-config: {sent:?}"
    );
}

#[tokio::test]
async fn settings_raw_rejects_malformed_json() {
    // No daemon needed: malformed JSON must be rejected before any IPC call.
    let state = hermetic_state();
    let html = pages::settings::render_save_raw(&state, "not json").await;
    assert!(!html.is_empty());
    assert!(
        html.to_lowercase().contains("invalid"),
        "expected an error marker for malformed JSON: {html}"
    );
}

#[tokio::test]
async fn settings_raw_rejects_non_object_json() {
    let state = hermetic_state();
    let html = pages::settings::render_save_raw(&state, "[1,2,3]").await;
    assert!(
        html.to_lowercase().contains("invalid") || html.to_lowercase().contains("object"),
        "expected an error marker for a non-object JSON body: {html}"
    );
}

#[tokio::test]
async fn settings_page_degrades_when_daemon_unreachable() {
    let state = hermetic_state();
    let html = pages::settings::render_page(&state).await;
    assert!(!html.is_empty());
    assert!(
        html.to_lowercase().contains("unreachable"),
        "settings page must show an unreachable marker when the daemon is down: {html}"
    );
}

#[tokio::test]
async fn widgets_page_degrades_when_daemon_unreachable() {
    let state = hermetic_state();
    let html = pages::widgets::render_page(&state).await;
    assert!(!html.is_empty());
    assert!(
        html.to_lowercase().contains("unreachable"),
        "widgets page must show an unreachable marker when the daemon is down: {html}"
    );
}

#[tokio::test]
async fn widgets_page_default_fills_missing_subtree() {
    // No "widgets" key at all in the canned get-config document — every
    // widget must still render, default-filled per WidgetManifests.qml.
    let (sock, _received) = spawn_config_daemon("widgets-page-empty", "{}");
    tokio::time::sleep(Duration::from_millis(20)).await;
    let state = state_for_socket(sock);
    let html = pages::widgets::render_page(&state).await;
    assert!(!html.is_empty());
    assert!(
        html.contains("Moonlight"),
        "expected all 5 widget cards: {html}"
    );
    assert!(
        html.contains("Now Playing"),
        "expected all 5 widget cards: {html}"
    );
    assert!(html.contains("Plex"), "expected all 5 widget cards: {html}");
    assert!(html.contains("Apps"), "expected all 5 widget cards: {html}");
    assert!(
        html.contains("Steam"),
        "expected all 5 widget cards: {html}"
    );
}

#[tokio::test]
async fn widgets_save_sends_all_five_widgets_with_valid_sizes() {
    let (sock, received) = spawn_config_daemon("widgets-save", "{}");
    tokio::time::sleep(Duration::from_millis(20)).await;
    let state = state_for_socket(sock);

    // Mirrors what the whole-page pre-filled form would submit: every
    // widget's fields present, one value (moonlight's size) changed.
    let mut form: HashMap<String, String> = HashMap::new();
    form.insert("w_moonlight_enabled".to_string(), "on".to_string());
    form.insert("w_moonlight_order".to_string(), "0".to_string());
    form.insert("w_moonlight_size".to_string(), "large".to_string());
    form.insert("w_nowplaying_enabled".to_string(), "on".to_string());
    form.insert("w_nowplaying_order".to_string(), "1".to_string());
    form.insert("w_nowplaying_size".to_string(), "medium".to_string());
    form.insert(
        "w_nowplaying_pref_hideFromRecent".to_string(),
        "on".to_string(),
    );
    form.insert("w_plex_enabled".to_string(), "on".to_string());
    form.insert("w_plex_order".to_string(), "2".to_string());
    form.insert("w_plex_size".to_string(), "medium".to_string());
    form.insert("w_plex_pref_hideFromRecent".to_string(), "on".to_string());
    form.insert("w_recent_enabled".to_string(), "on".to_string());
    form.insert("w_recent_order".to_string(), "3".to_string());
    form.insert("w_recent_size".to_string(), "medium".to_string());
    form.insert("w_steam_order".to_string(), "4".to_string());
    form.insert("w_steam_size".to_string(), "medium".to_string());
    // w_steam_enabled intentionally absent — steam defaults disabled.

    let html = pages::widgets::render_save(&state, &form).await;
    assert!(
        html.to_lowercase().contains("ok"),
        "expected an ok result: {html}"
    );

    let sent = received.lock().unwrap().clone();
    assert_eq!(
        sent.len(),
        1,
        "expected exactly one set-config call: {sent:?}"
    );
    let patch: serde_json::Value = serde_json::from_str(&sent[0]).unwrap();
    let widgets = patch["widgets"]
        .as_object()
        .expect("set-config body must contain a widgets object");
    for id in ["moonlight", "nowplaying", "plex", "recent", "steam"] {
        assert!(
            widgets.contains_key(id),
            "set-config body must include widget {id} (shallow merge would wipe \
             siblings if omitted): {patch}"
        );
    }
    assert_eq!(widgets["moonlight"]["size"], "large");
    assert_eq!(widgets["steam"]["enabled"], false);
    assert_eq!(widgets["steam"]["size"], "medium");
    assert_eq!(widgets["nowplaying"]["prefs"]["hideFromRecent"], true);
    assert_eq!(widgets["plex"]["prefs"]["hideFromRecent"], true);

    // Every widget's size must be one of its own manifest's allowed values.
    assert!(["small", "medium", "large"].contains(&widgets["moonlight"]["size"].as_str().unwrap()));
    assert!(["medium", "large"].contains(&widgets["steam"]["size"].as_str().unwrap()));
}

#[tokio::test]
async fn widgets_save_rejects_invalid_size_for_widget() {
    // "small" is not a valid Steam size (steam only offers medium/large) —
    // validation must reject this before any IPC call, so no daemon needed.
    let state = hermetic_state();
    let mut form: HashMap<String, String> = HashMap::new();
    form.insert("w_steam_size".to_string(), "small".to_string());
    let html = pages::widgets::render_save(&state, &form).await;
    assert!(
        html.to_lowercase().contains("invalid"),
        "expected a validation error for an out-of-enum size: {html}"
    );
}

#[tokio::test]
async fn widgets_reorder_up_swaps_with_predecessor_and_renumbers() {
    // Default (empty) config: declaration order is moonlight(0), nowplaying(1),
    // plex(2), recent(3), steam(4) — moving plex up should swap it with
    // nowplaying and renumber both.
    let (sock, received) = spawn_config_daemon("widgets-reorder-up", "{}");
    tokio::time::sleep(Duration::from_millis(20)).await;
    let state = state_for_socket(sock);

    let html = pages::widgets::render_reorder(&state, "plex", "up").await;
    assert!(
        html.contains("Plex") && html.contains("Now Playing"),
        "expected the refreshed grid to still show all cards: {html}"
    );

    let sent = received.lock().unwrap().clone();
    assert_eq!(
        sent.len(),
        1,
        "expected exactly one set-config call: {sent:?}"
    );
    let patch: serde_json::Value = serde_json::from_str(&sent[0]).unwrap();
    let widgets = patch["widgets"]
        .as_object()
        .expect("set-config body must contain a widgets object");
    for id in ["moonlight", "nowplaying", "plex", "recent", "steam"] {
        assert!(
            widgets.contains_key(id),
            "set-config body must include widget {id} (shallow merge would wipe \
             siblings if omitted): {patch}"
        );
    }
    assert_eq!(widgets["plex"]["order"], 1);
    assert_eq!(widgets["nowplaying"]["order"], 2);
    assert_eq!(
        widgets["moonlight"]["order"], 0,
        "a widget not involved in the swap keeps its position"
    );
}

#[tokio::test]
async fn widgets_reorder_at_the_boundary_is_a_position_noop_but_still_renumbers() {
    // moonlight is already first — "up" has no predecessor to swap with, but
    // the order fields still get renumbered to a clean 0..N sequence.
    let (sock, received) = spawn_config_daemon("widgets-reorder-boundary", "{}");
    tokio::time::sleep(Duration::from_millis(20)).await;
    let state = state_for_socket(sock);

    let html = pages::widgets::render_reorder(&state, "moonlight", "up").await;
    assert!(!html.is_empty());

    let sent = received.lock().unwrap().clone();
    assert_eq!(sent.len(), 1);
    let patch: serde_json::Value = serde_json::from_str(&sent[0]).unwrap();
    let widgets = patch["widgets"].as_object().unwrap();
    assert_eq!(widgets["moonlight"]["order"], 0);
    assert_eq!(widgets["nowplaying"]["order"], 1);
    assert_eq!(widgets["steam"]["order"], 4);
}

// ---------------------------------------------------------------------------
// M3: Tools console
// ---------------------------------------------------------------------------

#[tokio::test]
async fn tools_intent_rejects_whitespace_without_ipc() {
    // Validation must fail before any IPC call — no daemon needed.
    let state = hermetic_state();
    let html = pages::tools::render_intent(&state, "settings audio").await;
    assert!(
        html.to_lowercase().contains("whitespace"),
        "expected a whitespace validation error: {html}"
    );
}

#[tokio::test]
async fn tools_intent_degrades_when_daemon_unreachable() {
    let state = hermetic_state();
    let html = pages::tools::render_intent(&state, "home").await;
    assert!(
        html.to_lowercase().contains("unreachable"),
        "expected a daemon-unreachable marker: {html}"
    );
}

#[tokio::test]
async fn tools_key_rejects_unknown_key_without_ipc() {
    let state = hermetic_state();
    let html = pages::tools::render_key(&state, "north").await;
    assert!(
        html.to_lowercase().contains("unknown key"),
        "expected an unknown-key error: {html}"
    );
}

#[tokio::test]
async fn tools_net_ping_rejects_whitespace_in_host() {
    let state = hermetic_state();
    let html = pages::tools::render_net_ping(&state, "1.1.1.1 extra", None).await;
    assert!(
        html.to_lowercase().contains("whitespace"),
        "expected a whitespace validation error: {html}"
    );
}

#[tokio::test]
async fn tools_net_ping_rejects_out_of_range_count() {
    let state = hermetic_state();
    let html = pages::tools::render_net_ping(&state, "1.1.1.1", Some("99")).await;
    assert!(
        html.contains("1 and 10"),
        "expected a count-range validation error: {html}"
    );
}

#[tokio::test]
async fn tools_net_throughput_rejects_path_separator_in_iface() {
    let state = hermetic_state();
    let html = pages::tools::render_net_throughput(&state, "../etc").await;
    assert!(
        html.to_lowercase().contains("invalid interface"),
        "expected an invalid-interface error: {html}"
    );
}

#[tokio::test]
async fn tools_bt_action_rejects_unknown_action() {
    let state = hermetic_state();
    let html = pages::tools::render_bt_action(&state, "AA:BB:CC:DD:EE:FF", "reboot").await;
    assert!(
        html.to_lowercase().contains("unknown bluetooth action"),
        "expected an unknown-action error: {html}"
    );
}

#[tokio::test]
async fn tools_sys_status_json_roundtrip() {
    let mut replies = HashMap::new();
    replies.insert(
        "sys-status",
        r#"{"os":"Test OS","kernel":"1.2.3","hostname":"h","uptime":"1h"}"#,
    );
    let sock = spawn_canned_daemon("tools-sys-status", replies);
    tokio::time::sleep(Duration::from_millis(20)).await;
    let state = state_for_socket(sock);
    let html = pages::tools::run_line(&state, "sys-status").await;
    assert!(
        html.contains("Test OS"),
        "expected the pretty-printed sys-status JSON: {html}"
    );
}

#[tokio::test]
async fn tools_bt_power_status_bare_text_roundtrip() {
    let mut replies = HashMap::new();
    replies.insert("bt-power-status", "bt:on");
    let sock = spawn_canned_daemon("tools-bt-power", replies);
    tokio::time::sleep(Duration::from_millis(20)).await;
    let state = state_for_socket(sock);
    let html = pages::tools::run_line(&state, "bt-power-status").await;
    assert!(
        html.contains("bt:on"),
        "expected the bare-text reply: {html}"
    );
}

#[tokio::test]
async fn tools_raw_error_reply_roundtrip() {
    let mut replies = HashMap::new();
    replies.insert("sys-metrics", "error:input-runtime-down");
    let sock = spawn_canned_daemon("tools-raw-error", replies);
    tokio::time::sleep(Duration::from_millis(20)).await;
    let state = state_for_socket(sock);
    let html = pages::tools::render_raw(&state, "sys-metrics").await;
    assert!(
        html.to_lowercase().contains("input-runtime-down"),
        "expected the daemon's error message: {html}"
    );
}

#[tokio::test]
async fn tools_raw_warns_on_guarded_command() {
    let mut replies = HashMap::new();
    replies.insert("grab", "ok");
    let sock = spawn_canned_daemon("tools-raw-warn", replies);
    tokio::time::sleep(Duration::from_millis(20)).await;
    let state = state_for_socket(sock);
    let html = pages::tools::render_raw(&state, "grab").await;
    assert!(
        html.to_lowercase().contains("guarded"),
        "expected a warning banner for a guarded command: {html}"
    );
}

#[tokio::test]
async fn tools_raw_rejects_empty_command() {
    let state = hermetic_state();
    let html = pages::tools::render_raw(&state, "   ").await;
    assert!(
        html.to_lowercase().contains("empty"),
        "expected an empty-command validation error: {html}"
    );
}

// ---------------------------------------------------------------------------
// M3: Processes page
// ---------------------------------------------------------------------------

#[tokio::test]
async fn processes_page_renders_when_daemon_unreachable() {
    let state = hermetic_state();
    let html = pages::processes::render_page(&state).await;
    assert!(!html.is_empty());
    assert!(
        html.to_lowercase().contains("hyprland"),
        "expected the Hyprland section to render: {html}"
    );
    assert!(
        html.to_lowercase().contains("unavailable"),
        "expected a Hyprland-unavailable note when the daemon is down: {html}"
    );
}

#[tokio::test]
async fn processes_restart_rejects_unknown_unit_key() {
    let state = hermetic_state();
    let html = pages::processes::render_restart(&state, "bogus").await;
    assert!(
        html.to_lowercase().contains("unknown"),
        "expected an unknown-unit-key error: {html}"
    );
}

// ---------------------------------------------------------------------------
// M3: Dev screenshot viewer
// ---------------------------------------------------------------------------

#[tokio::test]
async fn dev_screenshot_capture_degrades_when_bridge_not_configured() {
    let state = hermetic_state();
    let html = pages::dev::render_screenshot_capture(&state).await;
    assert!(!html.is_empty());
    assert!(
        html.to_lowercase().contains("not configured"),
        "expected a bridge-not-configured message: {html}"
    );
    assert!(
        !html.contains("<img"),
        "must never emit an <img> tag when the capture itself failed: {html}"
    );
}

// ---------------------------------------------------------------------------
// M4: Controllers page
// ---------------------------------------------------------------------------

#[tokio::test]
async fn controllers_page_degrades_when_daemon_unreachable() {
    let state = hermetic_state();
    let html = pages::controllers::render_page(&state).await;
    assert!(!html.is_empty());
    assert!(
        html.to_lowercase().contains("unreachable"),
        "expected a daemon-unreachable marker somewhere on the page: {html}"
    );
}

#[tokio::test]
async fn controllers_page_renders_pads_bindings_and_controllerdb() {
    let mut replies = HashMap::new();
    replies.insert("status", "connected:grabbed");
    replies.insert(
        "get-pads",
        r#"[{"id":"uniq:a","index":0,"name":"Test Pad","grabbed":true}]"#,
    );
    replies.insert(
        "get-bindings",
        r#"{"select":"BTN_SOUTH","back":"BTN_EAST","altSelect":"BTN_NORTH","confirm":"BTN_START"}"#,
    );
    replies.insert(
        "get-config",
        r#"{"perGameBindings":{"steam_1":{"select":"BTN_SOUTH"}}}"#,
    );
    replies.insert(
        "controllerdb-status",
        r#"{"source":"bundled_baseline","entryCount":100,"lastDownloaded":0,"upstreamUrl":"https://example.test"}"#,
    );
    let sock = spawn_canned_daemon("controllers-page", replies);
    tokio::time::sleep(Duration::from_millis(20)).await;
    let state = state_for_socket(sock);

    let html = pages::controllers::render_page(&state).await;
    assert!(
        html.contains("Test Pad"),
        "expected the fleet table to render the pad: {html}"
    );
    assert!(
        html.contains("BTN_SOUTH"),
        "expected the bindings table to render the current button: {html}"
    );
    assert!(
        html.contains("steam_1"),
        "expected the per-game bindings JSON to render: {html}"
    );
    assert!(
        html.contains("bundled_baseline"),
        "expected the controllerdb status JSON to render: {html}"
    );
}

#[tokio::test]
async fn controllers_bindings_set_rejects_unknown_action_without_ipc() {
    // Validation must fail before any IPC call — no daemon needed.
    let state = hermetic_state();
    let html = pages::controllers::render_set_binding(&state, "bogus", "BTN_SOUTH").await;
    assert!(
        html.to_lowercase().contains("unknown action"),
        "expected an unknown-action error: {html}"
    );
}

#[tokio::test]
async fn controllers_bindings_set_rejects_unknown_button_without_ipc() {
    let state = hermetic_state();
    let html = pages::controllers::render_set_binding(&state, "select", "BTN_BOGUS").await;
    assert!(
        html.to_lowercase().contains("unknown button"),
        "expected an unknown-button error: {html}"
    );
}

#[tokio::test]
async fn controllers_capture_rejects_unknown_action_without_ipc() {
    let state = hermetic_state();
    let html = pages::controllers::render_capture(&state, "bogus").await;
    assert!(
        html.to_lowercase().contains("unknown action"),
        "expected an unknown-action error: {html}"
    );
}

#[tokio::test]
async fn controllers_capture_applies_captured_button_to_binding() {
    let mut replies = HashMap::new();
    replies.insert("capture-next", "captured:BTN_NORTH");
    replies.insert("set-binding select BTN_NORTH", "ok");
    replies.insert("get-bindings", r#"{"select":"BTN_NORTH"}"#);
    let sock = spawn_canned_daemon("controllers-capture", replies);
    tokio::time::sleep(Duration::from_millis(20)).await;
    let state = state_for_socket(sock);

    let html = pages::controllers::render_capture(&state, "select").await;
    assert!(
        html.contains("BTN_NORTH") && html.to_lowercase().contains("captured"),
        "expected the captured button to be reported and applied: {html}"
    );
}

#[tokio::test]
async fn controllers_capture_reports_timeout() {
    let mut replies = HashMap::new();
    replies.insert("capture-next", "timeout");
    replies.insert("get-bindings", "{}");
    let sock = spawn_canned_daemon("controllers-capture-timeout", replies);
    tokio::time::sleep(Duration::from_millis(20)).await;
    let state = state_for_socket(sock);

    let html = pages::controllers::render_capture(&state, "select").await;
    assert!(
        html.to_lowercase().contains("timed out"),
        "expected a timeout message: {html}"
    );
}

#[tokio::test]
async fn controllers_pad_rumble_rejects_out_of_range_ms() {
    let state = hermetic_state();
    let html = pages::controllers::render_pad_rumble(&state, "uniq:a", "99999").await;
    assert!(
        html.to_lowercase().contains("between"),
        "expected an out-of-range ms error: {html}"
    );
}

#[tokio::test]
async fn controllers_pad_battery_rejects_whitespace_id() {
    let state = hermetic_state();
    let html = pages::controllers::render_pad_battery(&state, "bad id").await;
    assert!(
        html.to_lowercase().contains("whitespace"),
        "expected a whitespace validation error: {html}"
    );
}

#[tokio::test]
async fn controllers_active_game_set_rejects_whitespace_id() {
    let state = hermetic_state();
    let html = pages::controllers::render_active_game_set(&state, "bad id").await;
    assert!(
        html.to_lowercase().contains("whitespace"),
        "expected a whitespace validation error: {html}"
    );
}

// ---------------------------------------------------------------------------
// M4: CEC page
// ---------------------------------------------------------------------------

#[tokio::test]
async fn cec_page_degrades_when_daemon_unreachable() {
    let state = hermetic_state();
    let html = pages::cec::render_page(&state).await;
    assert!(!html.is_empty());
    assert!(
        html.to_lowercase().contains("unreachable"),
        "expected a daemon-unreachable marker in the health panel: {html}"
    );
}

#[tokio::test]
async fn cec_health_ok_round_trip() {
    let mut replies = HashMap::new();
    replies.insert(
        "cec-health",
        r#"{"transmit":"ok","reason":null,"since":1719500000000,"lastError":null}"#,
    );
    let sock = spawn_canned_daemon("cec-health-ok", replies);
    tokio::time::sleep(Duration::from_millis(20)).await;
    let state = state_for_socket(sock);

    let html = pages::cec::render_page(&state).await;
    assert!(
        html.to_lowercase().contains("healthy"),
        "expected a healthy marker: {html}"
    );
}

#[tokio::test]
async fn cec_test_wedge_recommends_restart() {
    let mut replies = HashMap::new();
    replies.insert(
        "cec-test",
        r#"{"transmit":"failing","reason":null,"since":1719500000000,"lastError":"TransmitFailed"}"#,
    );
    let sock = spawn_canned_daemon("cec-test-wedge", replies);
    tokio::time::sleep(Duration::from_millis(20)).await;
    let state = state_for_socket(sock);

    let html = pages::cec::render_test(&state).await;
    assert!(
        html.to_lowercase().contains("wedge"),
        "expected a transmit-wedge marker: {html}"
    );
    assert!(
        html.contains("Restart daemon (recommended)"),
        "expected the restart step to be flagged recommended: {html}"
    );
}

#[tokio::test]
async fn cec_scan_merges_device_names_and_falls_back_to_default() {
    let mut replies = HashMap::new();
    replies.insert(
        "cec-scan",
        r#"[{"logicalAddress":0,"powerStatus":"on"},{"logicalAddress":5,"powerStatus":"standby"}]"#,
    );
    replies.insert("get-config", r#"{"cecDeviceNames":{"0":"Living Room TV"}}"#);
    let sock = spawn_canned_daemon("cec-scan", replies);
    tokio::time::sleep(Duration::from_millis(20)).await;
    let state = state_for_socket(sock);

    let html = pages::cec::render_scan(&state).await;
    assert!(
        html.contains("Living Room TV"),
        "expected the cecDeviceNames override to render: {html}"
    );
    assert!(
        html.contains("Audio System"),
        "expected the default name for addr 5 (no override) to render: {html}"
    );
}

#[tokio::test]
async fn cec_scan_disabled_build_renders_honest_state_not_a_failure() {
    let mut replies = HashMap::new();
    replies.insert("cec-scan", "error:unsupported on this platform");
    replies.insert("get-config", "{}");
    let sock = spawn_canned_daemon("cec-scan-disabled", replies);
    tokio::time::sleep(Duration::from_millis(20)).await;
    let state = state_for_socket(sock);

    let html = pages::cec::render_scan(&state).await;
    assert!(
        html.contains("not available in this daemon build"),
        "expected the honest not-available message: {html}"
    );
    assert!(
        !html.contains("result-error"),
        "a disabled build must not render as a failure: {html}"
    );
}

#[tokio::test]
async fn cec_device_rejects_out_of_range_addr_without_ipc() {
    let state = hermetic_state();
    let html = pages::cec::render_device(&state, "99").await;
    assert!(
        html.contains("between 0 and 15"),
        "expected an out-of-range addr error: {html}"
    );
}

#[tokio::test]
async fn cec_power_on_rejects_non_integer_addr_without_ipc() {
    let state = hermetic_state();
    let html = pages::cec::render_power_on(&state, "not-a-number").await;
    assert!(
        html.to_lowercase().contains("integer"),
        "expected an invalid-addr error: {html}"
    );
}

#[tokio::test]
async fn cec_recover_restart_daemon_falls_back_to_exec_and_reports_health() {
    // Hermetic: no bridge configured and no real daemon — exercises the
    // bridge-unavailable -> direct-exec fallback path end to end (the exec
    // call itself will fail too, since `systemctl` isn't a real unit here /
    // may not exist on the test host, but the response must still degrade
    // gracefully rather than panicking).
    let state = hermetic_state();
    let html = pages::cec::render_recover_restart_daemon(&state).await;
    assert!(!html.is_empty());
    assert!(
        html.to_lowercase().contains("restart-daemon"),
        "expected the restart-daemon action to be reported: {html}"
    );
}
