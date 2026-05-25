import QtQuick
import QtQuick.Layouts
import Quickshell.Io

Item {
    id: root
    width: Theme.cardWidth
    height: Theme.cardHeight

    required property var target
    property bool isOnline: false

    signal activated()

    Process {
        id: pingCheck
        command: ["ping", "-c1", "-W1", root.target.host]
        onExited: (exitCode, exitStatus) => { root.isOnline = (exitCode === 0) }
    }

    Timer {
        interval: 10000
        running: true
        repeat: true
        triggeredOnStart: true
        onTriggered: {
            if (!pingCheck.running) pingCheck.running = true
        }
    }

    Rectangle {
        anchors.fill: parent
        radius: Theme.cardRadius
        color: root.activeFocus ? Theme.surfaceHover : Theme.surface
        border.width: root.activeFocus ? 3 : 0
        border.color: Theme.accent

        Behavior on border.width { NumberAnimation { duration: 150 } }
        Behavior on color { ColorAnimation { duration: 150 } }

        ColumnLayout {
            anchors.fill: parent
            anchors.margins: Theme.padding
            spacing: 8

            RowLayout {
                Layout.fillWidth: true
                spacing: 8

                Rectangle {
                    width: 12; height: 12; radius: 6
                    color: root.isOnline ? Theme.online : Theme.offline
                }

                Text {
                    text: root.target.name
                    font.pixelSize: Theme.fontTitle
                    font.bold: true
                    color: Theme.text
                    elide: Text.ElideRight
                    Layout.fillWidth: true
                }
            }

            Text {
                text: root.target.app
                font.pixelSize: Theme.fontBody
                color: Theme.textDim
            }

            Item { Layout.fillHeight: true }

            Text {
                text: {
                    let parts = []
                    if (root.target.resolution) parts.push(root.target.resolution)
                    if (root.target.fps) parts.push(root.target.fps + " fps")
                    if (root.target.hdr) parts.push("HDR")
                    return parts.join(" · ")
                }
                font.pixelSize: Theme.fontSmall
                color: Theme.textDim
            }
        }
    }

    Keys.onReturnPressed: root.activated()
    Keys.onEnterPressed: root.activated()
}
