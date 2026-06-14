import QtQuick
import "../"

// Selectable bordered list row used in settings list delegates.
// Bluetooth (paired + available) and Network (wifi) all share this
// outer Rectangle+ColorAnimation pattern.
//
// Usage (inside a ListView delegate):
//   SettingsListRow {
//       width: listView.width
//       itemHeight: 96
//       selected: listView.currentIndex === index && listView.activeFocus
//       borderColor: modelData.connected ? Theme.online : Theme.surfaceBorder
//
//       RowLayout { ... }   // delegate content via default alias
//   }
//
// The caller provides `width`, `itemHeight`, `borderColor`, and `selected`.
// MouseArea and KeyNavigation stay at the call site (delegate-specific logic).
Rectangle {
    id: row

    property int itemHeight: 96
    property bool selected: false
    // Dynamic border color — caller sets this per row state.
    property color borderColor: Theme.surfaceBorder

    default property alias content: inner.data

    height: itemHeight
    radius: Units.radiusMD
    color: row.selected ? Theme.surfaceHover : Theme.surface
    border.width: 2
    border.color: row.borderColor

    Behavior on color {
        ColorAnimation {
            duration: 150
        }
    }

    Item {
        id: inner
        anchors.fill: parent
    }
}
