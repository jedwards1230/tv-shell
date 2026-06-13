import QtQuick
import QtQuick.Layouts

// Modal confirmation dialog shell. Full-screen scrim + centered surface card +
// Escape/click-outside dismissal. Inject body content (heading Text, button
// RowLayout) via the default property alias. Caller controls `opened` and
// handles button actions; `dismissed` fires on scrim-click and Escape.
//
// The caller is responsible for focusing the desired default button when
// `opened` becomes true (done by binding `focus:` on the safe cancel button,
// or via onOpenedChanged in the caller).
//
// z defaults to 55 so it paints above any page content. Override `z:` on the
// instance if a different stacking order is needed.
FocusScope {
    id: root

    property bool opened: false
    property real cardWidth: 800
    property real cardHeight: 350
    property real scrimOpacity: 0.7
    // Content column spacing — default matches the original `spacing: 32` literal
    // used in all migrated dialogs. Pass a different value to override.
    property real contentSpacing: 32

    signal dismissed

    default property alias content: contentColumn.data

    anchors.fill: parent
    visible: opened
    focus: opened
    z: 55

    onOpenedChanged: {
        if (opened)
            root.forceActiveFocus();
    }

    Keys.onEscapePressed: root.dismissed()

    Rectangle {
        anchors.fill: parent
        color: Qt.rgba(0, 0, 0, root.scrimOpacity)

        MouseArea {
            anchors.fill: parent
            onClicked: root.dismissed()
        }
    }

    Rectangle {
        anchors.centerIn: parent
        width: root.cardWidth
        height: root.cardHeight
        radius: Units.radiusXL
        color: Theme.surface

        ColumnLayout {
            id: contentColumn
            anchors.centerIn: parent
            spacing: root.contentSpacing
        }
    }
}
