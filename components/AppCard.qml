import QtQuick
import QtQuick.Layouts

Item {
    id: root
    width: Theme.cardWidth
    height: parent ? parent.height - 20 : Theme.cardHeight

    required property var app
    signal activated()

    Rectangle {
        anchors.fill: parent
        radius: Theme.cardRadius
        color: root.activeFocus || mouseArea.containsMouse ? Theme.surfaceHover : Theme.surface
        border.width: root.activeFocus ? 6 : 2
        border.color: root.activeFocus ? Theme.accent : Theme.surfaceHover

        Behavior on border.width { NumberAnimation { duration: 150 } }
        Behavior on color { ColorAnimation { duration: 150 } }

        MouseArea {
            id: mouseArea
            anchors.fill: parent
            hoverEnabled: true
            cursorShape: Qt.PointingHandCursor
            onClicked: {
                root.forceActiveFocus()
                root.activated()
            }
        }

        ColumnLayout {
            anchors.fill: parent
            anchors.margins: Theme.padding
            spacing: 12

            Text {
                text: root.app.name || "Unknown"
                font.pixelSize: Theme.fontTitle
                font.bold: true
                color: Theme.text
                elide: Text.ElideRight
                Layout.fillWidth: true
            }

            Text {
                visible: (root.app.comment || "") !== ""
                text: root.app.comment || ""
                font.pixelSize: Theme.fontSmall
                color: Theme.textDim
                elide: Text.ElideRight
                Layout.fillWidth: true
                maximumLineCount: 2
                wrapMode: Text.WordWrap
            }

            Item { Layout.fillHeight: true }

            Rectangle {
                Layout.preferredHeight: 6
                Layout.preferredWidth: 80
                radius: 3
                color: Theme.accentGold
                opacity: 0.5
            }
        }
    }

    Keys.onReturnPressed: root.activated()
    Keys.onEnterPressed: root.activated()
}
