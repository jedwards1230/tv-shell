import QtQuick
import Quickshell

// LaunchOverlay (#193)
//
// An opaque "Launching <app>…" surface shown from the moment a local app launch
// is initiated until its window is confirmed mapped + fullscreen. It exists
// because the main shell PanelWindow is `visible:false` / `transparent` during
// the launching/appRunning states (shell.qml), so during the ~2s window-detect
// gap whatever app was previously open bleeds through. This overlay is hosted in
// its own dedicated Overlay-layer PanelWindow (like ScreenshotFlash) so it
// renders on top even though the main shell surface is unmapped.
//
// Purely presentational: it reads nothing and owns no lifecycle state — shell.qml
// drives its visibility from the AppLifecycleManager launchStarted/windowConfirmed
// signals (with a safety timeout so it can never get stuck).
Item {
    id: root

    // Display strings supplied by the host.
    property string appName: ""
    // Freedesktop icon name (best-effort; falls back to the letter initial).
    property string appIcon: ""

    // Opaque backdrop — fully covers whatever is behind during the launch gap.
    Rectangle {
        anchors.fill: parent
        color: Theme.background
    }

    Column {
        anchors.centerIn: parent
        spacing: Units.spacingXL

        // App icon (matches AppCard's resolve-or-letter approach; no icon theme
        // on some targets, so the letter fallback is the common case).
        Item {
            width: Units.iconSizeXL
            height: Units.iconSizeXL
            anchors.horizontalCenter: parent.horizontalCenter

            Image {
                id: iconImage
                anchors.fill: parent
                // iconPath(name, true) → "" when not in the theme, so the letter
                // fallback shows instead of a magenta placeholder (#194 pattern).
                source: root.appIcon ? Quickshell.iconPath(root.appIcon, true) : ""
                sourceSize: Qt.size(Units.iconSizeXL, Units.iconSizeXL)
                fillMode: Image.PreserveAspectFit
                cache: false
                visible: status === Image.Ready && source != ""
            }
            Text {
                visible: !iconImage.visible
                anchors.fill: parent
                text: (root.appName || "?").charAt(0).toUpperCase()
                font.pixelSize: Units.iconSizeLG
                font.bold: true
                color: Theme.textSecondary
                horizontalAlignment: Text.AlignHCenter
                verticalAlignment: Text.AlignVCenter
            }
        }

        Text {
            anchors.horizontalCenter: parent.horizontalCenter
            text: root.appName ? "Launching " + root.appName + "…" : "Launching…"
            font.pixelSize: Theme.fontTitle
            color: Theme.textPrimary
            horizontalAlignment: Text.AlignHCenter
        }

        // Indeterminate spinner: a faint track ring with a single crimson dot
        // orbiting it. Only animates while the overlay is visible.
        Item {
            id: spinner
            width: Units.iconSizeMD
            height: Units.iconSizeMD
            anchors.horizontalCenter: parent.horizontalCenter

            Rectangle {
                anchors.fill: parent
                radius: width / 2
                color: "transparent"
                border.width: 3
                border.color: Theme.surfaceHover
            }

            Item {
                id: orbit
                anchors.fill: parent

                Rectangle {
                    width: spinner.width * 0.18
                    height: width
                    radius: width / 2
                    color: Theme.crimson
                    anchors.horizontalCenter: parent.horizontalCenter
                    y: -height / 2
                }

                RotationAnimator on rotation {
                    from: 0
                    to: 360
                    duration: 900
                    loops: Animation.Infinite
                    running: root.visible
                }
            }
        }
    }
}
