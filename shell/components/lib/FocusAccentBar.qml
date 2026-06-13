import QtQuick
import "../"

// Left-edge accent bar marking a focused/selected list row (settings sidebar,
// nav drawer). Anchor it to the row's left edge; it self-sizes vertically.
Rectangle {
    id: bar
    property bool active: false
    anchors.left: parent.left
    anchors.verticalCenter: parent.verticalCenter
    width: 4
    height: parent ? parent.height - 16 : 0
    radius: 2
    color: Theme.focusBorder
    visible: bar.active
}
