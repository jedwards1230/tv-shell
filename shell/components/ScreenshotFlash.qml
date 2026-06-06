import QtQuick

// ScreenshotFlash (#166)
//
// A self-contained full-screen white vignette that plays a brief (~150 ms)
// fade-in / fade-out animation as visual feedback when a remote screenshot is
// taken via `GET /screenshot?flash=1`.
//
// Usage:
//   One instance is created per screen: it is the LAST child of each
//   per-screen PanelWindow inside `Variants` (so it renders on top of
//   everything else on that output).
//   Call `flash()` when the `inputManager.screenshotFlash` signal fires.
//
// The component is purely cosmetic and stateless: it neither reads nor writes
// any shared state.  It is safe to call `flash()` while a previous animation
// is still running — opacity is reset to 0.0 and the animation restarts from
// that baseline, so every flash looks identical regardless of timing.
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
                // Only hide when the animation truly finished and was not
                // immediately restarted by a rapid back-to-back flash() call.
                // Without this guard, restart() fires onStopped before the new
                // animation starts, clobbering visible on the next frame.
                if (!flashAnim.running) {
                    root.visible = false;
                }
            }
        }
    }

    // Public API: trigger a single flash cycle.
    // Calling flash() while a previous animation is still playing restarts
    // cleanly: opacity is reset to 0.0 before restart() so every flash starts
    // from the same baseline regardless of where the previous animation was.
    // stop() + visible=true happen synchronously before restart(), so onStopped
    // cannot fire between them and clobber visible.
    function flash() {
        flashAnim.stop();
        root.visible = true;
        vignette.opacity = 0.0;
        flashAnim.restart();
    }
}
