import QtQuick
import QtQuick.Layouts
import Quickshell.Io

Item {
    id: root
    width: Theme.cardWidth
    height: Theme.cardHeight

    required property var target
    property string appName: ""  // When set, card shows app name (app-view mode)
    property bool isOnline: false
    property bool isFocused: (activeFocus && !Theme.mouseMode) || (mouseArea.containsMouse && Theme.mouseMode)

    signal activated()

    Connections {
        target: Theme
        function onMouseModeChanged() {
            if (!Theme.mouseMode && mouseArea.containsMouse) {
                if (root.ListView.view)
                    root.ListView.view.currentIndex = root.ListView.view.indexAt(root.x, root.y)
                root.forceActiveFocus()
            }
        }
    }

    Process {
        id: pingCheck
        command: ["ping", "-c1", "-W1", root.target.host || "127.0.0.1"]
        onExited: (exitCode, exitStatus) => { root.isOnline = (exitCode === 0) }
    }

    Timer {
        interval: 10000
        running: true
        repeat: true
        triggeredOnStart: true
        onTriggered: { if (!pingCheck.running) pingCheck.running = true }
    }

    transform: [
        Scale {
            origin.x: root.width / 2
            origin.y: root.height / 2
            xScale: root.isFocused ? 1.05 : 1.0
            yScale: root.isFocused ? 1.05 : 1.0
            Behavior on xScale { NumberAnimation { duration: 250; easing.type: Easing.OutCubic } }
            Behavior on yScale { NumberAnimation { duration: 250; easing.type: Easing.OutCubic } }
        }
    ]

    z: root.isFocused ? 10 : 0

    Rectangle {
        id: card
        anchors.fill: parent
        radius: Theme.cardRadius
        color: Theme.cardBackground
        border.width: root.isFocused ? 6 : 2
        border.color: root.isFocused ? Theme.focusBorder : Theme.surfaceBorder

        Behavior on border.width { NumberAnimation { duration: 200 } }
        Behavior on border.color { ColorAnimation { duration: 200 } }

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
            anchors.margins: Theme.padding / 2
            spacing: 8

            Item { Layout.fillHeight: true }

            // Gamepad icon (server view) or app initial (app view)
            Text {
                text: root.appName !== "" ? root.appName.charAt(0).toUpperCase() : "\u{1F3AE}"
                font.pixelSize: 120
                font.bold: root.appName !== ""
                color: root.appName !== "" ? Theme.textSecondary : Theme.textPrimary
                Layout.alignment: Qt.AlignHCenter
            }

            // Online/offline dot
            Rectangle {
                width: 16; height: 16; radius: 8
                color: root.isOnline ? Theme.online : Theme.offline
                Layout.alignment: Qt.AlignHCenter
            }

            Item { Layout.fillHeight: true }

            // Primary label: app name (app view) or server name (server view)
            Text {
                text: root.appName !== "" ? root.appName : (root.target.name || "Unknown")
                font.pixelSize: Theme.fontSmall
                font.bold: true
                color: Theme.textPrimary
                elide: Text.ElideRight
                Layout.fillWidth: true
                horizontalAlignment: Text.AlignHCenter
            }

            // Subtitle: server name (app view) or resolution/fps (server view)
            Text {
                text: {
                    if (root.appName !== "") {
                        return root.target.name || ""
                    }
                    let parts = []
                    if (root.target.resolution) parts.push(root.target.resolution)
                    if (root.target.fps) parts.push(root.target.fps + " fps")
                    if (root.target.hdr) parts.push("HDR")
                    return parts.join(" · ")
                }
                font.pixelSize: Theme.fontXSmall
                color: Theme.textSecondary
                Layout.fillWidth: true
                horizontalAlignment: Text.AlignHCenter
                visible: text !== ""
            }
        }
    }

    Keys.onReturnPressed: root.activated()
    Keys.onEnterPressed: root.activated()
}
