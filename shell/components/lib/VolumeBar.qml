import QtQuick
import "../"

// Reusable volume track + fill + label.
// Duplicated 3× in VolumeOverlay, SessionQAM, AudioSettings — unified here.
//
// Usage:
//   VolumeBar {
//       volume: audioCtl.volume           // 0-100
//       muted: audioCtl.muted
//       trackHeight: Units.gridUnit * 1.5
//       showFocusBorder: root.activeFocus && root._focusRow === 0
//       // optional font/font overrides kept as Layout.fillWidth on the caller
//   }
//
// The caller is responsible for setting width (e.g. Layout.fillWidth: true).
// trackHeight defaults to Units.gridUnit * 1.5 matching VolumeOverlay.
Rectangle {
    id: root

    property int volume: 0
    property bool muted: false
    property real trackHeight: Units.gridUnit * 1.5
    property bool showFocusBorder: false
    // Font size for the centered label — callers that need a larger label
    // (AudioSettings uses fontBody) can override.
    property int labelPixelSize: Theme.fontHint

    height: trackHeight
    radius: height / 2
    color: Theme.surfaceHover
    border.width: root.showFocusBorder ? Units.borderMedium : 0
    border.color: Theme.focusBorder

    Rectangle {
        id: fill
        width: parent.width * (root.volume / 100)
        height: parent.height
        radius: parent.radius
        color: root.muted ? Theme.textSecondary : (Theme.darkMode ? Theme.ember : Theme.navy)

        Behavior on width {
            NumberAnimation {
                duration: 80
            }
        }
    }

    Text {
        anchors.centerIn: parent
        text: root.muted ? "MUTED" : root.volume + "%"
        font.pixelSize: root.labelPixelSize
        font.bold: true
        color: root.volume > 40 && !root.muted ? Theme.textOnDark : Theme.textPrimary
    }
}
