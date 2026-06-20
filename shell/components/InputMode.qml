pragma Singleton
import QtQuick

// Input-mode state, extracted from Theme (#45 follow-up). Owns the single
// "is the mouse/right-stick driving focus?" flag plus the global-pointer delta
// filter that decides when a pointer event is a *genuine* move.
//
// This is QML-owned and multi-source: a physical K400 mouse never reaches the
// daemon, so hover/move must flip mouse-mode with NO daemon round-trip. The
// daemon's `input-mode:*` event (InputManager) is just one more source (its
// right-stick→cursor case). Writes funnel through enterMouseMode()/exitMouseMode()
// so a redundant set never emits a spurious mouseModeChanged() (every consumer
// re-syncs focus on that signal).
//
// Lives in its own singleton (not Theme) so input state and visual theme are
// separable; Theme re-exports mouseMode as a pass-through for call sites not yet
// migrated.
Item {
    // true when mouse/right-stick is driving focus, false for controller/D-pad.
    property bool mouseMode: false

    // A real Wayland pointer event (hover/move/click) — switch to mouse mode.
    function enterMouseMode() {
        if (!mouseMode)
            mouseMode = true;
    }

    // A key or gamepad-nav event — switch back to controller mode.
    function exitMouseMode() {
        if (mouseMode)
            mouseMode = false;
    }

    // Last sampled GLOBAL pointer position (-1 = no sample yet). Tracked here so
    // every hover handler shares one delta filter instead of each MouseArea
    // guessing whether the pointer actually moved.
    property real _lastPointerX: -1
    property real _lastPointerY: -1

    // Called from MouseArea.onPositionChanged with the GLOBAL pointer coords.
    // Only flips to mouse mode on a *genuine* pointer move. Why global coords:
    // when content scrolls under a still cursor, onPositionChanged fires because
    // the item moved under the pointer — but the pointer's GLOBAL position is
    // UNCHANGED (the local coords shift by exactly the same delta the item moved,
    // so mapToGlobal cancels it out). A real mouse move changes the global
    // position. The first sample only records a baseline (never flips), so the
    // initial hover that lands when a row scrolls into place can't trip it.
    function pointerMoved(gx, gy) {
        if (_lastPointerX >= 0 && (Math.abs(gx - _lastPointerX) + Math.abs(gy - _lastPointerY) > 1.0))
            enterMouseMode();
        _lastPointerX = gx;
        _lastPointerY = gy;
    }
}
