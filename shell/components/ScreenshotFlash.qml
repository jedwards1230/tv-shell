import QtQuick
import Qt5Compat.GraphicalEffects

// ScreenshotFlash (#166)
//
// A self-contained edge vignette that plays a brief (~200 ms) fade-in /
// fade-out animation as visual feedback when a remote screenshot is taken via
// `GET /screenshot?flash=1`.
//
// Unlike a full-screen white flash, this lights up only the screen *edges*
// (transparent center → white border), so gameplay/content in the middle stays
// visible while the capture is clearly acknowledged.
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

    // Radial gradient vignette: fully transparent through the center, ramping
    // to white only near the edges. Default radii (width/2, height/2) place the
    // gradient's outer stop at every mid-edge simultaneously, so the glow is
    // even all the way around; the corners (beyond the radius) clamp to the
    // final white stop, giving a clean frame.
    RadialGradient {
        id: vignette
        anchors.fill: parent
        opacity: 0.0

        gradient: Gradient {
            GradientStop {
                position: 0.0
                color: "transparent"
            }
            GradientStop {
                position: 0.35
                color: "transparent"
            }
            GradientStop {
                position: 0.62
                color: Qt.rgba(1, 1, 1, 0.30)
            }
            GradientStop {
                position: 0.85
                color: Qt.rgba(1, 1, 1, 0.70)
            }
            GradientStop {
                position: 1.0
                color: Qt.rgba(1, 1, 1, 0.97)
            }
        }

        SequentialAnimation {
            id: flashAnim
            running: false

            // Fade in quickly (≈ 55 ms).
            NumberAnimation {
                target: vignette
                property: "opacity"
                to: 1.0
                duration: 55
                easing.type: Easing.OutQuad
            }
            // Hold briefly.
            PauseAnimation {
                duration: 40
            }
            // Fade out over the remainder of the ~200 ms window.
            NumberAnimation {
                target: vignette
                property: "opacity"
                to: 0.0
                duration: 105
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
