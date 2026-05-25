import QtQuick
import QtQuick.Layouts
import Quickshell.Io

Rectangle {
    id: root
    height: Theme.statusBarHeight
    color: Theme.primary

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
        spacing: 32

        Text {
            text: "Game Shell"
            font.pixelSize: Theme.fontStatus
            font.bold: true
            color: "#ffffff"
        }

        Rectangle { width: 2; height: 40; color: "#ffffff"; opacity: 0.3 }

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
            color: root.shellState === "streaming" ? Theme.accentGold :
                   root.shellState === "reconnecting" ? Theme.accentOrange : "#ffffffcc"
        }

        Item { Layout.fillWidth: true }

        Text {
            id: ipText
            text: "..."
            font.pixelSize: Theme.fontSmall
            color: "#ffffffcc"
        }

        Rectangle { width: 2; height: 40; color: "#ffffff"; opacity: 0.3 }

        Text {
            id: clockText
            font.pixelSize: Theme.fontStatus
            color: "#ffffff"

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
