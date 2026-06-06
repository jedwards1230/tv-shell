import QtQuick

// ScreenshotFlash (#166)
//
// A self-contained full-screen white vignette that plays a brief (~150 ms)
// fade-in / fade-out animation as visual feedback when a remote screenshot is
// taken via `GET /screenshot?flash=1`.
//
// Usage:
//   1. Instantiate once as the LAST child of ShellRoot (so it renders on top
//      of everything else) and assign an id.
//   2. Call `flash()` when the `inputManager.screenshotFlash` signal fires.
//
// The component is purely cosmetic and stateless: it neither reads nor writes
// any shared state.  It is safe to call `flash()` while a previous animation
// is still running — the animation restarts from full opacity for a clean
// double-flash.
Item {
    id: root

    // Fill the parent (expected to be anchors.fill: parent at the call site).
    anchors.fill: parent

    // Invisible until flash() is called.
    visible: false

    // The white vignette — full-screen, semi-opaque white rectangle.
    Rectangle {
        id: vignette
        anchors.fill: parent
        color: "white"
        opacity: 0.0

        SequentialAnimation {
            id: flashAnim
            running: false

            // Fade in quickly (≈ 40 ms).
            NumberAnimation {
                target: vignette
                property: "opacity"
                to: 0.85
                duration: 40
                easing.type: Easing.OutQuad
            }
            // Hold briefly.
            PauseAnimation {
                duration: 30
            }
            // Fade out over the remainder of the ~150 ms window.
            NumberAnimation {
                target: vignette
                property: "opacity"
                to: 0.0
                duration: 80
                easing.type: Easing.InQuad
            }

            onStopped: {
                root.visible = false;
            }
        }
    }

    // Public API: trigger a single flash cycle.
    function flash() {
        root.visible = true;
        flashAnim.restart();
    }
}
