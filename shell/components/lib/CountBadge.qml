import QtQuick
import "../"

// Small circular count badge (unread notifications, etc.). Anchor it to the
// top-right of the host glyph. Hidden when count <= 0.
Rectangle {
    id: badge
    property int count: 0
    property color badgeColor: Theme.crimson
    // Cap glyph diameter; the original used 20 in QuickActions. Callers that need
    // a gridUnit-relative size can override width/height.
    property int diameter: 20

    visible: badge.count > 0
    width: diameter
    height: diameter
    radius: diameter / 2
    color: badge.badgeColor

    Text {
        anchors.centerIn: parent
        text: badge.count > 9 ? "9+" : badge.count.toString()
        font.pixelSize: Math.round(badge.diameter * 0.55)
        font.bold: true
        color: Theme.textOnDark
    }
}
