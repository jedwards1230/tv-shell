import QtQuick
import QtQuick.Layouts
import Quickshell.Io
import "lib"

// AV Control settings — rewritten (#16) to consume the daemon's `cec-*` IPC
// instead of shelling out to cec-ctl / cec-client / living-room-cec. The daemon
// owns one persistent in-process libcec connection (cec-rs), which fixes the
// intermittent detection the per-call subprocess approach suffered from.
//
// IPC commands used (see docs/IPC_PROTOCOL.md):
//   cec-scan              -> JSON array of {logicalAddress, powerStatus}
//   cec-power-on <addr>   -> ok | error:*
//   cec-power-off <addr>  -> ok | error:*
//   cec-active-source     -> ok | error:*  ("Switch Input" = set active source)
//
// Subscribe events (prefix-matched on the subscribe stream):
//   cec:device:<json>  -> {logicalAddress, powerStatus}; merge into root.devices
//   cec:power:<json>   -> {addr, power}; update powerStatus for that device
//
// NOTE: cec-rs 12.0.1 wraps no per-device metadata query (OSD name, physical
// address, vendor, type), so the daemon emits only logicalAddress + powerStatus.
// Friendly names are derived locally from the logical address. The former
// living-room-cec / cec-ctl / cec-client QML fallback chain is removed — when
// the daemon is built without `--features cec` (or libcec is absent at runtime)
// every cec-* command replies error:* and this panel shows an unavailable card.
//
// Root MUST be a FocusScope (not Item) so SettingsPanel's
// contentLoader.item.forceActiveFocus() delegates focus into the focus:true
// child (wakeScope). A plain Item swallows focus and the page becomes
// unnavigable when entered via the Right d-pad.
FocusScope {
    id: root
    // SettingsPanel sizes the scroll pane from item.implicitHeight; without this
    // the pane height is 0 and lower controls become unreachable (mirror the
    // other settings pages). Derived from the content column's implicit size.
    implicitHeight: avMainCol.implicitHeight + 2 * Theme.padding

    property bool cecAvailable: false
    property var devices: []
    property string statusText: "Checking CEC availability..."
    property string actionFeedback: ""

    // Friendly label for a CEC logical address (no OSD name in cec-rs 12.0.1).
    function nameForAddress(addr) {
        // Prefer a local name override (#16, set via the config file — a freeform
        // on-screen editor is deferred to #20). Keyed by the stringified address.
        var o = SettingsStore.cecDeviceNames[String(addr)];
        if (o && o.length)
            return o;
        // CEC logical-address table (HDMI-CEC spec), consistent with the
        // daemon's cec-rs CecLogicalAddress semantics.
        switch (addr) {
        case 0:
            return "TV";
        case 1:
            return "Recorder 1";
        case 2:
            return "Recorder 2";
        case 9:
            return "Recorder 3";
        case 3:
            return "Tuner 1";
        case 6:
            return "Tuner 2";
        case 7:
            return "Tuner 3";
        case 10:
            return "Tuner 4";
        case 4:
            return "Playback 1";
        case 8:
            return "Playback 2";
        case 11:
            return "Playback 3";
        case 5:
            return "Audio System";
        case 14:
            return "Free Use";
        case 15:
            return "Broadcast";
        default:
            return "Device " + addr;  // 12, 13 reserved
        }
    }

    // --- Daemon IPC via SocketClient (mirrors NetworkSettings.qml) ---

    // Round-trip scan: request `cec-scan`, receive a JSON array of devices.
    SocketClient {
        id: scanClient
        onResponseReceived: line => {
            var trimmed = line.trim();
            if (trimmed.length === 0 || (trimmed[0] !== "[" && trimmed[0] !== "{")) {
                // error:* reply or empty — daemon reports no usable CEC adapter.
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

    // Subscribe stream: receive live cec:device:* and cec:power:* events so
    // device rows update without re-polling — the reliability win behind #16.
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
                    // The daemon sends addr as a wire string ("0"); logicalAddress
                    // is numeric, so coerce before matching.
                    var pwrAddr = Number(pwr.addr);
                    var pwrCopy = root.devices.slice();
                    for (var j = 0; j < pwrCopy.length; j++) {
                        if (pwrCopy[j].logicalAddress === pwrAddr) {
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

    // Focus the first toggle (always present) so the page is navigable even
    // when CEC is unavailable.
    function focusFirst() {
        focusStartupScope.forceActiveFocus();
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
        id: avMainCol
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
            StatusPill {
                pillState: root.cecAvailable ? "good" : "neutral"
                text: root.statusText
            }

            // Refresh button
            FocusScope {
                id: refreshScope
                width: refreshBtn.implicitWidth
                height: refreshBtn.implicitHeight
                visible: root.cecAvailable

                KeyNavigation.down: focusStartupScope

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

        // Focus preference toggles — always visible (not gated on cecAvailable).
        // The daemon reads these at CEC startup and resume to decide whether to
        // claim the active source.
        ColumnLayout {
            Layout.fillWidth: true
            spacing: 16

            SectionHeader {
                text: "Focus Preferences"
            }

            // Focus TV on startup toggle
            PreferenceRow {
                label: "Focus TV on startup"
                description: "Switch the TV/AVR to this input when the shell starts (off keeps your current input on restart)."

                FocusScope {
                    id: focusStartupScope
                    width: focusStartupBtn.width
                    height: focusStartupBtn.height
                    activeFocusOnTab: true

                    // Refresh is hidden when CEC is unavailable — wrap Up to the
                    // last (always-visible) toggle so focus can't vanish.
                    KeyNavigation.up: root.cecAvailable ? refreshScope : autoSwitchScope
                    KeyNavigation.down: focusWakeScope

                    SettingsButton {
                        id: focusStartupBtn
                        text: SettingsStore.cecFocusOnStartup ? "On" : "Off"
                        focus: parent.activeFocus
                        anchors.fill: parent

                        color: SettingsStore.cecFocusOnStartup ? Theme.sidebarActive : (parent.activeFocus ? Theme.surfaceHover : Theme.surface)

                        onActivated: SettingsStore.setCecFocusOnStartup(!SettingsStore.cecFocusOnStartup)

                        MouseArea {
                            anchors.fill: parent
                            hoverEnabled: true
                            cursorShape: Qt.PointingHandCursor
                            onClicked: {
                                focusStartupScope.forceActiveFocus();
                                focusStartupBtn.activated();
                            }
                        }
                    }
                }
            }

            // Focus TV on wake toggle
            PreferenceRow {
                label: "Focus TV on wake from sleep"
                description: "Switch to this input when the box wakes from sleep."

                FocusScope {
                    id: focusWakeScope
                    width: focusWakeBtn.width
                    height: focusWakeBtn.height
                    activeFocusOnTab: true

                    KeyNavigation.up: focusStartupScope
                    KeyNavigation.down: autoSwitchScope

                    SettingsButton {
                        id: focusWakeBtn
                        text: SettingsStore.cecFocusOnWake ? "On" : "Off"
                        focus: parent.activeFocus
                        anchors.fill: parent

                        color: SettingsStore.cecFocusOnWake ? Theme.sidebarActive : (parent.activeFocus ? Theme.surfaceHover : Theme.surface)

                        onActivated: SettingsStore.setCecFocusOnWake(!SettingsStore.cecFocusOnWake)

                        MouseArea {
                            anchors.fill: parent
                            hoverEnabled: true
                            cursorShape: Qt.PointingHandCursor
                            onClicked: {
                                focusWakeScope.forceActiveFocus();
                                focusWakeBtn.activated();
                            }
                        }
                    }
                }
            }

            // Auto-switch input on power-on toggle (persist-only in Phase 1; the
            // daemon does not act on it yet — behaviour wiring is a follow-up).
            PreferenceRow {
                label: "Auto-switch input on power-on"
                description: "Switch the TV/AVR to this input automatically when a device powers on."

                FocusScope {
                    id: autoSwitchScope
                    width: autoSwitchBtn.width
                    height: autoSwitchBtn.height
                    activeFocusOnTab: true

                    KeyNavigation.up: focusWakeScope
                    // When CEC is unavailable the action row below is hidden —
                    // wrap Down back to the first toggle instead of self-looping
                    // (which would trap focus).
                    KeyNavigation.down: root.cecAvailable ? wakeScope : focusStartupScope

                    SettingsButton {
                        id: autoSwitchBtn
                        text: SettingsStore.cecAutoSwitchOnPowerOn ? "On" : "Off"
                        focus: parent.activeFocus
                        anchors.fill: parent

                        color: SettingsStore.cecAutoSwitchOnPowerOn ? Theme.sidebarActive : (parent.activeFocus ? Theme.surfaceHover : Theme.surface)

                        onActivated: SettingsStore.setCecAutoSwitchOnPowerOn(!SettingsStore.cecAutoSwitchOnPowerOn)

                        MouseArea {
                            anchors.fill: parent
                            hoverEnabled: true
                            cursorShape: Qt.PointingHandCursor
                            onClicked: {
                                autoSwitchScope.forceActiveFocus();
                                autoSwitchBtn.activated();
                            }
                        }
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
                    text: "CEC requires the daemon built with libcec support."
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

            SectionHeader {
                text: "Quick Actions"
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
                    // (CEC available). Otherwise the root FocusScope holds focus
                    // itself so the read-only state stays dismissable.
                    focus: root.cecAvailable
                    activeFocusOnTab: true

                    KeyNavigation.up: autoSwitchScope
                    KeyNavigation.right: sleepScope
                    KeyNavigation.down: deviceRepeater.count > 0 ? deviceRepeater.itemAt(0) : wakeScope

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
                    KeyNavigation.down: deviceRepeater.count > 0 ? deviceRepeater.itemAt(0) : sleepScope

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
                    KeyNavigation.down: deviceRepeater.count > 0 ? deviceRepeater.itemAt(0) : switchScope

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
            spacing: 16
            visible: root.cecAvailable

            SectionHeader {
                text: "Detected Devices"
            }

            // No devices placeholder
            Rectangle {
                Layout.fillWidth: true
                Layout.preferredHeight: 120
                radius: Units.radiusMD
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

            // Device rows as a Repeater of per-row FocusScopes (not a ListView):
            // each row becomes its own activeFocusItem so SettingsPanel's outer
            // contentFlick scrolls the WHOLE page to follow focus down the rows
            // (the Focus Preferences section slides up out of view). A ListView
            // holds focus as one tall item, so moving currentIndex never changed
            // Window.activeFocusItem and the page never scrolled per-row.
            ColumnLayout {
                Layout.fillWidth: true
                spacing: 16
                visible: root.devices.length > 0

                Repeater {
                    id: deviceRepeater
                    model: root.devices

                    delegate: FocusScope {
                        id: deviceRow
                        required property int index
                        required property var modelData
                        Layout.fillWidth: true
                        implicitHeight: 160
                        activeFocusOnTab: true

                        // Measure the longer "Set as default" label so every row's
                        // action button shares one stable width — otherwise the
                        // default row's narrower "Default ✓" button would jut left
                        // out of the right-aligned column.
                        TextMetrics {
                            id: defaultBtnMetrics
                            font.pixelSize: Theme.fontBody
                            text: "Set as default"
                        }

                        // First row goes back up to the Wake-AV action button
                        // (matches the old deviceListView.KeyNavigation.up).
                        // itemAt() can be transiently null during model rebuilds
                        // (CEC rescans reassign root.devices) — acceptable, the
                        // KeyNavigation binding re-evaluates.
                        KeyNavigation.up: index > 0 ? deviceRepeater.itemAt(index - 1) : wakeScope
                        KeyNavigation.down: index < deviceRepeater.count - 1 ? deviceRepeater.itemAt(index + 1) : null

                        // Controller path for "Set as default": Return/Enter on the
                        // focused row toggles that device as the preferred default
                        // input — re-selecting the current default clears it (#16).
                        Keys.onReturnPressed: {
                            var a = modelData.logicalAddress;
                            SettingsStore.setCecDefaultInput(a === SettingsStore.cecDefaultInput ? -1 : a);
                        }
                        Keys.onEnterPressed: {
                            var a = modelData.logicalAddress;
                            SettingsStore.setCecDefaultInput(a === SettingsStore.cecDefaultInput ? -1 : a);
                        }

                        Rectangle {
                            anchors.fill: parent
                            radius: Units.radiusMD
                            color: deviceRow.activeFocus ? Theme.surfaceHover : Theme.surface
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

                                // Device name + power badge + address detail.
                                ColumnLayout {
                                    spacing: 8
                                    Layout.fillWidth: true

                                    RowLayout {
                                        spacing: 16

                                        Text {
                                            // cec-rs 12.0.1 exposes no OSD name; derive a
                                            // friendly label from the logical address.
                                            text: root.nameForAddress(deviceRow.modelData.logicalAddress)
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
                                                if (deviceRow.modelData.powerStatus === "on")
                                                    return Qt.rgba(0.176, 0.541, 0.306, 0.2);
                                                if (deviceRow.modelData.powerStatus === "standby")
                                                    return Qt.rgba(0.843, 0.651, 0.294, 0.2);
                                                return Theme.surfaceHover;
                                            }

                                            Text {
                                                id: powerLabel
                                                anchors.centerIn: parent
                                                text: {
                                                    if (deviceRow.modelData.powerStatus === "on")
                                                        return "On";
                                                    if (deviceRow.modelData.powerStatus === "standby")
                                                        return "Standby";
                                                    if (deviceRow.modelData.powerStatus === "waking")
                                                        return "Waking";
                                                    if (deviceRow.modelData.powerStatus === "sleeping")
                                                        return "Sleeping";
                                                    return "Unknown";
                                                }
                                                font.pixelSize: Theme.fontCaption
                                                color: {
                                                    if (deviceRow.modelData.powerStatus === "on")
                                                        return Theme.online;
                                                    if (deviceRow.modelData.powerStatus === "standby")
                                                        return Theme.gold;
                                                    return Theme.textSecondary;
                                                }
                                            }
                                        }

                                        // Default-input badge — shown when this device is
                                        // the persisted preferred default (#16). Reuses the
                                        // power-badge styling (crimson/sidebarActive tint).
                                        Rectangle {
                                            visible: deviceRow.modelData.logicalAddress === SettingsStore.cecDefaultInput
                                            width: defaultLabel.implicitWidth + 24
                                            height: 40
                                            radius: 20
                                            color: Theme.sidebarActive

                                            Text {
                                                id: defaultLabel
                                                anchors.centerIn: parent
                                                text: "Default"
                                                font.pixelSize: Theme.fontCaption
                                                color: Theme.textOnDark
                                            }
                                        }
                                    }

                                    Text {
                                        text: "Logical address: " + deviceRow.modelData.logicalAddress
                                        font.pixelSize: Theme.fontSmall
                                        color: Theme.textSecondary
                                    }
                                }

                                // Set-as-default action — persists the preference (Phase 1
                                // is persist-only; daemon behaviour wiring is a follow-up).
                                // z lifts it above the row-selection MouseArea so its own
                                // click handler wins; Return on the focused row is the
                                // controller path.
                                SettingsButton {
                                    z: 1
                                    Layout.alignment: Qt.AlignRight | Qt.AlignVCenter
                                    Layout.preferredWidth: defaultBtnMetrics.width + 80
                                    text: deviceRow.modelData.logicalAddress === SettingsStore.cecDefaultInput ? "Default ✓" : "Set as default"
                                    highlighted: deviceRow.modelData.logicalAddress === SettingsStore.cecDefaultInput
                                    onActivated: SettingsStore.setCecDefaultInput(deviceRow.modelData.logicalAddress === SettingsStore.cecDefaultInput ? -1 : deviceRow.modelData.logicalAddress)
                                }
                            }

                            MouseArea {
                                anchors.fill: parent
                                hoverEnabled: true
                                cursorShape: Qt.PointingHandCursor
                                onClicked: {
                                    deviceRow.forceActiveFocus();
                                }
                            }
                        }
                    }
                }
            }
        }

        // Hint bar
        HintBar {
            text: root.cecAvailable ? "A: Set as default input  |  Auto-refresh every 30s" : "HDMI-CEC unavailable — daemon reports no CEC adapter"
        }
    }
}
