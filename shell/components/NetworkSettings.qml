import QtQuick
import QtQuick.Layouts
import Quickshell.Io
import "lib"

// Network settings — rewired (Phase 3) to read system network state from the
// input daemon's NetworkManager-over-zbus backbone instead of shelling out to
// `nmcli` / `ip`. This page is READ-ONLY: WiFi *join* still belongs to
// `nmcli device wifi connect` and is intentionally not implemented here.
//
// IPC commands used (see docs/IPC_PROTOCOL.md):
//   net-status     -> {connectivity, primaryType, hasWifi, ipv4, gateway, dns, activeConnections:[{name,type,device,speed}]}
//   net-wifi-list  -> [{ssid, signal, security, inUse}]
//   net-wifi-rescan -> ok|error (NetworkManager RequestScan)
FocusScope {
    id: root
    implicitHeight: netMainCol.implicitHeight + 2 * Theme.padding

    property var activeConnections: []
    property string ipAddress: ""
    property var wifiNetworks: []
    property bool hasWifi: false
    property string gateway: ""
    property var dnsServers: []
    property string testResult: ""

    // --- Daemon IPC over a native Quickshell socket (SocketClient, #97) — the
    // python3 socket shim was retired in Phase 8. ---

    // --- Processes ---

    // Single round-trip for connectivity, primary type, IPv4, and the active
    // connection list — replaces three separate nmcli/ip/bash invocations.
    SocketClient {
        id: getStatus
        onResponseReceived: line => {
            try {
                let obj = JSON.parse(line);
                root.activeConnections = Array.isArray(obj.activeConnections) ? obj.activeConnections : [];
                root.ipAddress = obj.ipv4 || "";
                root.hasWifi = obj.hasWifi === true;
                root.gateway = obj.gateway || "";
                root.dnsServers = Array.isArray(obj.dns) ? obj.dns : [];
                if (root.hasWifi)
                    getWifi.request("net-wifi-list");
            } catch (e) {
                console.log("NetworkSettings: failed to parse net-status:", e);
            }
        }
    }

    SocketClient {
        id: getWifi
        onResponseReceived: line => {
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

    // Ask NetworkManager to rescan, then refresh status (which pulls the list).
    SocketClient {
        id: rescanWifi
        onResponseReceived: response => {
            getStatus.request("net-status");
        }
        onRequestFailed: {
            // Daemon unreachable / rescan socket failed — still try a status read.
            getStatus.request("net-status");
        }
    }

    // Bounded one-shot ping for the Test connection action.
    // `-c 3 -W 2` ensures it exits within ~6 s regardless of host reachability.
    property string pingOutput: ""

    Process {
        id: pingTest
        command: ["bash", "-c", "ping -c 3 -W 2 1.1.1.1 2>&1"]
        stdout: SplitParser {
            onRead: line => {
                root.pingOutput += line + "\n";
            }
        }
        onExited: (code, status) => {
            if (code === 0) {
                // Extract average RTT from the ping summary line, e.g.
                // "rtt min/avg/max/mdev = 12.3/14.5/16.7/1.2 ms"
                let match = root.pingOutput.match(/= [\d.]+\/([\d.]+)\//);
                if (match)
                    root.testResult = "OK — " + Math.round(parseFloat(match[1])) + " ms avg";
                else
                    root.testResult = "OK";
            } else {
                root.testResult = "Failed";
            }
        }
    }

    function refresh() {
        // A rescan kicks off discovery; net-status/net-wifi-list then read the
        // freshest results. On a wired-only host the rescan is a harmless no-op.
        rescanWifi.request("net-wifi-rescan");
    }

    Component.onCompleted: {
        getStatus.request("net-status");
        rescanWifi.request("net-wifi-rescan");
    }

    onVisibleChanged: {
        if (visible)
            refresh();
    }

    // Read-only page: focus the WiFi list when an adapter exists; otherwise
    // focus the Test button so D-pad entry still registers on a wired-only host.
    function focusFirst() {
        if (root.hasWifi)
            wifiList.forceActiveFocus();
        else
            testButtonScope.forceActiveFocus();
    }

    ColumnLayout {
        id: netMainCol
        anchors.fill: parent
        anchors.margins: Theme.padding
        spacing: 32

        // Connection status
        SectionHeader {
            text: "Active Connections"
        }

        SettingsList {
            // rowStride = delegate 96 + spacing 8 (#123/#139 row-count sizing).
            rowStride: 104
            maxHeight: 300
            spacing: 8
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
                        Layout.preferredWidth: 20
                        Layout.preferredHeight: 20
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

                    // Link speed — shown only for wired connections where NM
                    // reports a non-zero speed value.
                    Text {
                        visible: (modelData.speed || 0) > 0
                        text: (modelData.speed || 0) + " Mb/s"
                        font.pixelSize: Theme.fontSmall
                        color: Theme.textSecondary
                    }
                }
            }
        }

        SettingsEmptyState {
            Layout.fillWidth: true
            Layout.preferredHeight: Units.gridUnit * 3
            visible: root.activeConnections.length === 0
            line: "No active connections"
        }

        // IP Address
        SectionHeader {
            text: "IP Addresses"
        }

        Rectangle {
            Layout.fillWidth: true
            Layout.preferredHeight: ipLabel.implicitHeight + 48
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

        // Gateway / DNS — read-only card mirroring the IP Addresses card.
        SectionHeader {
            text: "Gateway / DNS"
            visible: root.gateway !== "" || root.dnsServers.length > 0
        }

        Rectangle {
            Layout.fillWidth: true
            Layout.preferredHeight: gatewayDnsContent.implicitHeight + 48
            radius: 16
            color: Theme.surface
            visible: root.gateway !== "" || root.dnsServers.length > 0

            Column {
                id: gatewayDnsContent
                anchors.fill: parent
                anchors.margins: 24
                spacing: 8

                Text {
                    text: "GW: " + (root.gateway || "—")
                    font.pixelSize: Theme.fontSmall
                    font.family: "monospace"
                    color: Theme.textPrimary
                    visible: root.gateway !== ""
                }

                Repeater {
                    model: root.dnsServers
                    Text {
                        required property var modelData
                        text: "DNS: " + modelData
                        font.pixelSize: Theme.fontSmall
                        font.family: "monospace"
                        color: Theme.textSecondary
                    }
                }
            }
        }

        // Diagnostics
        SectionHeader {
            text: "Diagnostics"
        }

        // Test connection action — bounded one-shot ping.
        RowLayout {
            Layout.fillWidth: true
            spacing: 24

            FocusScope {
                id: testButtonScope
                width: testButton.implicitWidth
                height: testButton.implicitHeight

                KeyNavigation.up: wifiList

                SettingsButton {
                    id: testButton
                    text: "Test connection"
                    focus: parent.activeFocus
                    anchors.fill: parent
                    onActivated: {
                        root.testResult = "Testing…";
                        root.pingOutput = "";
                        pingTest.running = true;
                    }
                }
            }

            Text {
                text: root.testResult
                font.pixelSize: Theme.fontSmall
                color: root.testResult.startsWith("OK") ? Theme.online : root.testResult === "Failed" ? Theme.offline : Theme.textSecondary
                visible: root.testResult !== ""
                Layout.fillWidth: true
            }
        }

        // WiFi networks
        SectionHeader {
            text: "WiFi Networks"
            visible: root.hasWifi
        }

        SettingsList {
            id: wifiList
            KeyNavigation.down: testButtonScope
            // rowStride = delegate 96 + spacing 8 (#123/#139 row-count sizing).
            rowStride: 104
            maxHeight: 400
            spacing: 8
            visible: root.hasWifi && root.wifiNetworks.length > 0
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
                                    return active ? Theme.online : Theme.textSecondary;
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

        SettingsEmptyState {
            Layout.fillWidth: true
            Layout.preferredHeight: Units.gridUnit * 3
            visible: !root.hasWifi || root.wifiNetworks.length === 0
            line: !root.hasWifi ? "No WiFi adapter detected" : "No WiFi networks found"
        }

        // Absorb remaining vertical space so content top-packs and the hint
        // pins to the bottom (mirrors ControllerSettings.qml).
        Item {
            Layout.fillHeight: true
        }

        HintBar {
            text: "Network configuration is read-only"
        }
    }
}
