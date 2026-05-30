import QtQuick
import Quickshell.Io

BaseCard {
    id: root

    required property var target
    property string appName: ""
    property bool isOnline: false
    property string shellState: "idle"
    property bool hasActiveSession: false
    property string activeAppName: ""

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

    // Resolve the running app name via the daemon's `sunshine-status` IPC command
    // (see docs/IPC_PROTOCOL.md) + the local Moonlight config. The daemon owns the
    // Sunshine /serverinfo HTTPS fetch and returns {online,paired,currentApp,...},
    // where currentApp is the busy game id (or "" when idle). This replaced the
    // inline urllib /serverinfo poll; the game-id -> friendly-name resolution
    // against Moonlight.conf stays local, preserving the prior output contract
    // ("IDLE" / resolved name / "Unknown App").
    Process {
        id: sessionCheck
        property string _response: ""
        stdout: SplitParser {
            onRead: line => {
                sessionCheck._response = line.trim();
            }
        }
        onExited: (exitCode, exitStatus) => {
            let name = sessionCheck._response;
            sessionCheck._response = "";
            if (exitCode !== 0 || name === "" || name === "IDLE") {
                root.hasActiveSession = false;
                root.activeAppName = "";
            } else {
                root.hasActiveSession = true;
                root.activeAppName = name;
            }
        }
    }

    Timer {
        interval: 10000
        running: root.shellState === "idle" && root.isOnline
        repeat: true
        triggeredOnStart: true
        onTriggered: {
            if (sessionCheck.running)
                return;
            let host = root.target.host || "127.0.0.1";
            let port = root.target.sunshinePort || "47990";
            sessionCheck._response = "";
            // Ask the daemon for the Sunshine session state (sunshine-status reply
            // is one compact JSON line; read until the first newline since the
            // daemon holds the connection open). Then resolve the busy game id to a
            // friendly name from the local Moonlight.conf, exactly as before.
            sessionCheck.command = ["python3", "-c", `
import socket, os, json, configparser, sys
try:
    sk = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    sk.settimeout(10)
    sk.connect(os.environ.get('GAME_SHELL_SOCK', '/run/user/' + str(os.getuid()) + '/game-shell-input.sock'))
    sk.sendall(('sunshine-status ' + sys.argv[1] + ' ' + sys.argv[2] + '\\n').encode())
    buf = b''
    while b'\\n' not in buf:
        c = sk.recv(65536)
        if not c:
            break
        buf += c
    sk.close()
    resp = buf.split(b'\\n', 1)[0].decode()
    data = json.loads(resp)
    game_id = str(data.get("currentApp", "") or "")
    if game_id == "" or game_id == "0":
        print("IDLE")
        sys.exit(0)
    conf = os.path.expanduser("~/.config/Moonlight Game Streaming Project/Moonlight.conf")
    if os.path.exists(conf):
        cp = configparser.ConfigParser()
        cp.read(conf)
        for k, v in cp.items("hosts"):
            if k.endswith("\\\\id") and v == game_id:
                name_key = k.replace("\\\\id", "\\\\name")
                if cp.has_option("hosts", name_key):
                    print(cp.get("hosts", name_key))
                    sys.exit(0)
    print("Unknown App")
except Exception:
    print("IDLE")
`, String(host), String(port)];
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
        id: sessionBadge
        visible: root.hasActiveSession && (root.appName === "" || root.activeAppName === root.appName || root.activeAppName === (root.target.app || ""))
        height: 24
        radius: 12
        color: Theme.online
        width: badgeText.implicitWidth + 16
        anchors.top: parent.top
        anchors.right: parent.right
        anchors.topMargin: 8
        anchors.rightMargin: 8

        Text {
            id: badgeText
            anchors.centerIn: parent
            text: "LIVE"
            font.pixelSize: 12
            font.bold: true
            color: "#ffffff"
        }
    }
}
