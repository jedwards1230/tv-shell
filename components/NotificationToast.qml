import QtQuick

Item {
    id: root

    required property var notification

    width: Units.gridUnit * 14
    height: contentRow.height + Units.spacingMD * 2

    signal dismissed(int id)

    // Slide in from right
    x: Units.gridUnit * 2
    opacity: 0

    Component.onCompleted: {
        enterAnim.start()
        if (notification.duration > 0)
            autoDismissTimer.start()
    }

    NumberAnimation {
        id: enterAnim
        target: root
        property: "x"
        from: Units.gridUnit * 2
        to: 0
        duration: 400
        easing.type: Easing.OutCubic
    }

    NumberAnimation {
        id: enterOpacity
        target: root
        property: "opacity"
        from: 0
        to: 1.0
        duration: 300
        easing.type: Easing.OutCubic
        running: enterAnim.running
    }

    SequentialAnimation {
        id: exitAnim

        NumberAnimation {
            target: root
            property: "opacity"
            to: 0
            duration: 300
            easing.type: Easing.InCubic
        }

        ScriptAction {
            script: root.dismissed(root.notification.id)
        }
    }

    Timer {
        id: autoDismissTimer
        interval: root.notification.duration
        onTriggered: exitAnim.start()
    }

    // Background
    Rectangle {
        anchors.fill: parent
        radius: Units.radiusMD
        color: Theme.surface
        border.width: Units.borderThin
        border.color: Theme.surfaceBorder

        // Left accent bar
        Rectangle {
            anchors.left: parent.left
            anchors.top: parent.top
            anchors.bottom: parent.bottom
            width: 4
            radius: Units.radiusMD
            color: {
                if (root.notification.level === "error") return Theme.crimson
                if (root.notification.level === "warning") return Theme.ember
                return Theme.navy
            }

            // Clip right side of the accent bar radius so it sits flush
            Rectangle {
                anchors.right: parent.right
                anchors.top: parent.top
                anchors.bottom: parent.bottom
                width: parent.radius
                color: parent.color
            }
        }
    }

    Row {
        id: contentRow
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.verticalCenter: parent.verticalCenter
        anchors.leftMargin: Units.spacingMD + 4  // Clear the accent bar
        anchors.rightMargin: Units.spacingMD
        spacing: Units.spacingSM

        // Icon
        Text {
            visible: root.notification.icon !== ""
            text: root.notification.icon
            font.pixelSize: Theme.fontTitle
            anchors.verticalCenter: parent.verticalCenter
        }

        // Text column
        Column {
            width: contentRow.width - (root.notification.icon !== "" ? Theme.fontTitle + Units.spacingSM : 0) - Units.spacingMD - 4 - Units.spacingMD
            spacing: Units.spacingXS

            Text {
                width: parent.width
                text: root.notification.title
                font.pixelSize: Theme.fontBody
                font.bold: true
                color: Theme.textPrimary
                elide: Text.ElideRight
            }

            Text {
                visible: root.notification.message !== ""
                width: parent.width
                text: root.notification.message
                font.pixelSize: Theme.fontSmall
                color: Theme.textSecondary
                elide: Text.ElideRight
            }
        }
    }
}
