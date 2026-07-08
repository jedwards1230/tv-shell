//! Device enumeration / discovery and the hot-join / leave transitions.
//!
//! Split out of the former monolithic `input.rs` (behavior-preserving).

use super::*;

/// Parse `/proc/bus/input/devices` into a map from event-node name (e.g.
/// `event18`) to that device's full handler list (e.g. `["event18", "js0"]`).
///
/// The file is blocks separated by blank lines; the `H: Handlers=...` line lists
/// the device's handlers (event node, `js*`, `mouseN`, `kbd`, …). We key each
/// block by its `eventN` handler so the evdev enumeration (which yields devnodes)
/// can recover the `js*` handlers a bare evdev `Device` doesn't expose. A missing
/// or unreadable file yields an empty map (the enumerator still lists event-node
/// handlers it derives directly).
pub(crate) fn parse_proc_input_handlers() -> HashMap<String, Vec<String>> {
    let mut map = HashMap::new();
    let Ok(text) = std::fs::read_to_string("/proc/bus/input/devices") else {
        return map;
    };
    for block in text.split("\n\n") {
        let mut handlers: Vec<String> = Vec::new();
        for line in block.lines() {
            if let Some(rest) = line.strip_prefix("H: Handlers=") {
                handlers = rest.split_whitespace().map(|h| h.to_string()).collect();
            }
        }
        if let Some(event) = handlers.iter().find(|h| h.starts_with("event")).cloned() {
            map.insert(event, handlers);
        }
    }
    map
}

/// Build the `list-input-devices` reply (#97): EVERY controller-like input
/// device on the host — anything that advertises `BTN_SOUTH` or carries a `js*`
/// handler — as a compact JSON array, including ungrabbed and virtual devices.
///
/// This is a diagnostics enumerator (it replaces `ControllerSettings`'
/// `/proc/bus/input/devices` python reader), distinct from `get-pads` (the
/// grabbed fleet only). `grabbed` is `true` only for devices whose devnode path
/// the fleet currently owns. Devices are returned in ascending devnode-path
/// order for a stable wire.
///
/// Called via `spawn_blocking` from the `Control::ListInputDevices` arm (#108)
/// so `evdev::enumerate()` does not stall the input runtime. The caller collects
/// the fleet's grabbed paths before the blocking boundary and passes them in.
pub(crate) fn list_input_devices_with(grabbed_paths: HashSet<PathBuf>) -> String {
    let proc_handlers = parse_proc_input_handlers();

    let mut devices: Vec<(PathBuf, Device)> = evdev::enumerate().collect();
    devices.sort_by(|a, b| a.0.cmp(&b.0));

    let mut rows: Vec<crate::protocol::InputDeviceInfo> = Vec::new();
    for (path, dev) in devices {
        let event_name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_string();
        // Handlers from /proc keyed by the event node; fall back to just the
        // event node name when /proc didn't have the block.
        let handlers = proc_handlers.get(&event_name).cloned().unwrap_or_else(|| {
            if event_name.is_empty() {
                Vec::new()
            } else {
                vec![event_name.clone()]
            }
        });
        let has_btn_south = dev
            .supported_keys()
            .is_some_and(|keys| keys.contains(KeyCode::BTN_SOUTH));
        let has_js = handlers.iter().any(|h| h.starts_with("js"));
        // Controller-like: BTN_SOUTH OR a js* handler. Ungrabbed + virtual ones
        // are intentionally included (this is a diagnostics enumerator).
        if !has_btn_south && !has_js {
            continue;
        }
        let id = dev.input_id();
        rows.push(crate::protocol::InputDeviceInfo {
            name: dev.name().unwrap_or("unknown").to_string(),
            path: path.to_string_lossy().to_string(),
            vendor: id.vendor(),
            product: id.product(),
            phys: dev.physical_path().unwrap_or("").to_string(),
            handlers,
            grabbed: grabbed_paths.contains(&path),
        });
    }
    crate::protocol::resp_input_devices(&rows)
}

// --- hot-join / leave ----------------------------------------------------

/// Discover any newly-connected pads and add them to the fleet. Skips pads
/// already in the fleet (by **device path** — an already-grabbed pad
/// re-enumerates at the same path but a fresh fd) and our own virtual devices
/// (by fd, inside `find_gamepads`). Each joining pad is grabbed (shell
/// presenter), calibrated, assigned the lowest free slot, and announced via
/// `controller-wake` + `pad:connected:{id,index,name}`.
pub(crate) fn try_join(sh: &mut Shared, fleet: &mut Fleet) {
    // Paths already represented in the fleet, so a re-enumeration of a
    // connected pad doesn't open + grab it a second time.
    let known_paths: HashSet<PathBuf> = fleet.pads.values().map(|p| p.path.clone()).collect();

    for handle in device::find_gamepads(&sh.db, &sh.reg) {
        // Destructure before consuming `device` (into_event_stream moves it).
        let device::GamepadHandle {
            device,
            name,
            path,
            wire_id,
        } = handle;
        if known_paths.contains(&path) {
            continue; // already in the fleet
        }
        let stream = match device.into_event_stream() {
            Ok(s) => s,
            Err(e) => {
                error!("Failed to open event stream for {}: {e}", path.display());
                continue;
            }
        };
        let fd = stream.device().as_raw_fd();
        // A freshly-opened physical pad's fd can't already be in the fleet;
        // guard anyway so a duplicate enumeration never double-inserts.
        if fleet.pads.contains_key(&fd) {
            continue;
        }
        let slot = fleet.slots.alloc();
        info!(
            "Pad joined: {} at {} (id={}, slot={})",
            name,
            path.display(),
            wire_id,
            slot,
        );
        let mut pad = PadDevice::new(fd, stream, wire_id.clone(), name.clone(), path, slot);
        pad.calibrate();
        // Match the joining pad to the fleet's current presenter:
        //   * Shell / Keyboard — grab so its input drives the shell key-map (nav,
        //     or a keyboard-contract app like Plex). No virtual pad in either.
        //   * Game  — grab + clean virtual gamepad (a 2nd player joining a stream
        //     that runs through the virtual-pad path).
        //   * Handoff (#221) — leave it UNGRABBED so SDL/Moonlight reads the real
        //     evdev node directly, exactly like the pads already handed off.
        match sh.presenter {
            Presenter::Shell | Presenter::Keyboard => pad.grab(sh),
            Presenter::Game => {
                pad.grab(sh);
                pad.enter_game(sh);
            }
            Presenter::Handoff => { /* leave ungrabbed — SDL reads it directly */ }
        }
        // Overlay-focus layered on top of the base setup (#262): a modal shell
        // overlay is open over the app, so force the grab even for a Handoff
        // base — the joining pad drives the overlay via the shell key-map
        // (`handle_event` routes it there while `overlay_focus` is on). Idempotent
        // for the Shell/Game arms above; `Game` keeps its virtual pad so the
        // clean overlay-off restore forwards correctly.
        if sh.overlay_focus {
            pad.grab(sh);
        }
        // Fleet outputs (ride-along, Phase 5.5): light the player LED to match
        // the slot (#101 LED) and read the initial battery (#100). Both no-op on
        // pads lacking the capability. A short connect rumble (#99) gives haptic
        // feedback that the pad is live, gated by the `rumbleEnabled` setting and
        // the pad's FF support.
        pad.set_player_led(sh);
        pad.poll_battery(sh);
        if sh.rumble_enabled {
            pad.rumble(CONNECT_RUMBLE_MS);
        }
        fleet.pads.insert(fd, pad);

        // Legacy single-pad signal (any pad still drives `controller-wake` so the
        // existing QML wake path is unchanged), plus the fleet-aware
        // `pad:connected:{id,index,name}`.
        sh.publish(Event::ControllerWake);
        sh.publish(Event::PadConnected(pad_connected_json(
            &wire_id, slot, &name,
        )));
    }
}

/// A pad's stream errored (USB disconnect): drop it from the fleet, free its
/// slot for reuse, abort its tasks, and announce the leave.
pub(crate) fn on_pad_leave(sh: &mut Shared, fleet: &mut Fleet, fd: RawFd) {
    let Some(mut pad) = fleet.pads.remove(&fd) else {
        return;
    };
    warn!(
        "Pad left: slot {} ({}), freeing slot",
        pad.player_slot, pad.wire_id
    );
    pad.reset_stick_state(sh);
    pad.abort_all_tasks();
    // Drop any per-player virtual pad and forget its devnode (Phase 5 game presenter).
    if let Some(mut vpad) = pad.virtual_pad.take() {
        unregister_vpad_devnodes(&mut sh.reg, &mut vpad);
    }
    let slot = pad.player_slot;
    let wire_id = pad.wire_id.clone();
    fleet.slots.free(slot);
    drop(pad);

    // Legacy single-pad signal + fleet-aware `pad:disconnected:{id}`.
    sh.publish(Event::ControllerDisconnected);
    sh.publish(Event::PadDisconnected(wire_id));
}
