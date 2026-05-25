import QtQuick
import QtQuick.Layouts

Item {
    id: root
    width: Theme.cardWidth
    height: parent ? parent.height - 20 : Theme.cardHeight

    required property var app
    property bool isFocused: activeFocus || mouseArea.containsMouse

    signal activated()

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
            origin.x: root.width / 2; origin.y: root.height / 2
            axis { x: 0; y: 1; z: 0 }
            angle: root.isFocused ? root.tiltX : 0
            Behavior on angle { NumberAnimation { duration: 200; easing.type: Easing.OutCubic } }
        },
        Rotation {
            origin.x: root.width / 2; origin.y: root.height / 2
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
        color: Theme.surface
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
            anchors.margins: Theme.padding
            spacing: 12

            // Marquee name — scrolls when focused and text overflows
            MarqueeText {
                Layout.fillWidth: true
                Layout.preferredHeight: Theme.fontTitle * 1.3
                animate: root.isFocused
                text: root.app.name || "Unknown"
                font.pixelSize: Theme.fontTitle
                font.bold: true
                color: Theme.textPrimary
            }

            Text {
                visible: (root.app.comment || "") !== ""
                text: root.app.comment || ""
                font.pixelSize: Theme.fontSmall
                color: Theme.textSecondary
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
                color: Theme.cardAccent
                opacity: 0.4
            }
        }
    }

    Keys.onReturnPressed: root.activated()
    Keys.onEnterPressed: root.activated()
}
