import QtQuick

Rectangle {
    id: root
    width: 120; height: 48
    radius: 8
    color: root.activeFocus ? Theme.accent : Theme.surface
    border.width: root.activeFocus ? 0 : 1
    border.color: Theme.textDim

    property alias text: label.text

    Behavior on color { ColorAnimation { duration: 150 } }

    Text {
        id: label
        anchors.centerIn: parent
        font.pixelSize: Theme.fontBody
        color: Theme.text
    }
}
