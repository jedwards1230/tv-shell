import QtQuick
import QtQuick.Layouts
import Quickshell.Io

Rectangle {
    id: root
    color: Theme.background
    visible: false

    signal closed()

    Process { id: powerOff; command: ["systemctl", "poweroff"] }
    Process { id: rebootCmd; command: ["systemctl", "reboot"] }
    Process { id: volumeUp; command: ["wpctl", "set-volume", "@DEFAULT_AUDIO_SINK@", "5%+"] }
    Process { id: volumeDown; command: ["wpctl", "set-volume", "@DEFAULT_AUDIO_SINK@", "5%-"] }
    Process { id: volumeMute; command: ["wpctl", "set-mute", "@DEFAULT_AUDIO_SINK@", "toggle"] }

    ColumnLayout {
        anchors.centerIn: parent
        spacing: 24
        width: 400

        Text {
            text: "Settings"
            font.pixelSize: Theme.fontTitle
            font.bold: true
            color: Theme.text
            Layout.alignment: Qt.AlignHCenter
        }

        Text {
            text: "Audio"
            font.pixelSize: Theme.fontBody
            color: Theme.accent
        }

        RowLayout {
            Layout.fillWidth: true
            spacing: 12

            SettingsButton {
                id: volDownBtn
                text: "Vol -"
                focus: true
                KeyNavigation.right: volUpBtn
                KeyNavigation.down: restartBtn
                Keys.onReturnPressed: { volumeDown.running = true }
            }

            SettingsButton {
                id: volUpBtn
                text: "Vol +"
                KeyNavigation.right: muteBtn
                KeyNavigation.down: restartBtn
                Keys.onReturnPressed: { volumeUp.running = true }
            }

            SettingsButton {
                id: muteBtn
                text: "Mute"
                KeyNavigation.down: shutdownBtn
                Keys.onReturnPressed: { volumeMute.running = true }
            }
        }

        Text {
            text: "Power"
            font.pixelSize: Theme.fontBody
            color: Theme.accent
        }

        RowLayout {
            Layout.fillWidth: true
            spacing: 12

            SettingsButton {
                id: restartBtn
                text: "Restart"
                KeyNavigation.right: shutdownBtn
                KeyNavigation.up: volDownBtn
                Keys.onReturnPressed: { rebootCmd.running = true }
            }

            SettingsButton {
                id: shutdownBtn
                text: "Shutdown"
                KeyNavigation.up: muteBtn
                Keys.onReturnPressed: { powerOff.running = true }
            }
        }

        Item { height: 24 }

        Text {
            text: "Press B to go back"
            font.pixelSize: Theme.fontSmall
            color: Theme.textDim
            Layout.alignment: Qt.AlignHCenter
        }
    }

    Keys.onEscapePressed: root.closed()
}
