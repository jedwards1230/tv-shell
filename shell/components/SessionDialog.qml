import QtQuick
import QtQuick.Layouts
import "lib"

ModalOverlay {
    id: root

    property string runningApp: ""
    property string hostName: ""

    signal resumeRequested
    signal quitRequested
    signal cancelled

    z: 55

    // The base's scrim-click + B/Escape emit closed(); for this dialog that's a
    // cancel.
    onClosed: root.cancelled()

    onOpenedChanged: {
        if (opened)
            resumeBtn.forceActiveFocus();
    }

    Rectangle {
        anchors.centerIn: parent
        width: 900
        height: 380
        radius: Units.radiusXL
        color: Theme.surface

        ColumnLayout {
            anchors.centerIn: parent
            spacing: 40

            Text {
                text: "\"" + root.runningApp + "\" is running on " + root.hostName
                font.pixelSize: Theme.fontTitle
                font.bold: true
                color: Theme.textPrimary
                Layout.alignment: Qt.AlignHCenter
            }

            Text {
                text: "Resume the existing session, or quit and start fresh?"
                font.pixelSize: Theme.fontBody
                color: Theme.textSecondary
                Layout.alignment: Qt.AlignHCenter
            }

            RowLayout {
                Layout.alignment: Qt.AlignHCenter
                spacing: 32

                FocusButton {
                    id: resumeBtn
                    KeyNavigation.right: quitBtn
                    text: "Resume"
                    onActivated: root.resumeRequested()
                }

                FocusButton {
                    id: quitBtn
                    KeyNavigation.left: resumeBtn
                    KeyNavigation.right: cancelBtn
                    text: "Quit & Relaunch"
                    onActivated: root.quitRequested()
                }

                FocusButton {
                    id: cancelBtn
                    KeyNavigation.left: quitBtn
                    text: "Cancel"
                    onActivated: root.cancelled()
                    Keys.onEscapePressed: root.cancelled()
                }
            }
        }
    }
}
