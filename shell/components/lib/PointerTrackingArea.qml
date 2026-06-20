import QtQuick
import "../"

// Mouse-mode-aware MouseArea. Bakes in the pointer-tracking boilerplate that
// every interactive element otherwise repeats (the #45 mouse-mode dance):
// hover-enabled, a pointing cursor, and a genuine-move → InputMode.pointerMoved
// hover handler (global scene coords, delta-filtered) so focus flips into
// mouse-mode only on a real pointer move — not when content scrolls under a
// stationary cursor. A click enters mouse-mode and then emits activated(mouse);
// callers do the actual work in onActivated rather than re-deriving the dance.
//
//   PointerTrackingArea {
//       anchors.fill: parent
//       onActivated: root.doThing()
//   }
MouseArea {
    id: area
    hoverEnabled: true
    cursorShape: Qt.PointingHandCursor

    // Emitted after a click has flipped the shell into mouse-mode. Carries the
    // original mouse event so callers that need coordinates/buttons still have them.
    signal activated(var mouse)

    onPositionChanged: mouse => {
        let p = area.mapToItem(null, mouse.x, mouse.y);
        InputMode.pointerMoved(p.x, p.y);
    }
    onClicked: mouse => {
        InputMode.enterMouseMode();
        area.activated(mouse);
    }
}
