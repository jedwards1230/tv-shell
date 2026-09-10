//! Pad → keyboard translation, for the route that drives the shell.
//!
//! Pure: no syscall, no device, and **no clock**. Every time-dependent decision
//! takes `now: Instant` from its caller, so the stick auto-repeat below is
//! exercised deterministically in CI rather than by sleeping.
//!
//! # Why keys at all
//!
//! `V2_GAMEPAD_HANDOFF.md` §1: a pad is read by the app straight off
//! `/dev/input/eventN` and gamescope never routes it, while a **key** goes
//! libinput → gamescope → the focused client. Qt 6 has no gamepad input
//! (QtGamepad was removed), so a controller can only reach the v2 shell as keys,
//! and those keys must be synthesised by something that already owns the pad
//! stream. That is this crate. §4 Q2 settles it as option (a), measured: a uinput
//! keyboard drives the v2 shell with Moonlight running or suspended.
//!
//! # Only codes measured to ARRIVE are emitted
//!
//! gamescope **silently drops `KEY_MENU`** (§2.1, measured on htpc-1
//! 2026-09-10): XTEST `Menu` — injected inside Xwayland, past gamescope — opened
//! the drawer, and the same key from a uinput device never reached the client at
//! all. That is why the drawer moved to Tab (jedwards1230/tv-shell#490) and why
//! [`VERIFIED_CODES`] exists: a mapping to a code nobody watched arrive is a
//! button that does nothing, with no error anywhere to say so.
//!
//! # What is deliberately NOT here
//!
//! * **Guide / `BTN_MODE`.** [`key_for_button`] returns `None` for it on
//!   purpose, and that stays true now that the Guide escape exists
//!   (jedwards1230/tv-shell#496): a held Guide is a core-side `home` — a
//!   base-layer write performed by [`super::escape`] — never a key. It is
//!   intercepted in [`super::session`] before this module is reached, on every
//!   route, so nothing here ever sees it.
//! * **Masking** (`masked_keys` / `masked_axes`): phase 3.
//! * **The right stick** (v1 drove a mouse from it) and remappable bindings:
//!   dropped for now, per the handoff plan §5.

use std::collections::BTreeSet;
use std::time::Instant;

use super::presenter::{abs, btn, ev, AbsRange};

/// Kernel key codes (`input-event-codes.h`).
///
/// Only the ones this module can emit, plus the `1..=31` block the keyboard
/// profile must advertise — see [`KeyboardProfile`].
pub mod key {
    pub const ESC: u16 = 1;
    pub const TAB: u16 = 15;
    pub const ENTER: u16 = 28;
    pub const UP: u16 = 103;
    pub const LEFT: u16 = 105;
    pub const RIGHT: u16 = 106;
    pub const DOWN: u16 = 108;
}

/// The key codes **measured to reach the v2 shell through gamescope**
/// (V2_GAMEPAD_HANDOFF §2.1, htpc-1, 2026-09-09/10).
///
/// `KEY_MENU` is absent because it was measured NOT to arrive, which is the
/// whole reason this list is a list and not a comment.
pub const VERIFIED_CODES: [u16; 11] = [
    key::ESC,
    key::TAB,
    key::ENTER,
    key::UP,
    key::LEFT,
    key::RIGHT,
    key::DOWN,
    57,  // KEY_SPACE
    59,  // KEY_F1
    60,  // KEY_F2
    102, // KEY_HOME
];

/// One key event for the core's uinput keyboard.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyEmit {
    pub code: u16,
    /// `1` press, `0` release. Never `2`: an autorepeat this crate did not
    /// decide is one it cannot cancel, and the stick repeat below is expressed
    /// as release-then-press for exactly that reason.
    pub value: i32,
}

/// The capability profile of the core's uinput keyboard.
///
/// # The 1..=31 rule, which cost a measurement cycle
///
/// systemd's `input_id` builtin sets `ID_INPUT_KEYBOARD` **only when key codes
/// 1..=31 are all advertised**, and without that property libinput does not
/// treat the device as a keyboard at all — so the first attempt, which
/// advertised exactly the keys it meant to send (Esc, the arrows, Menu, A–Z,
/// 0–9), was tagged `ID_INPUT_KEY=1` with no `ID_INPUT_KEYBOARD` and drove
/// nothing (V2_GAMEPAD_HANDOFF §2.1.1).
///
/// So the block below includes `KEY_MINUS`, `KEY_EQUAL`, `KEY_LEFTBRACE`,
/// `KEY_RIGHTBRACE` and `KEY_LEFTCTRL` — codes the shell will never be sent.
/// **Advertising only the keys you intend to send is the intuitive thing to do
/// and it is wrong.**
pub struct KeyboardProfile;

impl KeyboardProfile {
    /// Vendor/product/version/bus for the virtual keyboard.
    ///
    /// Deliberately NOT the pad's `045e:028e`: this device must never look like
    /// a controller to anything enumerating one.
    pub const VENDOR: u16 = 0x1d6b;
    pub const PRODUCT: u16 = 0x0001;
    pub const VERSION: u16 = 0x0001;
    /// `BUS_VIRTUAL`.
    pub const BUS: u16 = 0x06;

    /// The device name.
    pub fn device_name() -> String {
        "tv-shell-keys".to_string()
    }

    /// Every key code the device advertises: the mandatory `1..=31` block, plus
    /// every code this module can actually emit.
    pub fn keys() -> Vec<u16> {
        let mut keys: BTreeSet<u16> = (1..=31).collect();
        keys.extend(VERIFIED_CODES);
        keys.into_iter().collect()
    }
}

/// The key a pad button becomes, or `None` for one that is not routed.
///
/// Matches v1's default bindings (`daemon/src/config.rs::default_bindings`) for
/// the three the v2 shell actually handles: A activates, B goes back, Start
/// toggles the drawer. v1's `altSelect`/`altAction` (Y → Tab, X → `KEY_X`) are
/// left out — the v2 shell binds neither, and `KEY_X` is not in
/// [`VERIFIED_CODES`].
///
/// **Mutation note.** Add `btn::MODE => key::ESC` and
/// `guide_is_never_a_key` fails; map anything to `KEY_MENU` (0x8b)
/// and `every_emittable_code_was_measured_to_arrive` fails.
pub fn key_for_button(code: u16) -> Option<u16> {
    match code {
        btn::SOUTH => Some(key::ENTER),
        btn::EAST => Some(key::ESC),
        // The drawer. Tab, not Menu: gamescope does not deliver KEY_MENU
        // (§2.1), so the shell's Menu binding is unreachable from any real
        // device and Tab is the one that arrives.
        btn::START => Some(key::TAB),
        // BTN_MODE (Guide) is handled by the core itself, not as a key — see
        // the module docs. Everything else has no shell binding to reach.
        _ => None,
    }
}

/// v1's calibrated stick-to-d-pad constants
/// (`daemon/src/config.rs`), tuned on this hardware and ported unchanged.
pub mod repeat {
    /// Fraction of the axis half-range a stick must pass to count as deflected.
    pub const DEADZONE: f64 = 0.30;
    /// How long a deflection is held before it starts repeating.
    pub const INITIAL_DELAY: std::time::Duration = std::time::Duration::from_millis(300);
    /// The gap between repeats thereafter.
    pub const INTERVAL: std::time::Duration = std::time::Duration::from_millis(150);
}

/// The centre and deflection threshold for one axis, from the pad's own range.
///
/// Ported from v1's `Pad::calibrate`: centre is the midpoint of the reported
/// range and the threshold is [`repeat::DEADZONE`] of the half-range, so a pad
/// reporting `0..255` and one reporting `-32768..32767` behave identically.
fn calibrate(range: AbsRange) -> (i32, i32) {
    let center = (range.min + range.max) / 2;
    let half = (range.max - range.min) / 2;
    (center, (half as f64 * repeat::DEADZONE) as i32)
}

/// Which direction an axis is deflected in, as a key.
///
/// Ported from v1's `state::left_stick_target`, including its strict
/// comparisons: exactly at the threshold is still centre.
fn direction(value: i32, center: i32, threshold: i32, neg: u16, pos: u16) -> Option<u16> {
    let offset = value - center;
    if offset < -threshold {
        Some(neg)
    } else if offset > threshold {
        Some(pos)
    } else {
        None
    }
}

/// One latched axis: the key it currently holds, and when that key next repeats.
#[derive(Debug, Default, Clone, Copy)]
struct Latch {
    key: Option<u16>,
    /// `None` for an axis that does not auto-repeat (the d-pad hat, matching
    /// v1) or one at rest.
    next_repeat: Option<Instant>,
}

/// Per-pad translation state.
///
/// One of these per claimed pad, so two players leaning on two sticks keep
/// independent repeat timers — v1's per-pad `stick_x_repeat` / `stick_y_repeat`,
/// without the tasks.
#[derive(Debug, Default)]
pub struct KeyMap {
    stick_x: Latch,
    stick_y: Latch,
    hat_x: Latch,
    hat_y: Latch,
    /// Every key this map believes is pressed, so a pad that leaves can be
    /// unwound. Without it a pad unplugged mid-deflection leaves the shell
    /// holding an arrow key nothing will ever release.
    held: BTreeSet<u16>,
}

impl KeyMap {
    pub fn new() -> KeyMap {
        KeyMap::default()
    }

    /// Translate one physical pad event.
    ///
    /// `source_axis` is the pad's own `absinfo` range for an `EV_ABS` code, read
    /// by the backend at claim time and passed in so this stays pure. `None`
    /// falls back to the canonical stick range.
    pub fn on_event(
        &mut self,
        event_type: u16,
        code: u16,
        value: i32,
        source_axis: Option<AbsRange>,
        now: Instant,
    ) -> Vec<KeyEmit> {
        match event_type {
            ev::KEY => self.on_button(code, value),
            ev::ABS => self.on_axis(code, value, source_axis, now),
            _ => Vec::new(),
        }
    }

    fn on_button(&mut self, code: u16, value: i32) -> Vec<KeyEmit> {
        let Some(key) = key_for_button(code) else {
            return Vec::new();
        };
        match value {
            1 => self.press(key),
            0 => self.release(key),
            // A kernel autorepeat (`2`) on a gamepad button is not something the
            // shell should see twice: the press already landed, and Qt does its
            // own repeat. Swallowed rather than forwarded.
            _ => Vec::new(),
        }
    }

    fn on_axis(
        &mut self,
        code: u16,
        value: i32,
        source_axis: Option<AbsRange>,
        now: Instant,
    ) -> Vec<KeyEmit> {
        // Which latch, which key pair, and whether this axis repeats.
        let (neg, pos, repeats) = match code {
            abs::X | abs::HAT0X => (key::LEFT, key::RIGHT, code == abs::X),
            abs::Y | abs::HAT0Y => (key::UP, key::DOWN, code == abs::Y),
            // Triggers, the right stick and anything else drive nothing here.
            _ => return Vec::new(),
        };
        let range = source_axis.unwrap_or(match code {
            abs::HAT0X | abs::HAT0Y => AbsRange::new(-1, 1, 0, 0),
            _ => AbsRange::new(-32768, 32767, 16, 128),
        });
        let (center, threshold) = calibrate(range);
        let want = direction(value, center, threshold, neg, pos);

        let latch = match code {
            abs::X => &mut self.stick_x,
            abs::Y => &mut self.stick_y,
            abs::HAT0X => &mut self.hat_x,
            _ => &mut self.hat_y,
        };
        if latch.key == want {
            return Vec::new();
        }
        let previous = latch.key.take();
        latch.key = want;
        latch.next_repeat = match (want, repeats) {
            (Some(_), true) => Some(now + repeat::INITIAL_DELAY),
            _ => None,
        };

        let mut out = Vec::new();
        // Release the old direction BEFORE pressing the new one, or a stick
        // flicked across centre leaves both arrows held.
        if let Some(old) = previous {
            out.extend(self.release(old));
        }
        if let Some(new) = want {
            out.extend(self.press(new));
        }
        out
    }

    /// The earliest armed repeat, if any. The runtime sleeps until this.
    pub fn next_deadline(&self) -> Option<Instant> {
        [self.stick_x, self.stick_y, self.hat_x, self.hat_y]
            .iter()
            .filter_map(|l| l.next_repeat)
            .min()
    }

    /// Fire every repeat that is due at `now`.
    ///
    /// A repeat is **release then press**, which is v1's `StickRepeat`
    /// (`daemon/src/input/mod.rs`: "emit up+down"). A synthesised `value = 2`
    /// would be the intuitive alternative and is wrong here: Qt treats a
    /// repeated key as a distinct event only when it sees the down edge, and a
    /// repeat this crate cannot cancel is one that outlives the deflection.
    pub fn tick(&mut self, now: Instant) -> Vec<KeyEmit> {
        let mut out = Vec::new();
        for latch in [
            &mut self.stick_x,
            &mut self.stick_y,
            &mut self.hat_x,
            &mut self.hat_y,
        ] {
            let (Some(key), Some(due)) = (latch.key, latch.next_repeat) else {
                continue;
            };
            if due > now {
                continue;
            }
            latch.next_repeat = Some(now + repeat::INTERVAL);
            out.push(KeyEmit {
                code: key,
                value: 0,
            });
            out.push(KeyEmit {
                code: key,
                value: 1,
            });
        }
        out
    }

    /// Release everything this map is holding and disarm every repeat.
    ///
    /// The keyboard's equivalent of `presenter::quiesce`, and needed for the
    /// same reason: the keyboard outlives the pad, so a pad that leaves
    /// mid-press would otherwise leave the shell with a key down forever.
    pub fn quiesce(&mut self) -> Vec<KeyEmit> {
        self.stick_x = Latch::default();
        self.stick_y = Latch::default();
        self.hat_x = Latch::default();
        self.hat_y = Latch::default();
        let held = std::mem::take(&mut self.held);
        held.into_iter()
            .map(|code| KeyEmit { code, value: 0 })
            .collect()
    }

    fn press(&mut self, code: u16) -> Vec<KeyEmit> {
        if !self.held.insert(code) {
            // Already down: a second press would be a phantom activation, and
            // the release that eventually arrives would be the only edge.
            return Vec::new();
        }
        vec![KeyEmit { code, value: 1 }]
    }

    fn release(&mut self, code: u16) -> Vec<KeyEmit> {
        if !self.held.remove(&code) {
            return Vec::new();
        }
        vec![KeyEmit { code, value: 0 }]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    const STICK: AbsRange = AbsRange::new(-32768, 32767, 16, 128);
    const HAT: AbsRange = AbsRange::new(-1, 1, 0, 0);

    fn t0() -> Instant {
        Instant::now()
    }

    /// **Rule: every code this module can emit was measured to arrive.**
    ///
    /// The one that matters most, because its failure is invisible: gamescope
    /// drops `KEY_MENU` silently, so a button mapped to it is a button that does
    /// nothing with no error anywhere (§2.1).
    ///
    /// **Mutation note.** Point any arm of `key_for_button` at `0x8b`
    /// (`KEY_MENU`) — the intuitive drawer key, and the one that was actually
    /// tried on hardware — and this fails. It also fails for any code added to
    /// the map without being added to the measured list, which is the point: the
    /// list is evidence, not documentation.
    #[test]
    fn every_emittable_code_was_measured_to_arrive() {
        // The whole gamepad button space, not a hand-picked few, so a mapping
        // added for a button nobody thought to test here is still checked.
        for code in 0x130u16..=0x13f {
            if let Some(key) = key_for_button(code) {
                assert!(
                    VERIFIED_CODES.contains(&key),
                    "button {code:#x} emits {key}, which was never measured to reach the shell"
                );
            }
        }
        // And the axes, through the real entry point.
        let mut map = KeyMap::new();
        for (code, range) in [
            (abs::X, STICK),
            (abs::Y, STICK),
            (abs::HAT0X, HAT),
            (abs::HAT0Y, HAT),
        ] {
            for value in [range.min, range.max] {
                for emit in map.on_event(ev::ABS, code, value, Some(range), t0()) {
                    assert!(
                        VERIFIED_CODES.contains(&emit.code),
                        "axis {code:#x} emits {}, which was never measured to arrive",
                        emit.code
                    );
                }
            }
            map.quiesce();
        }
    }

    /// **Rule: Guide is never a key.**
    ///
    /// It is a core-side `home` (a base-layer write), and mapping it to a key
    /// would leak it to whatever holds focus — v1's stated reason for handling
    /// `BTN_MODE` directly rather than binding it. Still true with the escape
    /// shipped: the session intercepts Guide before this module sees it.
    ///
    /// **Mutation note.** Give `btn::MODE` any mapping and this fails.
    #[test]
    fn guide_is_never_a_key() {
        assert_eq!(key_for_button(btn::MODE), None);
        let mut map = KeyMap::new();
        assert!(map.on_event(ev::KEY, btn::MODE, 1, None, t0()).is_empty());
        assert!(map.on_event(ev::KEY, btn::MODE, 0, None, t0()).is_empty());
        assert!(
            map.quiesce().is_empty(),
            "a Guide press must not even be recorded as held"
        );
    }

    /// The three buttons the v2 shell actually binds reach it, pressed and
    /// released. Without this the rule above passes vacuously on a map that
    /// emits nothing at all.
    #[test]
    fn the_bound_buttons_press_and_release() {
        let mut map = KeyMap::new();
        for (button, expect) in [
            (btn::SOUTH, key::ENTER),
            (btn::EAST, key::ESC),
            (btn::START, key::TAB),
        ] {
            assert_eq!(
                map.on_event(ev::KEY, button, 1, None, t0()),
                vec![KeyEmit {
                    code: expect,
                    value: 1
                }]
            );
            assert_eq!(
                map.on_event(ev::KEY, button, 0, None, t0()),
                vec![KeyEmit {
                    code: expect,
                    value: 0
                }]
            );
        }
    }

    /// **Rule: a button already down is not pressed twice.**
    ///
    /// A pad that repeats its own buttons, or a duplicate event after a
    /// `SYN_DROPPED`, would otherwise deliver a second down edge — which the
    /// shell reads as a second activation.
    ///
    /// **Mutation note.** Make `press` unconditional and this fails.
    #[test]
    fn a_held_button_does_not_re_press() {
        let mut map = KeyMap::new();
        assert_eq!(map.on_event(ev::KEY, btn::SOUTH, 1, None, t0()).len(), 1);
        assert!(map.on_event(ev::KEY, btn::SOUTH, 1, None, t0()).is_empty());
        assert!(
            map.on_event(ev::KEY, btn::SOUTH, 2, None, t0()).is_empty(),
            "a kernel autorepeat is not a new press"
        );
        assert_eq!(map.on_event(ev::KEY, btn::SOUTH, 0, None, t0()).len(), 1);
    }

    /// **Rule: the deadzone is a fraction of the pad's OWN range.**
    ///
    /// v1 calibrated per device because a pad reporting `0..255` and one
    /// reporting `-32768..32767` are the same stick. A fixed absolute threshold
    /// would make one of them hair-trigger and the other unreachable.
    ///
    /// **Mutation note.** Replace `calibrate`'s half-range scaling with a
    /// constant and the `0..255` half of this fails.
    #[test]
    fn the_deadzone_scales_with_the_pads_own_range() {
        let byte = AbsRange::new(0, 255, 0, 0);
        let mut map = KeyMap::new();
        // Just inside the deadzone on a byte-ranged pad (centre 127, threshold
        // 38): no key.
        assert!(map
            .on_event(ev::ABS, abs::X, 160, Some(byte), t0())
            .is_empty());
        // Past it: Right.
        assert_eq!(
            map.on_event(ev::ABS, abs::X, 250, Some(byte), t0()),
            vec![KeyEmit {
                code: key::RIGHT,
                value: 1
            }]
        );

        // The same raw value on a full-range pad is nowhere near deflected.
        let mut map = KeyMap::new();
        assert!(map
            .on_event(ev::ABS, abs::X, 250, Some(STICK), t0())
            .is_empty());
    }

    /// **Rule: crossing centre releases the old direction before pressing the
    /// new one.**
    ///
    /// A stick flicked left-to-right in one event must not leave Left held.
    ///
    /// **Mutation note.** Emit the press before the release and the ORDER
    /// assertion fails; skip the release entirely and the count does.
    #[test]
    fn crossing_centre_releases_before_it_presses() {
        let mut map = KeyMap::new();
        map.on_event(ev::ABS, abs::X, -32768, Some(STICK), t0());
        let out = map.on_event(ev::ABS, abs::X, 32767, Some(STICK), t0());
        assert_eq!(
            out,
            vec![
                KeyEmit {
                    code: key::LEFT,
                    value: 0
                },
                KeyEmit {
                    code: key::RIGHT,
                    value: 1
                },
            ]
        );
    }

    /// **Rule: v1's repeat timing, ported unchanged — 300 ms, then every
    /// 150 ms.**
    ///
    /// **Mutation note.** Swap `INITIAL_DELAY` and `INTERVAL`, or drop either
    /// re-arm, and the assertions below fail at a specific instant rather than
    /// in aggregate — the deadline is checked either side of each boundary.
    #[test]
    fn a_held_stick_repeats_after_the_calibrated_delay() {
        let t = t0();
        let mut map = KeyMap::new();
        assert_eq!(
            map.on_event(ev::ABS, abs::Y, 32767, Some(STICK), t),
            vec![KeyEmit {
                code: key::DOWN,
                value: 1
            }]
        );
        assert_eq!(map.next_deadline(), Some(t + repeat::INITIAL_DELAY));

        // Not yet due.
        assert!(map.tick(t + Duration::from_millis(299)).is_empty());

        // Due: release then press, and the next one is an INTERVAL away, not
        // another INITIAL_DELAY.
        let out = map.tick(t + repeat::INITIAL_DELAY);
        assert_eq!(
            out,
            vec![
                KeyEmit {
                    code: key::DOWN,
                    value: 0
                },
                KeyEmit {
                    code: key::DOWN,
                    value: 1
                },
            ]
        );
        assert_eq!(
            map.next_deadline(),
            Some(t + repeat::INITIAL_DELAY + repeat::INTERVAL)
        );

        // Centring disarms it entirely.
        map.on_event(ev::ABS, abs::Y, 0, Some(STICK), t + Duration::from_secs(1));
        assert_eq!(map.next_deadline(), None);
        assert!(map.tick(t + Duration::from_secs(9)).is_empty());
    }

    /// **Rule: the d-pad does NOT auto-repeat, matching v1.**
    ///
    /// v1 handles `ABS_HAT0*` as a plain press/release with no timer
    /// (`daemon/src/input/pad.rs`), and this is a port, not a redesign.
    ///
    /// **Mutation note.** Arm a repeat for the hat and `next_deadline` stops
    /// being `None` here.
    #[test]
    fn the_dpad_presses_once_and_does_not_repeat() {
        let t = t0();
        let mut map = KeyMap::new();
        assert_eq!(
            map.on_event(ev::ABS, abs::HAT0X, -1, Some(HAT), t),
            vec![KeyEmit {
                code: key::LEFT,
                value: 1
            }]
        );
        assert_eq!(map.next_deadline(), None);
        assert!(map.tick(t + Duration::from_secs(5)).is_empty());
        assert_eq!(
            map.on_event(ev::ABS, abs::HAT0X, 0, Some(HAT), t),
            vec![KeyEmit {
                code: key::LEFT,
                value: 0
            }]
        );
    }

    /// **Rule: a leave releases everything the pad was holding.**
    ///
    /// The keyboard outlives the pad, so nothing downstream can correct a key
    /// left down — the same reasoning as `presenter::quiesce`, and the same
    /// failure if it is missing (a shell stuck navigating right forever).
    ///
    /// **Mutation note.** Make `quiesce` clear `held` without emitting and this
    /// fails; make it emit without clearing the latches and the second
    /// `quiesce` here stops being empty.
    #[test]
    fn quiesce_releases_every_key_the_pad_was_holding() {
        let t = t0();
        let mut map = KeyMap::new();
        map.on_event(ev::KEY, btn::SOUTH, 1, None, t);
        map.on_event(ev::ABS, abs::X, 32767, Some(STICK), t);

        let mut released: Vec<u16> = map.quiesce().into_iter().map(|e| e.code).collect();
        released.sort_unstable();
        assert_eq!(released, vec![key::ENTER, key::RIGHT]);

        assert!(
            map.quiesce().is_empty(),
            "a quiesced map holds nothing more to release"
        );
        assert_eq!(map.next_deadline(), None, "and repeats are disarmed");
    }

    /// **Rule: the keyboard advertises the whole `1..=31` block.**
    ///
    /// systemd's `input_id` sets `ID_INPUT_KEYBOARD` only then, and without that
    /// property libinput does not treat the device as a keyboard at all — the
    /// device is created, looks right in `/proc/bus/input/devices`, and drives
    /// nothing (§2.1.1).
    ///
    /// **Mutation note.** Change `KeyboardProfile::keys` to advertise only
    /// `VERIFIED_CODES` — which is the intuitive implementation, and the one
    /// that was actually built and measured to fail — and this fails.
    #[test]
    fn the_keyboard_profile_advertises_the_block_udev_requires() {
        let keys = KeyboardProfile::keys();
        for code in 1u16..=31 {
            assert!(
                keys.contains(&code),
                "key {code} is missing; without the full 1..=31 block udev does not set \
                 ID_INPUT_KEYBOARD and libinput ignores the device"
            );
        }
        // The named ones from §2.1.1, so a future edit that trims the block
        // fails naming what it broke.
        for (code, name) in [
            (12u16, "KEY_MINUS"),
            (13, "KEY_EQUAL"),
            (26, "KEY_LEFTBRACE"),
            (27, "KEY_RIGHTBRACE"),
            (29, "KEY_LEFTCTRL"),
        ] {
            assert!(keys.contains(&code), "{name} missing");
        }
        // And every code we can actually emit is there too, or the kernel drops
        // it silently.
        for code in VERIFIED_CODES {
            assert!(keys.contains(&code), "emittable key {code} not advertised");
        }
        // It must not look like a pad to anything enumerating one.
        assert_ne!(
            (KeyboardProfile::VENDOR, KeyboardProfile::PRODUCT),
            (0x045e, 0x028e)
        );
    }
}
