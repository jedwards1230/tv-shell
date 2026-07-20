.pragma library

// Pure resume-focus decision logic (#347).
//
// WHY THIS IS A SEPARATE LIBRARY: the resume path is the one place in the shell
// where a wrong decision is INVISIBLE — `hyprctl dispatch` exits 0 even when its
// selector matched no window, so a resume that focuses nothing looks exactly
// like a resume that worked. That made #347 take four hypotheses to corner. The
// decision ("which selector do we focus with, and did it land?") is therefore
// pulled out of AppLifecycleManager into pure functions that can be asserted
// headlessly, so the branch that used to be a silent `return` is now pinned by
// tests rather than by a device.
//
// Mirrors the prewarm.js / resumeModel.js pattern: no QML types, no imports, all
// inputs passed in.

// How a resume should be dispatched.
//   ADDRESS — the address is a window we currently know about; focus that exact
//             window. The precise case, and the only one that may claim the
//             address as the tracked foreground window (#203).
//   CLASS   — the address is NOT in our window snapshot but the caller supplied
//             the window's class. Our snapshot is a poll a few seconds old, so
//             an address miss usually means WE are stale, not that the app is
//             gone; focusing by class still reaches the live window. Strictly
//             better than the silent `return` this replaces.
//   NONE    — nothing actionable: no known address AND no class. The caller must
//             log this rather than return quietly (that silence WAS the bug).
var MODE_ADDRESS = "address";
var MODE_CLASS = "class";
var MODE_NONE = "none";

// Reasons, logged verbatim as the `reason=` trace field.
var REASON_NO_ADDRESS = "no-address";
var REASON_UNKNOWN_ADDRESS = "unknown-address";
var REASON_NO_ACTIVE_WINDOW = "no-active-window";
var REASON_ADDRESS_MISMATCH = "active-address-mismatch";
var REASON_CLASS_MISMATCH = "active-class-mismatch";
var REASON_NO_TARGET = "no-target";
var REASON_ALREADY_FULLSCREEN = "already-fullscreen";

function _s(v) {
    return (v === undefined || v === null) ? "" : String(v);
}

// Decide how to focus a resume request.
//   address        — the Hyprland window address the UI row carried.
//   windowClass    — the row's window class, when the caller has one ("" if not).
//   runningWindows — the current window snapshot (AppLifecycleManager.runningWindows).
//
// Returns { mode, address, windowClass, reason }. `windowClass` on an ADDRESS
// hit comes from the SNAPSHOT (authoritative) rather than the caller's argument.
function resolve(address, windowClass, runningWindows) {
    var addr = _s(address);
    var cls = _s(windowClass);
    var windows = runningWindows || [];

    if (addr !== "") {
        for (var i = 0; i < windows.length; i++) {
            var w = windows[i];
            if (w && _s(w.address) === addr) {
                return {
                    mode: MODE_ADDRESS,
                    address: addr,
                    windowClass: _s(w.windowClass),
                    reason: ""
                };
            }
        }
    }

    var reason = (addr === "") ? REASON_NO_ADDRESS : REASON_UNKNOWN_ADDRESS;
    if (cls !== "") {
        return {
            mode: MODE_CLASS,
            address: addr,
            windowClass: cls,
            reason: reason
        };
    }
    return {
        mode: MODE_NONE,
        address: addr,
        windowClass: "",
        reason: reason
    };
}

// Did a focus dispatch actually land?
//
// EXIT CODE CANNOT ANSWER THIS. `hyprctl dispatch focuswindow class:nope` exits
// 0 and prints "ok" — a selector that matched nothing is indistinguishable from
// a selector that worked. The only real evidence is the compositor's own
// active-window read afterwards (the daemon's `hypr-active` IPC), which is what
// this compares against the decision we acted on.
//
//   decision — the object returned by resolve() above.
//   active   — the parsed `hypr-active` reply: {class,address,fullscreen}, or {}
//              when nothing is focused.
//
// Returns { ok, reason }. Class comparison is case-insensitive because Hyprland
// reports the window's own class casing (`tv.plex.Plex`) while our callers may
// hold a lowercased StartupWMClass.
function verifyFocus(decision, active) {
    var d = decision || {};
    var a = active || {};
    var activeAddr = _s(a.address);
    var activeCls = _s(a["class"]);

    if (d.mode === MODE_ADDRESS) {
        if (activeAddr === "")
            return {
                ok: false,
                reason: REASON_NO_ACTIVE_WINDOW
            };
        var addrOk = activeAddr === _s(d.address);
        return {
            ok: addrOk,
            reason: addrOk ? "" : REASON_ADDRESS_MISMATCH
        };
    }

    if (d.mode === MODE_CLASS) {
        if (activeCls === "")
            return {
                ok: false,
                reason: REASON_NO_ACTIVE_WINDOW
            };
        var clsOk = activeCls.toLowerCase() === _s(d.windowClass).toLowerCase();
        return {
            ok: clsOk,
            reason: clsOk ? "" : REASON_CLASS_MISMATCH
        };
    }

    // Nothing was dispatched, so nothing can have landed.
    return {
        ok: false,
        reason: REASON_NO_TARGET
    };
}

// May the resume path issue `hyprctl dispatch fullscreen 0 set`?
//
// THE ORDERING REQUIREMENT THIS FUNCTION EXISTS TO ENFORCE — do not "optimize"
// it away. Hyprland's `fullscreen` dispatcher takes NO window selector: it acts
// on whatever is ACTIVE at the instant it runs (verified on-device; the daemon's
// own force_fullscreen in daemon/src/hyprland.rs must dispatch `focuswindow
// address:<a>` FIRST for exactly this reason, and `hyprctl dispatch fullscreen 0
// set` prints "Window not found" and still exits 0 when nothing is active). The
// compositor applies focus asynchronously, so at the moment our focus Process
// returns, the active window may STILL BE THE PREVIOUS ONE. Asserting fullscreen
// there would fullscreen that previous window — in the #347 scenario (resume a
// tiled Plex while a fullscreen Steam is active) it would re-assert fullscreen on
// STEAM, which is the very bug this path is meant to fix. `set` being idempotent
// does not save us: it is idempotent in WHICH STATE it applies, not in WHICH
// WINDOW it applies to.
//
// So the assertion is gated on the SAME verified `hypr-active` read that
// verifyFocus consumes. Only once the compositor itself reports our intended
// window as active is "the active window" provably the right target. The cost is
// one settle interval before the window goes fullscreen; the alternative is a
// coin-flip on which window gets fullscreened.
//
// Mirrors the daemon's `needs_fullscreen` skip conditions (something IS focused,
// and it isn't already fullscreen) so QML and the daemon share one idiom rather
// than competing.
//
//   decision — the object returned by resolve().
//   active   — the parsed `hypr-active` reply: {class,address,fullscreen}.
//
// Returns { assert, reason }. `reason` is why we are NOT asserting, "" when we
// are. Fail-safe direction: only an EXPLICIT `fullscreen === true` suppresses the
// dispatch, so a missing/unknown field still asserts (a redundant idempotent
// `set` is harmless; a skipped needed one leaves the window invisible).
function shouldAssertFullscreen(decision, active) {
    var landed = verifyFocus(decision, active);
    if (!landed.ok)
        return {
            assert: false,
            reason: landed.reason
        };
    if ((active || {}).fullscreen === true)
        return {
            assert: false,
            reason: REASON_ALREADY_FULLSCREEN
        };
    return {
        assert: true,
        reason: ""
    };
}
