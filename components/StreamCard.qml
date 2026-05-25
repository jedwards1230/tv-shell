import QtQuick
import QtQuick.Layouts
import Quickshell.Io

Item {
    id: root
    width: Theme.cardWidth
    height: parent ? parent.height - 20 : Theme.cardHeight

    required property var target
    property bool isOnline: false
    property bool isFocused: activeFocus || mouseArea.containsMouse

    signal activated()

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

    // Navigation direction tracking for parallax tilt
    property real tiltX: 0
    property real tiltY: 0

    Keys.onLeftPressed: (event) => { tiltX = -4; tiltResetTimer.restart(); event.accepted = false }
    Keys.onRightPressed: (event) => { tiltX = 4; tiltResetTimer.restart(); event.accepted = false }
    Keys.onUpPressed: (event) => { tiltY = -3; tiltResetTimer.restart(); event.accepted = false }
    Keys.onDownPressed: (event) => { tiltY = 3; tiltResetTimer.restart(); event.accepted = false }

    Timer {
        id: tiltResetTimer
        interval: 300
        onTriggered: { root.tiltX = 0; root.tiltY = 0 }
    }

    transform: [
        Scale {
            origin.x: root.width / 2
            origin.y: root.height / 2
            xScale: root.isFocused ? 1.08 : 1.0
            yScale: root.isFocused ? 1.08 : 1.0

            Behavior on xScale { NumberAnimation { duration: 250; easing.type: Easing.OutCubic } }
            Behavior on yScale { NumberAnimation { duration: 250; easing.type: Easing.OutCubic } }
        },
        Rotation {
            origin.x: root.width / 2
            origin.y: root.height / 2
            axis { x: 0; y: 1; z: 0 }
            angle: root.isFocused ? root.tiltX : 0

            Behavior on angle { NumberAnimation { duration: 200; easing.type: Easing.OutCubic } }
        },
        Rotation {
            origin.x: root.width / 2
            origin.y: root.height / 2
            axis { x: 1; y: 0; z: 0 }
            angle: root.isFocused ? root.tiltY : 0

            Behavior on angle { NumberAnimation { duration: 200; easing.type: Easing.OutCubic } }
        }
    ]

    z: root.isFocused ? 10 : 0

    Rectangle {
        id: card
        anchors.fill: parent
        radius: Theme.cardRadius
        color: root.isFocused ? Theme.surface : Theme.surface
        border.width: root.isFocused ? 6 : 2
        border.color: root.isFocused ? Theme.focusBorder : Theme.surfaceBorder

        Behavior on border.width { NumberAnimation { duration: 200 } }

        // Drop shadow
        layer.enabled: root.isFocused
        layer.effect: Item {
            Rectangle {
                anchors.fill: parent
                anchors.margins: -8
                radius: Theme.cardRadius + 8
                color: "#00000033"
                z: -1
            }
        }

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
            spacing: 16

            RowLayout {
                Layout.fillWidth: true
                spacing: 16

                Rectangle {
                    width: 24; height: 24; radius: 12
                    color: root.isOnline ? Theme.online : Theme.offline
                }

                Text {
                    text: root.target.name || "Unknown"
                    font.pixelSize: Theme.fontTitle
                    font.bold: true
                    color: Theme.textPrimary
                    elide: Text.ElideRight
                    Layout.fillWidth: true
                }
            }

            Text {
                text: root.target.app || ""
                font.pixelSize: Theme.fontBody
                color: Theme.textSecondary
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
                color: Theme.textSecondary
            }
        }
    }

    Keys.onReturnPressed: root.activated()
    Keys.onEnterPressed: root.activated()
}
