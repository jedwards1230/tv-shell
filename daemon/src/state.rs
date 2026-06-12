//! Control-channel message types (IPC -> input runtime) and the pure input
//! logic shared with the Linux input loop.
//!
//! The daemon uses a single owner for all mutable input state — the input
//! runtime in `input.rs` — which the IPC server messages over an `mpsc`
//! channel, each request carrying a `oneshot` reply. This avoids `Arc<Mutex>`
//! held across `.await`. Keeping the *decisions* (velocity curves, deadzone
//! direction, combo subsets, debug-string formatting) here as pure functions
//! lets them be unit-tested without a controller.

use crate::config;
use tokio::sync::oneshot;

/// Reply channel: the input runtime sends back the exact response line.
pub type Reply = oneshot::Sender<String>;

/// Messages from the IPC server to the input runtime.
#[derive(Debug)]
pub enum Control {
    Grab(Reply),
    Release(Reply),
    /// `handoff`: ungrab the physical pads so SDL/Moonlight reads them directly
    /// (#221). Keeps the session active for safety-combo watching.
    Handoff(Reply),
    Status(Reply),
    GetBindings(Reply),
    SetBinding {
        action: String,
        button: String,
        reply: Reply,
    },
    /// Arm capture; the reply is sent later (`captured:*` / `timeout` /
    /// `cancelled`).
    CaptureNext(Reply),
    CaptureCancel(Reply),
    /// `get-pads`: reply with the gamepad fleet as a compact JSON array
    /// (`[{id,index,name,grabbed}, …]`) in ascending player-index order.
    GetPads(Reply),
    /// `list-input-devices`: reply with EVERY controller-like input device on the
    /// host (anything with `BTN_SOUTH` or a `js*` handler) as a compact JSON
    /// array (`[{name,path,vendor,product,phys,handlers,grabbed}, …]`), including
    /// ungrabbed and virtual ones. A diagnostics enumerator (#97); the runtime
    /// marks `grabbed=true` for devices the fleet currently owns.
    ListInputDevices(Reply),
    /// `intent <name>`: validate `<name>` against the closed vocabulary and, if
    /// valid, broadcast `intent:<name>` to all subscribers. Pure broadcast —
    /// touches no device. The reply is `ok` or `error:unknown intent '<name>'`.
    Intent {
        name: String,
        reply: Reply,
    },
    /// `rumble <id> <ms>`: fire a rumble (FF_RUMBLE) effect on the pad whose
    /// stable wire id is `id` for `ms` milliseconds. A no-op (still replies `ok`)
    /// when no pad matches `id`, the pad has no `EV_FF` support, or the persisted
    /// `rumbleEnabled` setting is off. The reply is `ok` or `error:*`.
    Rumble {
        id: String,
        ms: u32,
        reply: Reply,
    },
    /// `key <name>`: synthesize a single keystroke (press+release) on the shared
    /// virtual keyboard. The runtime maps `name` via `config::key_for_action`
    /// and emits the keycode; an unknown name replies `error:unknown key
    /// '<name>'`. Unlike `Intent`, this touches the device — it is the headless
    /// nav surface (parity with the gamepad d-pad / `wtype`).
    Key {
        name: String,
        reply: Reply,
    },
    /// `set-active-game <id>` — signal the current foreground game to the
    /// daemon so per-game binding overrides activate. `id` is `Some(game_id)`
    /// to set the active game, or `None` to clear it (bare `set-active-game`).
    /// In-memory only: resets on daemon restart.
    SetActiveGame {
        id: Option<String>,
        reply: Reply,
    },
    /// `set-config` succeeded — refresh any cached settings (currently the
    /// `rumbleEnabled` flag, #108). Sent fire-and-forget by the IPC dispatch after
    /// a successful `set-config`; the input runtime re-reads the affected keys
    /// from disk.
    ConfigChanged,

    // --- #160: per-pad battery + rumble status queries ---
    /// `pad-battery <id>` — reply with the current battery state for the pad
    /// whose stable wire id is `<id>`. Reply is a compact JSON object
    /// `{id, present, level?, charging?}`. `present=false` for wired/unknown.
    PadBatteryQuery {
        id: String,
        reply: Reply,
    },
    /// `pad-rumble-status <id>` — reply with rumble capability/enabled status.
    /// Reply is `{id, supported, enabled}`.
    PadRumbleStatus {
        id: String,
        reply: Reply,
    },

    /// `controllerdb-refresh` notification: the IPC layer fetched and updated
    /// the upstream DB; the input runtime should swap in the new DB live.
    ControllerDbRefreshed {
        reply: Reply,
    },

    /// Our logind session became active/inactive (sent by the `session` actor).
    /// On `false` the input runtime releases every pad's physical `EVIOCGRAB`
    /// and stops processing their events, so a foreground session (e.g.
    /// Plasma/Bigscreen the user VT-switched to) gets the controller; on `true`
    /// it re-grabs every pad. Orthogonal to the presenter (`Grab`/`Release`),
    /// which only routes events while we own the grab.
    SetSessionActive(bool),

    Shutdown,
}

// ---------------------------------------------------------------------------
// Pure input logic
// ---------------------------------------------------------------------------

/// Quadratic mouse velocity for one right-stick axis. Port of Python
/// `_compute_mouse_velocity`. Returns signed pixels/poll.
pub fn compute_mouse_velocity(raw: i32, center: i32, threshold: i32, half_range: i32) -> i32 {
    let offset = raw - center;
    if offset.abs() < threshold {
        return 0;
    }
    let denom = (half_range - threshold).max(1) as f64;
    let mag = (((offset.abs() - threshold) as f64) / denom).min(1.0);
    let speed =
        config::MOUSE_SPEED_MIN + (config::MOUSE_SPEED_MAX - config::MOUSE_SPEED_MIN) * mag * mag;
    let magnitude = speed as i32; // truncates toward zero, like Python int()
    if offset > 0 {
        magnitude
    } else {
        -magnitude
    }
}

/// Which arrow key (if any) the left stick should emit for an axis value.
/// Port of the branch in `_handle_stick_axis`.
pub fn left_stick_target(
    value: i32,
    center: i32,
    threshold: i32,
    neg_key: u16,
    pos_key: u16,
) -> Option<u16> {
    let offset = value - center;
    if offset < -threshold {
        Some(neg_key)
    } else if offset > threshold {
        Some(pos_key)
    } else {
        None
    }
}

/// Right-stick X direction label for the debug overlay (`R<-`/`R->`/none).
pub fn rstick_x_dir(value: i32, center: i32, threshold: i32) -> Option<&'static str> {
    let offset = value - center;
    if offset < -threshold {
        Some("R←")
    } else if offset > threshold {
        Some("R→")
    } else {
        None
    }
}

/// Right-stick Y direction label for the debug overlay (`R^`/`Rv`/none).
pub fn rstick_y_dir(value: i32, center: i32, threshold: i32) -> Option<&'static str> {
    let offset = value - center;
    if offset < -threshold {
        Some("R↑")
    } else if offset > threshold {
        Some("R↓")
    } else {
        None
    }
}

/// True if every code in `combo` is currently held.
pub fn subset_held(combo: &[u16], held: &std::collections::HashSet<u16>) -> bool {
    combo.iter().all(|c| held.contains(c))
}

fn left_stick_label(key: u16) -> &'static str {
    match key {
        config::KEY_UP => "L↑",
        config::KEY_DOWN => "L↓",
        config::KEY_LEFT => "L←",
        config::KEY_RIGHT => "L→",
        _ => "L?",
    }
}

/// Build the `buttons:` debug payload (everything after `buttons:`). Port of
/// `_notify_held_buttons`. `held_sorted` must be the held codes in ascending
/// order; `raw_name` resolves a code to its kernel name for the rare fallback
/// branch (codes not in the button/d-pad tables).
#[allow(clippy::too_many_arguments)]
pub fn build_buttons_payload<F>(
    held_sorted: &[u16],
    left_trigger: bool,
    right_trigger: bool,
    stick_x_key: Option<u16>,
    stick_y_key: Option<u16>,
    rstick_x_dir: Option<&str>,
    rstick_y_dir: Option<&str>,
    raw_name: F,
) -> String
where
    F: Fn(u16) -> Option<String>,
{
    let mut names: Vec<String> = Vec::new();
    for &code in held_sorted {
        if let Some(n) = config::button_display_name(code) {
            names.push(n.to_string());
        } else if let Some(n) = config::dpad_display_name(code) {
            names.push(n.to_string());
        } else if let Some(raw) = raw_name(code) {
            let stripped = raw.replace("BTN_", "").replace("KEY_", "");
            names.push(config::python_title(&stripped));
        } else {
            names.push(format!("0x{code:x}"));
        }
    }
    if left_trigger {
        names.push("LT".to_string());
    }
    if right_trigger {
        names.push("RT".to_string());
    }
    if let Some(k) = stick_x_key {
        names.push(left_stick_label(k).to_string());
    }
    if let Some(k) = stick_y_key {
        names.push(left_stick_label(k).to_string());
    }
    if let Some(d) = rstick_x_dir {
        names.push(d.to_string());
    }
    if let Some(d) = rstick_y_dir {
        names.push(d.to_string());
    }
    names.join(" + ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn mouse_velocity_zero_inside_deadzone() {
        assert_eq!(compute_mouse_velocity(100, 100, 50, 1000), 0);
        assert_eq!(compute_mouse_velocity(140, 100, 50, 1000), 0); // offset 40 < 50
    }

    #[test]
    fn mouse_velocity_signed_and_capped() {
        // At full deflection, speed -> MOUSE_SPEED_MAX (25), sign follows offset.
        let full_pos = compute_mouse_velocity(1100, 100, 50, 1000);
        assert_eq!(full_pos, 25);
        let full_neg = compute_mouse_velocity(-900, 100, 50, 1000);
        assert_eq!(full_neg, -25);
        // Just past threshold -> near MOUSE_SPEED_MIN (2).
        let near_min = compute_mouse_velocity(151, 100, 50, 1000);
        assert_eq!(near_min, 2);
    }

    #[test]
    fn left_stick_direction() {
        assert_eq!(
            left_stick_target(0, 128, 30, config::KEY_LEFT, config::KEY_RIGHT),
            Some(config::KEY_LEFT)
        );
        assert_eq!(
            left_stick_target(255, 128, 30, config::KEY_LEFT, config::KEY_RIGHT),
            Some(config::KEY_RIGHT)
        );
        assert_eq!(
            left_stick_target(128, 128, 30, config::KEY_LEFT, config::KEY_RIGHT),
            None
        );
    }

    #[test]
    fn rstick_direction_labels() {
        assert_eq!(rstick_x_dir(0, 128, 30), Some("R←"));
        assert_eq!(rstick_x_dir(255, 128, 30), Some("R→"));
        assert_eq!(rstick_y_dir(0, 128, 30), Some("R↑"));
        assert_eq!(rstick_y_dir(255, 128, 30), Some("R↓"));
        assert_eq!(rstick_x_dir(128, 128, 30), None);
    }

    #[test]
    fn combo_subset() {
        let held: HashSet<u16> = [config::BTN_MODE, config::BTN_EAST, config::BTN_SOUTH]
            .into_iter()
            .collect();
        assert!(subset_held(&config::COMBO_KEYS, &held));
        assert!(!subset_held(&config::QUIT_COMBO_KEYS, &held));
    }

    #[test]
    fn buttons_payload_orders_by_ascending_code() {
        // The IPC doc illustrates this as "Home + B + ..." but Python iterates
        // sorted(held_keys) by code, and BTN_EAST (0x131) < BTN_MODE (0x13c),
        // so the real output is "B + Home + ...". We match Python (ground truth).
        let mut held = [config::BTN_MODE, config::BTN_EAST];
        held.sort();
        let payload = build_buttons_payload(
            &held,
            true, // left trigger -> LT
            false,
            Some(config::KEY_RIGHT), // L→
            None,
            None,
            Some("R↑"),
            |_| None,
        );
        assert_eq!(payload, "B + Home + LT + L→ + R↑");
    }

    #[test]
    fn buttons_payload_empty() {
        let payload = build_buttons_payload(&[], false, false, None, None, None, None, |_| None);
        assert_eq!(payload, "");
    }

    #[test]
    fn buttons_payload_fallback_strips_and_titlecases() {
        // An unmapped held code falls back to its kernel name, stripped + titlecased.
        // Python `str.title()` treats `_` as a word boundary, so
        // "BTN_TRIGGER_HAPPY" -> strip "BTN_" -> "TRIGGER_HAPPY" -> "Trigger_Happy".
        let payload = build_buttons_payload(&[0x2c0], false, false, None, None, None, None, |_| {
            Some("BTN_TRIGGER_HAPPY".to_string())
        });
        assert_eq!(payload, "Trigger_Happy");
    }
}
