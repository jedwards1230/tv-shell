import QtQuick

Rectangle {
    id: root
    width: 240; height: 96
    radius: 16
    color: root.activeFocus || mouseArea.containsMouse ? Theme.crimson : Theme.surface
    border.width: root.activeFocus ? 0 : 2
    border.color: Theme.surfaceHover

    property alias text: label.text

    Behavior on color { ColorAnimation { duration: 150 } }

    MouseArea {
        id: mouseArea
        anchors.fill: parent
        hoverEnabled: true
        cursorShape: Qt.PointingHandCursor
        onClicked: {
            root.forceActiveFocus()
            root.Keys.returnPressed(null)
        }
    }

    Text {
        id: label
        anchors.centerIn: parent
        font.pixelSize: Theme.fontBody
        color: root.activeFocus || mouseArea.containsMouse ? "#ffffff" : Theme.textPrimary
    }
}
