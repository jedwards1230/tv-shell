import QtQuick
import QtQuick.Layouts
import "../"

// Padded surface card wrapping static content (text, columns, etc.).
// Used for read-only info displays: NetworkSettings IP card, Gateway/DNS card,
// AudioSettings format card — 3× duplication.
//
// Usage:
//   ReadonlyInfoCard {
//       Layout.fillWidth: true    // caller controls width
//       Text {
//           text: root.ipAddress
//           font.pixelSize: Theme.fontSmall
//           font.family: "monospace"
//           color: Theme.textPrimary
//           wrapMode: Text.Wrap
//       }
//   }
//
// Height auto-sizes to content + 48px vertical padding (24 each side).
Rectangle {
    id: card

    default property alias content: inner.data

    readonly property real _margin: 24

    Layout.fillWidth: true
    implicitHeight: inner.implicitHeight + _margin * 2
    radius: Units.radiusMD
    color: Theme.surface

    Item {
        id: inner
        anchors.fill: parent
        anchors.margins: card._margin
    }
}
