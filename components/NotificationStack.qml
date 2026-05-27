import QtQuick

Item {
    id: root

    width: Units.gridUnit * 14
    height: parent.height

    Column {
        anchors.right: parent.right
        spacing: Units.spacingSM

        Repeater {
            model: NotificationManager.activeList

            delegate: NotificationToast {
                required property var modelData
                notification: modelData
                onDismissed: id => NotificationManager.dismiss(id)
            }
        }
    }
}
