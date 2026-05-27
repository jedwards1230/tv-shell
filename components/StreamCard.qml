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
        property string _response: ""
        stdout: SplitParser {
            onRead: line => {
                sessionCheck._response += line;
            }
        }
        onExited: (exitCode, exitStatus) => {
            let response = sessionCheck._response;
            sessionCheck._response = "";
            if (exitCode !== 0 || response === "") {
                root.hasActiveSession = false;
                return;
            }
            try {
                let data = JSON.parse(response);
                root.hasActiveSession = (data.currentApp || "") !== "";
            } catch (e) {
                root.hasActiveSession = false;
            }
        }
    }

    Timer {
        interval: 15000
        running: root.shellState === "idle" && !!root.target.sunshineUser && !!root.target.sunshinePass
        repeat: true
        triggeredOnStart: true
        onTriggered: {
            if (sessionCheck.running)
                return;
            let host = root.target.host;
            let port = root.target.sunshinePort || "47990";
            let user = root.target.sunshineUser;
            let pass = root.target.sunshinePass;
            sessionCheck._response = "";
            sessionCheck.command = ["curl", "-sk", "--connect-timeout", "3", "--max-time", "5", "--user", user + ":" + pass, "https://" + host + ":" + port + "/api/currentClient"];
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
