import QtQuick
import Quickshell.Io

BaseCard {
    id: root

    required property var target
    property string appName: ""
    property bool isOnline: false
    property string shellState: "idle"
    property bool hasActiveSession: false

    label: root.appName !== "" ? root.appName : (root.target.name || "Unknown")

    Process {
        id: pingCheck
        command: ["ping", "-c1", "-W1", root.target.host || "127.0.0.1"]
        onExited: (exitCode, exitStatus) => {
            root.isOnline = (exitCode === 0);
        }
    }

    Timer {
        interval: 10000
        running: true
        repeat: true
        triggeredOnStart: true
        onTriggered: {
            if (!pingCheck.running)
                pingCheck.running = true;
        }
    }

    Process {
        id: sessionCheck
        command: ["pgrep", "-f", "moonlight stream"]
        onExited: (exitCode, exitStatus) => {
            root.hasActiveSession = (exitCode === 0);
        }
    }

    Timer {
        interval: 5000
        running: root.shellState === "idle"
        repeat: true
        triggeredOnStart: true
        onTriggered: {
            if (!sessionCheck.running)
                sessionCheck.running = true;
        }
    }

    Text {
        anchors.fill: parent
        text: root.appName !== "" ? root.appName.charAt(0).toUpperCase() : "\u{1F3AE}"
        font.pixelSize: 96
        font.bold: root.appName !== ""
        color: root.appName !== "" ? Theme.textSecondary : Theme.textPrimary
        horizontalAlignment: Text.AlignHCenter
        verticalAlignment: Text.AlignVCenter
    }

    Rectangle {
        visible: root.hasActiveSession
        width: 16
        height: 16
        radius: 8
        color: Theme.online
        anchors.top: parent.top
        anchors.right: parent.right
        anchors.topMargin: 8
        anchors.rightMargin: 8
    }
}
