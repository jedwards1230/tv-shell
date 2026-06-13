import QtQuick
import QtQuick.Layouts

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

                FocusScope {
                    id: resumeBtn
                    width: resumeBtnInner.width
                    height: resumeBtnInner.height
                    KeyNavigation.right: quitBtn

                    SettingsButton {
                        id: resumeBtnInner
                        text: "Resume"
                        focus: parent.activeFocus
                        onActivated: root.resumeRequested()
                        MouseArea {
                            anchors.fill: parent
                            hoverEnabled: true
                            cursorShape: Qt.PointingHandCursor
                            onClicked: resumeBtnInner.activated()
                        }
                    }
                }

                FocusScope {
                    id: quitBtn
                    width: quitBtnInner.width
                    height: quitBtnInner.height
                    KeyNavigation.left: resumeBtn
                    KeyNavigation.right: cancelBtn

                    SettingsButton {
                        id: quitBtnInner
                        text: "Quit & Relaunch"
                        focus: parent.activeFocus
                        onActivated: root.quitRequested()
                        MouseArea {
                            anchors.fill: parent
                            hoverEnabled: true
                            cursorShape: Qt.PointingHandCursor
                            onClicked: quitBtnInner.activated()
                        }
                    }
                }

                FocusScope {
                    id: cancelBtn
                    width: cancelBtnInner.width
                    height: cancelBtnInner.height
                    KeyNavigation.left: quitBtn

                    SettingsButton {
                        id: cancelBtnInner
                        text: "Cancel"
                        focus: parent.activeFocus
                        onActivated: root.cancelled()
                        MouseArea {
                            anchors.fill: parent
                            hoverEnabled: true
                            cursorShape: Qt.PointingHandCursor
                            onClicked: cancelBtnInner.activated()
                        }
                    }
                    Keys.onEscapePressed: root.cancelled()
                }
            }
        }
    }

    Keys.onEscapePressed: root.cancelled()
}
