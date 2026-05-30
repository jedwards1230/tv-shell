import QtQuick
import QtQuick.Layouts
import Quickshell.Io

// Network settings — rewired (Phase 3) to read system network state from the
// input daemon's NetworkManager-over-zbus backbone instead of shelling out to
// `nmcli` / `ip`. This page is READ-ONLY: WiFi *join* still belongs to
// `nmcli device wifi connect` and is intentionally not implemented here.
//
// IPC commands used (see docs/IPC_PROTOCOL.md):
//   net-status     -> {connectivity, primaryType, hasWifi, ipv4, activeConnections:[{name,type,device}]}
//   net-wifi-list  -> [{ssid, signal, security, inUse}]
//   net-wifi-rescan -> ok|error (NetworkManager RequestScan)
FocusScope {
    id: root

    property var activeConnections: []
    property string ipAddress: ""
    property var wifiNetworks: []
    property bool hasWifi: false

    // --- Socket helper: read until the FIRST newline (the daemon keeps the
    // connection open after replying). Mirrors the Phase 2 SettingsStore pattern. ---
    function _ipc(cmd) {
        return "import socket,os;s=socket.socket(socket.AF_UNIX,socket.SOCK_STREAM);s.settimeout(20);s.connect(os.environ.get('GAME_SHELL_SOCK','/run/user/'+str(os.getuid())+'/game-shell-input.sock'));s.sendall(b'" + cmd + "\\n');buf=b'';exec(\"while b'\\\\n' not in buf:\\n c=s.recv(65536)\\n if not c: break\\n buf+=c\");s.close();print(buf.split(b'\\n',1)[0].decode())";
    }

    // --- Processes ---

    // Single round-trip for connectivity, primary type, IPv4, and the active
    // connection list — replaces three separate nmcli/ip/bash invocations.
    Process {
        id: getStatus
        command: ["python3", "-c", root._ipc("net-status")]
        stdout: SplitParser {
            onRead: line => {
                try {
                    let obj = JSON.parse(line);
                    root.activeConnections = Array.isArray(obj.activeConnections) ? obj.activeConnections : [];
                    root.ipAddress = obj.ipv4 || "";
                    root.hasWifi = obj.hasWifi === true;
                    if (root.hasWifi)
                        getWifi.running = true;
                } catch (e) {
                    console.log("NetworkSettings: failed to parse net-status:", e);
                }
            }
        }
    }

    Process {
        id: getWifi
        command: ["python3", "-c", root._ipc("net-wifi-list")]
        stdout: SplitParser {
            onRead: line => {
                try {
                    let nets = JSON.parse(line);
                    // Deduplicate by SSID, keep highest signal.
                    let seen = {};
                    for (let i = 0; i < nets.length; i++) {
                        let net = nets[i];
                        if (!net.ssid || net.ssid === "")
                            continue;
                        if (!seen[net.ssid] || net.signal > seen[net.ssid].signal)
                            seen[net.ssid] = net;
                    }
                    let deduped = [];
                    for (let ssid in seen)
                        deduped.push(seen[ssid]);
                    deduped.sort(function (a, b) {
                        return b.signal - a.signal;
                    });
                    root.wifiNetworks = deduped;
                } catch (e) {
                    console.log("NetworkSettings: failed to parse net-wifi-list:", e);
                }
            }
        }
    }

    // Ask NetworkManager to rescan, then refresh status (which pulls the list).
    Process {
        id: rescanWifi
        command: ["python3", "-c", root._ipc("net-wifi-rescan")]
        onExited: {
            getStatus.running = true;
        }
    }

    function refresh() {
        // A rescan kicks off discovery; net-status/net-wifi-list then read the
        // freshest results. On a wired-only host the rescan is a harmless no-op.
        rescanWifi.running = true;
    }

    Component.onCompleted: {
        getStatus.running = true;
        rescanWifi.running = true;
    }

    onVisibleChanged: {
        if (visible)
            refresh();
    }

    // Read-only page: focus the WiFi list when an adapter exists, otherwise
    // take scope-level focus on the root so entry still registers and Left/B
    // bubble back to the sidebar.
    function focusFirst() {
        if (root.hasWifi)
            wifiList.forceActiveFocus();
        else
            root.forceActiveFocus();
    }

    ColumnLayout {
        anchors.fill: parent
        anchors.margins: Theme.padding
        spacing: 32

        // Connection status
        Text {
            text: "Active Connections"
            font.pixelSize: Theme.fontBody
            font.bold: true
            color: Theme.textPrimary
        }

        ListView {
            Layout.fillWidth: true
            Layout.preferredHeight: Math.min(contentHeight, 300)
            spacing: 8
            clip: true
            model: root.activeConnections
            interactive: false

            delegate: Rectangle {
                required property var modelData
                width: parent ? parent.width : 0
                height: 96
                radius: 16
                color: Theme.surface
                border.width: 2
                border.color: Theme.surfaceBorder

                RowLayout {
                    anchors.fill: parent
                    anchors.leftMargin: 24
                    anchors.rightMargin: 24
                    anchors.topMargin: 16
                    anchors.bottomMargin: 16
                    spacing: 16

                    Rectangle {
                        width: 20
                        height: 20
                        radius: 10
                        color: Theme.online
                    }

                    Text {
                        text: modelData.name
                        font.pixelSize: Theme.fontSmall
                        font.bold: true
                        color: Theme.textPrimary
                        Layout.fillWidth: true
                    }

                    Text {
                        text: modelData.type === "802-3-ethernet" ? "Ethernet" : modelData.type === "802-11-wireless" ? "WiFi" : modelData.type
                        font.pixelSize: Theme.fontSmall
                        color: Theme.textSecondary
                    }

                    Text {
                        text: modelData.device
                        font.pixelSize: Theme.fontSmall
                        color: Theme.textSecondary
                    }
                }
            }
        }

        Text {
            text: root.activeConnections.length === 0 ? "No active connections" : ""
            font.pixelSize: Theme.fontSmall
            color: Theme.textSecondary
            visible: text !== ""
        }

        // IP Address
        Text {
            text: "IP Addresses"
            font.pixelSize: Theme.fontBody
            font.bold: true
            color: Theme.textPrimary
        }

        Rectangle {
            Layout.fillWidth: true
            height: ipLabel.implicitHeight + 48
            radius: 16
            color: Theme.surface

            Text {
                id: ipLabel
                anchors.fill: parent
                anchors.margins: 24
                text: root.ipAddress || "Fetching..."
                font.pixelSize: Theme.fontSmall
                font.family: "monospace"
                color: Theme.textPrimary
                wrapMode: Text.Wrap
            }
        }

        // WiFi networks
        Text {
            text: "WiFi Networks"
            font.pixelSize: Theme.fontBody
            font.bold: true
            color: Theme.textPrimary
            visible: root.hasWifi
        }

        ListView {
            id: wifiList
            Layout.fillWidth: true
            Layout.fillHeight: true
            spacing: 8
            clip: true
            visible: root.hasWifi
            model: root.wifiNetworks
            focus: root.hasWifi

            delegate: Rectangle {
                required property int index
                required property var modelData
                width: wifiList.width
                height: 96
                radius: 16
                color: {
                    if (modelData.inUse)
                        return Theme.sidebarActive;
                    if (wifiList.currentIndex === index && wifiList.activeFocus)
                        return Theme.surfaceHover;
                    return Theme.surface;
                }
                border.width: 2
                border.color: modelData.inUse ? Theme.focusBorder : Theme.surfaceBorder

                Behavior on color {
                    ColorAnimation {
                        duration: 150
                    }
                }

                RowLayout {
                    anchors.fill: parent
                    anchors.leftMargin: 24
                    anchors.rightMargin: 24
                    anchors.topMargin: 16
                    anchors.bottomMargin: 16
                    spacing: 16

                    Text {
                        text: modelData.ssid
                        font.pixelSize: Theme.fontSmall
                        font.bold: modelData.inUse
                        color: modelData.inUse ? Theme.textOnDark : Theme.textPrimary
                        Layout.fillWidth: true
                    }

                    // Signal strength bar
                    Row {
                        spacing: 4
                        Repeater {
                            model: 4
                            Rectangle {
                                required property int index
                                width: 8
                                height: 12 + index * 8
                                radius: 2
                                color: {
                                    let threshold = (index + 1) * 25;
                                    let active = modelData.signal >= threshold;
                                    if (modelData.inUse)
                                        return active ? Theme.textOnDark : Theme.textOnDarkMuted;
                                    return active ? Theme.focusBorder : Theme.surfaceHover;
                                }
                                anchors.bottom: parent.bottom
                            }
                        }
                    }

                    Text {
                        text: modelData.signal + "%"
                        font.pixelSize: Theme.fontSmall
                        color: modelData.inUse ? Theme.textOnDarkMuted : Theme.textSecondary
                    }

                    Text {
                        text: modelData.security
                        font.pixelSize: Theme.fontSmall
                        color: modelData.inUse ? Theme.textOnDarkMuted : Theme.textSecondary
                    }

                    Text {
                        text: modelData.inUse ? "Connected" : ""
                        font.pixelSize: Theme.fontSmall
                        font.bold: true
                        color: Theme.textOnDark
                        visible: modelData.inUse
                    }
                }

                MouseArea {
                    anchors.fill: parent
                    hoverEnabled: true
                    cursorShape: Qt.PointingHandCursor
                    onClicked: {
                        wifiList.currentIndex = index;
                        wifiList.forceActiveFocus();
                    }
                }
            }
        }

        Text {
            text: !root.hasWifi ? "No WiFi adapter detected" : root.wifiNetworks.length === 0 ? "No WiFi networks found" : ""
            font.pixelSize: Theme.fontSmall
            color: Theme.textSecondary
            visible: text !== ""
        }

        // Info hint
        Text {
            text: "Network configuration is read-only"
            font.pixelSize: Theme.fontHint
            color: Theme.textSecondary
            Layout.alignment: Qt.AlignHCenter
        }
    }
}
