import QtQuick
import QtQuick.Layouts
import "lib"

FocusScope {
    id: root

    property string runningApp: ""
    property string hostName: ""
    property bool opened: false

    signal resumeRequested
    signal quitRequested
    signal cancelled

    anchors.fill: parent
    visible: opened
    focus: opened
    z: 55

    onOpenedChanged: {
        if (opened)
            resumeBtn.forceActiveFocus();
    }

    Rectangle {
        anchors.fill: parent
        color: Qt.rgba(0, 0, 0, 0.7)

        MouseArea {
            anchors.fill: parent
            onClicked: root.cancelled()
        }
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

    Keys.onEscapePressed: root.cancelled()
}
