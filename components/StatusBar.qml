import QtQuick
import QtQuick.Layouts
import Quickshell.Io

Rectangle {
    id: root
    height: Theme.statusBarHeight
    color: Theme.surface

    property string shellState: "idle"

    Process {
        id: ipProcess
        command: ["hostname", "-I"]
        stdout: SplitParser {
            onRead: (line) => { ipText.text = line.trim().split(" ")[0] || "No IP" }
        }
    }

    Timer {
        interval: 30000
        running: true
        repeat: true
        triggeredOnStart: true
        onTriggered: { if (!ipProcess.running) ipProcess.running = true }
    }

    RowLayout {
        anchors.fill: parent
        anchors.leftMargin: Theme.padding
        anchors.rightMargin: Theme.padding
        spacing: 16

        Text {
            text: "Game Shell"
            font.pixelSize: Theme.fontStatus
            font.bold: true
            color: Theme.accent
        }

        Rectangle { width: 1; height: 20; color: Theme.textDim; opacity: 0.3 }

        Text {
            text: {
                switch (root.shellState) {
                    case "idle": return "Ready"
                    case "launching": return "Launching..."
                    case "streaming": return "Streaming"
                    case "reconnecting": return "Reconnecting..."
                    default: return root.shellState
                }
            }
            font.pixelSize: Theme.fontSmall
            color: root.shellState === "streaming" ? Theme.online :
                   root.shellState === "reconnecting" ? Theme.warning : Theme.textDim
        }

        Item { Layout.fillWidth: true }

        Text {
            id: ipText
            text: "..."
            font.pixelSize: Theme.fontSmall
            color: Theme.textDim
        }

        Rectangle { width: 1; height: 20; color: Theme.textDim; opacity: 0.3 }

        Text {
            id: clockText
            font.pixelSize: Theme.fontStatus
            color: Theme.text

            Timer {
                interval: 1000
                running: true
                repeat: true
                triggeredOnStart: true
                onTriggered: {
                    let now = new Date()
                    clockText.text = now.toLocaleTimeString(Qt.locale(), "h:mm AP")
                }
            }
        }
    }
}
