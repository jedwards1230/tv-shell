import QtQuick
import QtQuick.Layouts
import Quickshell.Io

// AV Control settings — rewritten (Phase 4, #16) to consume the daemon's
// `cec-*` IPC instead of shelling out to cec-ctl / cec-client / living-room-cec.
//
// IPC commands used (see docs/IPC_PROTOCOL.md):
//   cec-scan          -> JSON array of {logicalAddress, physicalAddress,
//                        vendor, osdName, powerStatus, type}
//   cec-power-on <addr>   -> ok | error:*
//   cec-power-off <addr>  -> ok | error:*
//   cec-active-source     -> ok | error:*
//
// Subscribe events (prefix-matched on the subscribe stream):
//   cec:device:<json>  -> full device object; merge/update into root.devices
//   cec:power:<json>   -> {addr, power}; update powerStatus for that device
//
// Root MUST be a FocusScope (not Item) so SettingsPanel's
// contentLoader.item.forceActiveFocus() delegates focus into the
// focus:true child (wakeScope). A plain Item swallows focus and the
// page becomes unnavigable when entered via the Right d-pad.
FocusScope {
    id: root

    property bool cecAvailable: false
    property var devices: []
    property string statusText: "Checking CEC availability..."
    property string actionFeedback: ""

    // --- Daemon IPC via SocketClient (mirrors NetworkSettings.qml) ---

    // Round-trip scan: request `cec-scan`, receive a JSON array of devices.
    SocketClient {
        id: scanClient
        onResponseReceived: line => {
            var trimmed = line.trim();
            if (trimmed.length === 0 || (trimmed[0] !== "[" && trimmed[0] !== "{")) {
                // error:* reply or empty — daemon reports no CEC adapter
                root.cecAvailable = false;
                root.statusText = "CEC unavailable";
            } else {
                try {
                    var arr = JSON.parse(trimmed);
                    root.devices = Array.isArray(arr) ? arr : [];
                    root.cecAvailable = true;
                    if (root.devices.length > 0) {
                        root.statusText = root.devices.length + " device" + (root.devices.length !== 1 ? "s" : "") + " detected";
                    } else {
                        root.statusText = "No CEC devices found";
                    }
                } catch (e) {
                    console.log("AVControlSettings: failed to parse cec-scan:", e);
                    root.cecAvailable = false;
                    root.statusText = "CEC unavailable";
                }
            }
        }
        onRequestFailed: {
            root.cecAvailable = false;
            root.statusText = "CEC unavailable";
        }
    }

    // Subscribe stream: receive live cec:device:* and cec:power:* events to
    // fix intermittent detection — device rows update without polling a subprocess.
    SocketClient {
        id: cecEvents
        subscribe: true
        onLineReceived: line => {
            if (line.startsWith("cec:device:")) {
                try {
                    var obj = JSON.parse(line.substring(11));
                    // Merge/update the device into root.devices by logicalAddress.
                    var updated = false;
                    var copy = root.devices.slice();
                    for (var i = 0; i < copy.length; i++) {
                        if (copy[i].logicalAddress === obj.logicalAddress) {
                            copy[i] = obj;
                            updated = true;
                            break;
                        }
                    }
                    if (!updated)
                        copy.push(obj);
                    root.devices = copy;
                    root.cecAvailable = true;
                    if (root.devices.length > 0) {
                        root.statusText = root.devices.length + " device" + (root.devices.length !== 1 ? "s" : "") + " detected";
                    }
                } catch (e) {
                    console.log("AVControlSettings: failed to parse cec:device event:", e);
                }
            } else if (line.startsWith("cec:power:")) {
                try {
                    var pwr = JSON.parse(line.substring(10));
                    var pwrCopy = root.devices.slice();
                    for (var j = 0; j < pwrCopy.length; j++) {
                        if (pwrCopy[j].logicalAddress === pwr.addr) {
                            pwrCopy[j] = Object.assign({}, pwrCopy[j], {
                                powerStatus: pwr.power
                            });
                            break;
                        }
                    }
                    root.devices = pwrCopy;
                } catch (e) {
                    console.log("AVControlSettings: failed to parse cec:power event:", e);
                }
            }
        }
    }

    // Action clients — each is a one-shot request/response socket.
    SocketClient {
        id: wakeClient
        onResponseReceived: response => {
            root.actionFeedback = response === "ok" ? "Wake command sent" : "Wake command failed";
            feedbackTimer.restart();
            if (response === "ok")
                refreshAfterAction.restart();
        }
        onRequestFailed: {
            root.actionFeedback = "Wake command failed";
            feedbackTimer.restart();
        }
    }

    SocketClient {
        id: sleepClient
        onResponseReceived: response => {
            root.actionFeedback = response === "ok" ? "Sleep command sent" : "Sleep command failed";
            feedbackTimer.restart();
            if (response === "ok")
                refreshAfterAction.restart();
        }
        onRequestFailed: {
            root.actionFeedback = "Sleep command failed";
            feedbackTimer.restart();
        }
    }

    SocketClient {
        id: switchClient
        onResponseReceived: response => {
            root.actionFeedback = response === "ok" ? "Input switch command sent" : "Input switch command failed";
            feedbackTimer.restart();
        }
        onRequestFailed: {
            root.actionFeedback = "Input switch command failed";
            feedbackTimer.restart();
        }
    }

    // --- Timers ---
    Timer {
        id: autoRefresh
        interval: 30000
        running: root.visible && root.cecAvailable
        repeat: true
        onTriggered: {
            scanClient.request("cec-scan");
        }
    }

    Timer {
        id: feedbackTimer
        interval: 3000
        onTriggered: {
            root.actionFeedback = "";
        }
    }

    Timer {
        id: refreshAfterAction
        interval: 5000
        onTriggered: {
            scanClient.request("cec-scan");
        }
    }

    Component.onCompleted: {
        scanClient.request("cec-scan");
        cecEvents.start();
    }

    onVisibleChanged: {
        if (visible) {
            scanClient.request("cec-scan");
            cecEvents.start();
        } else {
            cecEvents.stop();
        }
    }

    // Focus the Wake button when CEC is available, otherwise take scope-level
    // focus on the root (read-only state) so entry registers and Left/B return.
    function focusFirst() {
        if (root.cecAvailable)
            wakeScope.forceActiveFocus();
        else
            root.forceActiveFocus();
    }

    function doWake() {
        wakeClient.request("cec-power-on 0");
    }

    function doSleep() {
        sleepClient.request("cec-power-off 0");
    }

    function doSwitchInput() {
        switchClient.request("cec-active-source");
    }

    // --- UI ---
    ColumnLayout {
        anchors.fill: parent
        anchors.margins: Theme.padding
        spacing: 32

        // Header with status
        RowLayout {
            Layout.fillWidth: true
            spacing: 24

            Item {
                Layout.fillWidth: true
            }

            // Status indicator
            Rectangle {
                width: statusRow.implicitWidth + 32
                height: 56
                radius: 28
                color: root.cecAvailable ? Qt.rgba(0.176, 0.541, 0.306, 0.2) : Theme.surfaceHover

                RowLayout {
                    id: statusRow
                    anchors.centerIn: parent
                    spacing: 12

                    Rectangle {
                        width: 16
                        height: 16
                        radius: 8
                        color: root.cecAvailable ? Theme.online : Theme.textMuted
                    }

                    Text {
                        text: root.statusText
                        font.pixelSize: Theme.fontSmall
                        color: root.cecAvailable ? Theme.online : Theme.textSecondary
                    }
                }
            }

            // Refresh button
            FocusScope {
                id: refreshScope
                width: refreshBtn.implicitWidth
                height: refreshBtn.implicitHeight
                visible: root.cecAvailable

                KeyNavigation.down: wakeScope

                SettingsButton {
                    id: refreshBtn
                    text: "Refresh"
                    focus: parent.activeFocus
                    anchors.fill: parent

                    onActivated: {
                        root.statusText = "Scanning...";
                        scanClient.request("cec-scan");
                    }
                }

                MouseArea {
                    anchors.fill: parent
                    hoverEnabled: true
                    cursorShape: Qt.PointingHandCursor
                    onClicked: {
                        refreshScope.forceActiveFocus();
                        refreshBtn.activated();
                    }
                }
            }
        }

        // CEC unavailable message
        Rectangle {
            Layout.fillWidth: true
            Layout.preferredHeight: 200
            radius: 24
            color: Theme.surface
            border.width: 2
            border.color: Theme.surfaceBorder
            visible: !root.cecAvailable

            ColumnLayout {
                anchors.centerIn: parent
                spacing: 16

                Text {
                    text: "HDMI-CEC Not Available"
                    font.pixelSize: Theme.fontTitle
                    font.bold: true
                    color: Theme.textPrimary
                    Layout.alignment: Qt.AlignHCenter
                }

                Text {
                    text: "The daemon found no CEC adapter on this system."
                    font.pixelSize: Theme.fontSmall
                    color: Theme.textSecondary
                    Layout.alignment: Qt.AlignHCenter
                }
            }
        }

        // Quick actions — always visible when CEC available
        ColumnLayout {
            Layout.fillWidth: true
            spacing: 16
            visible: root.cecAvailable

            Text {
                text: "Quick Actions"
                font.pixelSize: Theme.fontBody
                font.bold: true
                color: Theme.textPrimary
            }

            // Feedback text
            Text {
                text: root.actionFeedback
                font.pixelSize: Theme.fontSmall
                color: Theme.online
                visible: root.actionFeedback !== ""
                Layout.alignment: Qt.AlignHCenter
            }

            RowLayout {
                Layout.alignment: Qt.AlignHCenter
                spacing: 32

                // Wake AV
                FocusScope {
                    id: wakeScope
                    width: 340
                    height: 120
                    // Only claim the root scope's focus when actually visible
                    // (CEC available). Otherwise the root FocusScope holds
                    // focus itself so the read-only state stays dismissable.
                    focus: root.cecAvailable
                    activeFocusOnTab: true

                    KeyNavigation.up: refreshScope
                    KeyNavigation.right: sleepScope
                    KeyNavigation.down: deviceListView.count > 0 ? deviceListView : wakeScope

                    Rectangle {
                        anchors.fill: parent
                        radius: 24
                        color: parent.activeFocus ? Theme.online : Theme.surface
                        border.width: parent.activeFocus ? 0 : 2
                        border.color: Theme.surfaceBorder

                        Behavior on color {
                            ColorAnimation {
                                duration: 150
                            }
                        }

                        ColumnLayout {
                            anchors.centerIn: parent
                            spacing: 4

                            Text {
                                text: "Wake AV"
                                font.pixelSize: Theme.fontTitle
                                font.bold: true
                                color: wakeScope.activeFocus ? Theme.textOnDark : Theme.textPrimary
                                Layout.alignment: Qt.AlignHCenter
                            }

                            Text {
                                text: "CEC power on"
                                font.pixelSize: Theme.fontSmall
                                color: wakeScope.activeFocus ? Theme.textOnDarkMuted : Theme.textSecondary
                                Layout.alignment: Qt.AlignHCenter
                            }
                        }

                        MouseArea {
                            anchors.fill: parent
                            hoverEnabled: true
                            cursorShape: Qt.PointingHandCursor
                            onClicked: {
                                wakeScope.forceActiveFocus();
                                root.doWake();
                            }
                        }
                    }

                    Keys.onReturnPressed: {
                        root.doWake();
                    }
                }

                // Sleep AV
                FocusScope {
                    id: sleepScope
                    width: 340
                    height: 120
                    activeFocusOnTab: true

                    KeyNavigation.left: wakeScope
                    KeyNavigation.right: switchScope
                    KeyNavigation.down: deviceListView.count > 0 ? deviceListView : sleepScope

                    Rectangle {
                        anchors.fill: parent
                        radius: 24
                        color: parent.activeFocus ? Theme.gold : Theme.surface
                        border.width: parent.activeFocus ? 0 : 2
                        border.color: Theme.surfaceBorder

                        Behavior on color {
                            ColorAnimation {
                                duration: 150
                            }
                        }

                        ColumnLayout {
                            anchors.centerIn: parent
                            spacing: 4

                            Text {
                                text: "Sleep AV"
                                font.pixelSize: Theme.fontTitle
                                font.bold: true
                                color: sleepScope.activeFocus ? Theme.textOnDark : Theme.textPrimary
                                Layout.alignment: Qt.AlignHCenter
                            }

                            Text {
                                text: "CEC standby"
                                font.pixelSize: Theme.fontSmall
                                color: sleepScope.activeFocus ? Theme.textOnDarkMuted : Theme.textSecondary
                                Layout.alignment: Qt.AlignHCenter
                            }
                        }

                        MouseArea {
                            anchors.fill: parent
                            hoverEnabled: true
                            cursorShape: Qt.PointingHandCursor
                            onClicked: {
                                sleepScope.forceActiveFocus();
                                root.doSleep();
                            }
                        }
                    }

                    Keys.onReturnPressed: {
                        root.doSleep();
                    }
                }

                // Switch Input
                FocusScope {
                    id: switchScope
                    width: 340
                    height: 120
                    activeFocusOnTab: true

                    KeyNavigation.left: sleepScope
                    KeyNavigation.down: deviceListView.count > 0 ? deviceListView : switchScope

                    Rectangle {
                        anchors.fill: parent
                        radius: 24
                        color: parent.activeFocus ? Theme.ember : Theme.surface
                        border.width: parent.activeFocus ? 0 : 2
                        border.color: Theme.surfaceBorder

                        Behavior on color {
                            ColorAnimation {
                                duration: 150
                            }
                        }

                        ColumnLayout {
                            anchors.centerIn: parent
                            spacing: 4

                            Text {
                                text: "Switch Input"
                                font.pixelSize: Theme.fontTitle
                                font.bold: true
                                color: switchScope.activeFocus ? Theme.textOnDark : Theme.textPrimary
                                Layout.alignment: Qt.AlignHCenter
                            }

                            Text {
                                text: "Set active source"
                                font.pixelSize: Theme.fontSmall
                                color: switchScope.activeFocus ? Theme.textOnDarkMuted : Theme.textSecondary
                                Layout.alignment: Qt.AlignHCenter
                            }
                        }

                        MouseArea {
                            anchors.fill: parent
                            hoverEnabled: true
                            cursorShape: Qt.PointingHandCursor
                            onClicked: {
                                switchScope.forceActiveFocus();
                                root.doSwitchInput();
                            }
                        }
                    }

                    Keys.onReturnPressed: {
                        root.doSwitchInput();
                    }
                }
            }
        }

        // Device list
        ColumnLayout {
            Layout.fillWidth: true
            Layout.fillHeight: true
            spacing: 16
            visible: root.cecAvailable

            Text {
                text: "Detected Devices"
                font.pixelSize: Theme.fontBody
                font.bold: true
                color: Theme.textPrimary
            }

            // No devices placeholder
            Rectangle {
                Layout.fillWidth: true
                Layout.preferredHeight: 120
                radius: 16
                color: Theme.surface
                border.width: 2
                border.color: Theme.surfaceBorder
                visible: root.devices.length === 0

                Text {
                    anchors.centerIn: parent
                    text: "No CEC devices detected. Try refreshing."
                    font.pixelSize: Theme.fontSmall
                    color: Theme.textSecondary
                }
            }

            ListView {
                id: deviceListView
                Layout.fillWidth: true
                Layout.fillHeight: true
                spacing: 16
                clip: true
                model: root.devices
                visible: root.devices.length > 0

                KeyNavigation.up: wakeScope

                delegate: Rectangle {
                    required property int index
                    required property var modelData
                    width: deviceListView.width
                    height: 160
                    radius: 16
                    color: deviceListView.currentIndex === index && deviceListView.activeFocus ? Theme.surfaceHover : Theme.surface
                    border.width: 2
                    border.color: Theme.surfaceBorder

                    Behavior on color {
                        ColorAnimation {
                            duration: 150
                        }
                    }

                    RowLayout {
                        anchors.fill: parent
                        anchors.leftMargin: 32
                        anchors.rightMargin: 32
                        spacing: 24

                        // Device icon/name
                        ColumnLayout {
                            spacing: 8
                            Layout.fillWidth: true

                            RowLayout {
                                spacing: 16

                                Text {
                                    // OSD name from daemon; fall back to type/address label if empty.
                                    text: (modelData.osdName && modelData.osdName !== "") ? modelData.osdName : (modelData.type ? modelData.type + " " + modelData.logicalAddress : "Device " + modelData.logicalAddress)
                                    font.pixelSize: Theme.fontBody
                                    font.bold: true
                                    color: Theme.textPrimary
                                }

                                // Power status badge
                                Rectangle {
                                    width: powerLabel.implicitWidth + 24
                                    height: 40
                                    radius: 20
                                    color: {
                                        if (modelData.powerStatus === "on")
                                            return Qt.rgba(0.176, 0.541, 0.306, 0.2);
                                        if (modelData.powerStatus === "standby")
                                            return Qt.rgba(0.843, 0.651, 0.294, 0.2);
                                        return Theme.surfaceHover;
                                    }

                                    Text {
                                        id: powerLabel
                                        anchors.centerIn: parent
                                        text: {
                                            if (modelData.powerStatus === "on")
                                                return "On";
                                            if (modelData.powerStatus === "standby")
                                                return "Standby";
                                            return "Unknown";
                                        }
                                        font.pixelSize: Theme.fontCaption
                                        color: {
                                            if (modelData.powerStatus === "on")
                                                return Theme.online;
                                            if (modelData.powerStatus === "standby")
                                                return Theme.gold;
                                            return Theme.textSecondary;
                                        }
                                    }
                                }
                            }

                            RowLayout {
                                spacing: 32

                                Text {
                                    text: "Address: " + modelData.logicalAddress + (modelData.physicalAddress ? " (" + modelData.physicalAddress + ")" : "")
                                    font.pixelSize: Theme.fontSmall
                                    color: Theme.textSecondary
                                }

                                Text {
                                    text: "Vendor: " + (modelData.vendor || "Unknown")
                                    font.pixelSize: Theme.fontSmall
                                    color: Theme.textSecondary
                                    visible: modelData.vendor !== ""
                                }

                                Text {
                                    text: "Type: " + modelData.type
                                    font.pixelSize: Theme.fontSmall
                                    color: Theme.textSecondary
                                    visible: modelData.type !== ""
                                }
                            }
                        }
                    }

                    MouseArea {
                        anchors.fill: parent
                        hoverEnabled: true
                        cursorShape: Qt.PointingHandCursor
                        onClicked: {
                            deviceListView.currentIndex = index;
                            deviceListView.forceActiveFocus();
                        }
                    }
                }
            }
        }

        // Hint bar
        Text {
            text: root.cecAvailable ? "A: Select  |  Auto-refresh every 30s" : "HDMI-CEC unavailable — daemon reports no CEC adapter"
            font.pixelSize: Theme.fontHint
            color: Theme.textSecondary
            Layout.alignment: Qt.AlignHCenter
        }
    }
}
