//! Virtual gamepad Fleet: the per-player uinput vpad presentation and the
//! shared virtual keyboard/mouse build, plus fleet event multiplexing.
//!
//! Split out of the former monolithic `input.rs` (behavior-preserving).

use super::*;

/// Build a clean per-player virtual gamepad mirroring the physical pad's
/// capabilities (Phase 5). The virtual device advertises the same key set,
/// absolute axes (with the source's calibration), and `input_id` so a game
/// recognizes `tv-shell-virtual-pad-<slot>` as the same controller model —
/// minus the daemon's internal Home/combo synthesis, which never reaches it.
///
/// We deliberately copy Home (`BTN_MODE`) into the *capability* set so the
/// virtual pad's profile matches the physical one, but `handle_game` never
/// forwards a Home event, so the game still never sees a Home press.
pub(crate) fn build_virtual_pad(src: &Device, slot: u8) -> std::io::Result<VirtualDevice> {
    let name = format!("tv-shell-virtual-pad-{slot}");

    let keys: AttributeSet<KeyCode> = src
        .supported_keys()
        .into_iter()
        .flat_map(|set| set.iter())
        .collect();

    let mut builder = VirtualDevice::builder()?
        .name(&name)
        .input_id(src.input_id())
        .with_keys(&keys)?;

    // Copy each absolute axis with the source's absinfo (calibration), so the
    // virtual pad reports identical ranges/deadzones to the game.
    if let Ok(absinfo) = src.get_absinfo() {
        for (code, info) in absinfo {
            let setup = UinputAbsSetup::new(AbsoluteAxisCode(code.0), info);
            builder = builder.with_absolute_axis(&setup)?;
        }
    }

    let vpad = builder.build()?;
    Ok(vpad)
}

/// Register a virtual pad's `/dev/input/eventN` devnode(s) as daemon-owned so
/// fleet discovery skips it. The devnode is the identity that survives
/// `evdev::enumerate` reopening the device (the raw fd is not), which is what
/// discovery actually compares against — a virtual pad copies the physical
/// pad's `input_id`, so without this it would pass the DB gate and be grabbed
/// as a bogus pad on the next discovery poll.
pub(crate) fn register_vpad_devnodes(reg: &mut VirtualRegistry, vpad: &mut VirtualDevice) {
    // The kernel may not have created /dev/input/eventN the instant build()
    // returns; without the node we can't claim ownership and the 2s discovery
    // poll could briefly see the new pad as a candidate (#108). Retry briefly
    // (~100ms total) until the node appears.
    for attempt in 0..10 {
        match vpad.enumerate_dev_nodes_blocking() {
            Ok(nodes) => {
                let mut any = false;
                for node in nodes.flatten() {
                    reg.register(node);
                    any = true;
                }
                if any {
                    return;
                }
            }
            Err(e) => {
                warn!("could not enumerate virtual pad devnodes for ownership: {e}");
                return;
            }
        }
        if attempt < 9 {
            std::thread::sleep(Duration::from_millis(10));
        }
    }
    warn!("virtual pad devnode not present after retries; discovery may briefly see it as a candidate");
}

/// Forget a virtual pad's devnode(s) on teardown (re-enumerates the same paths
/// while the device is still alive).
pub(crate) fn unregister_vpad_devnodes(reg: &mut VirtualRegistry, vpad: &mut VirtualDevice) {
    if let Ok(nodes) = vpad.enumerate_dev_nodes_blocking() {
        for node in nodes.flatten() {
            reg.unregister(&node);
        }
    }
}

/// The gamepad fleet: physical pads keyed by raw fd, plus the stable player-slot
/// allocator (#101).
pub(crate) struct Fleet {
    pub(crate) pads: HashMap<RawFd, PadDevice>,
    pub(crate) slots: SlotAllocator,
}

impl Fleet {
    pub(crate) fn new() -> Fleet {
        Fleet {
            pads: HashMap::new(),
            slots: SlotAllocator::new(),
        }
    }

    /// Fleet aggregate for the `status` reply: connected if any pad is present;
    /// the second field reflects the **presenter** (`grab`→shell→`grabbed`,
    /// `release`→game→`released`), NOT the physical EVIOCGRAB — the pad now stays
    /// grabbed in both modes (Phase 5), so keying `status` off the physical grab
    /// would always report `grabbed` and break the `release` UI semantics that
    /// `ControllerSettings.qml` reads. For a single pad in the shell presenter
    /// this is byte-identical to the pre-fleet `connected:grabbed`.
    ///
    /// [`Presenter::Keyboard`] reports `grabbed` too: like Shell, the daemon is
    /// actively translating the pad (to keyboard/mouse for the focused app) and
    /// holds no virtual pad — the controller drives a UI, it is not "released" to
    /// a game. Only [`Presenter::Game`]/[`Presenter::Handoff`] report released.
    pub(crate) fn status_string(&self, presenter: Presenter) -> String {
        let connected = !self.pads.is_empty();
        let grabbed = matches!(presenter, Presenter::Shell | Presenter::Keyboard);
        resp_status(connected, grabbed)
    }

    /// True if any pad currently holds the Home (`BTN_MODE`) button. Used to
    /// clear the fleet-level Home-hold latch.
    pub(crate) fn any_holds_home(&self) -> bool {
        self.pads
            .values()
            .any(|p| p.held_keys.contains(&cfg::BTN_MODE))
    }

    /// Find a pad by its stable wire id (for the `rumble` command, #99). Linear
    /// scan — the fleet is tiny (a handful of pads) so a map keyed by wire id
    /// isn't worth the extra bookkeeping alongside the fd-keyed map.
    pub(crate) fn find_by_wire_id_mut(&mut self, wire_id: &str) -> Option<&mut PadDevice> {
        self.pads.values_mut().find(|p| p.wire_id == wire_id)
    }

    /// The `get-pads` reply: one JSON object per pad in ascending player-slot
    /// order.
    pub(crate) fn pads_json(&self) -> String {
        let mut pads: Vec<&PadDevice> = self.pads.values().collect();
        pads.sort_by_key(|p| p.player_slot);
        let rows: Vec<(String, u8, String, bool)> = pads
            .iter()
            .map(|p| (p.wire_id.clone(), p.player_slot, p.name.clone(), p.grabbed))
            .collect();
        resp_pads(&rows)
    }
}

/// Await the next event from any pad's stream, tagged with its fd. Pends forever
/// when the fleet is empty (the select arm is guarded on `!pads.is_empty()`).
pub(crate) async fn next_fleet_event(
    fleet: &mut Fleet,
) -> Option<(RawFd, std::io::Result<InputEvent>)> {
    use futures::stream::{FuturesUnordered, StreamExt};
    if fleet.pads.is_empty() {
        return std::future::pending().await;
    }
    // Race every pad's next_event; the first to resolve wins this tick. Each
    // future borrows one pad mutably and yields its fd alongside the result.
    let mut futs = FuturesUnordered::new();
    for (&fd, pad) in fleet.pads.iter_mut() {
        futs.push(async move { (fd, pad.event_stream.next_event().await) });
    }
    futs.next().await
}

pub(crate) fn build_uinput(
    button_map: &HashMap<u16, u16>,
) -> std::io::Result<(VirtualDevice, VirtualDevice)> {
    // Keyboard: all mapped keys (deduped) + the arrows, modifiers, and Q used
    // for d-pad/left-stick and the Moonlight force-quit chord. Enter/Esc are
    // fixed members too (not just transitively via `button_map`) so the
    // `key select`/`key back` IPC always has an advertised keycode to emit,
    // independent of how `select`/`back` are bound — a uinput device silently
    // drops events for keycodes it never declared.
    let mut mapped: Vec<u16> = button_map.values().copied().collect();
    mapped.sort_unstable();
    mapped.dedup();
    let extra = [
        cfg::KEY_UP,
        cfg::KEY_DOWN,
        cfg::KEY_LEFT,
        cfg::KEY_RIGHT,
        cfg::KEY_ENTER,
        cfg::KEY_ESC,
        cfg::KEY_LEFTCTRL,
        cfg::KEY_LEFTALT,
        cfg::KEY_LEFTSHIFT,
        cfg::KEY_Q,
    ];
    let keys: AttributeSet<KeyCode> = mapped
        .iter()
        .chain(extra.iter())
        .map(|&k| KeyCode::new(k))
        .collect();
    let kb = VirtualDevice::builder()?
        .name("tv-shell-virtual-kb")
        .with_keys(&keys)?
        .build()?;
    info!("uinput keyboard device created");

    let mkeys: AttributeSet<KeyCode> = [cfg::BTN_LEFT, cfg::BTN_RIGHT, cfg::BTN_MIDDLE]
        .into_iter()
        .map(KeyCode::new)
        .collect();
    // RelativeAxisCode is a tuple struct `RelativeAxisCode(pub u16)` (no `new`).
    let axes: AttributeSet<RelativeAxisCode> =
        [cfg::REL_X, cfg::REL_Y, cfg::REL_WHEEL, cfg::REL_HWHEEL]
            .into_iter()
            .map(RelativeAxisCode)
            .collect();
    let mouse = VirtualDevice::builder()?
        .name("tv-shell-virtual-mouse")
        .with_keys(&mkeys)?
        .with_relative_axes(&axes)?
        .build()?;
    info!("uinput mouse device created");

    Ok((kb, mouse))
}
