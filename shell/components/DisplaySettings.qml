import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import Quickshell.Io
import "lib"

FocusScope {
    id: root
    implicitHeight: contentColumn.implicitHeight + 2 * Theme.padding

    property var monitors: []
    property int selectedMonitor: 0

    // --- Daemon read: hypr-monitors (replaces the hyprctl monitors -j Process) ---

    SocketClient {
        id: getMonitors
        onResponseReceived: line => {
            try {
                let data = JSON.parse(line);
                let mons = [];
                for (let i = 0; i < data.length; i++) {
                    let m = data[i];
                    mons.push({
                        name: m.name || "Unknown",
                        description: m.description || "",
                        width: m.width || 0,
                        height: m.height || 0,
                        refreshRate: m.refreshRate || 0,
                        scale: m.scale || 1.0,
                        x: m.x || 0,
                        y: m.y || 0,
                        activeWorkspace: m.activeWorkspace || "",
                        dpmsStatus: m.dpmsStatus !== false,
                        vrr: m.vrr || false,
                        availableModes: m.availableModes || [],
                        currentFormat: m.currentFormat || "",
                        hdr: m.hdr || false
                    });
                }
                root.monitors = mons;
            } catch (e) {
                console.log("Failed to parse monitor data:", e);
            }
        }
    }

    // --- One-shot APPLY paths (hyprctl keyword monitor) ---

    Process {
        id: setScale
        property string monName: ""
        property real scaleVal: 1.0
        command: ["hyprctl", "keyword", "monitor", monName + ",preferred,auto," + scaleVal]
        onExited: {
            getMonitors.request("hypr-monitors");
        }
    }

    Process {
        id: setMode
        property string monName: ""
        property string mode: ""
        command: ["hyprctl", "keyword", "monitor", monName + "," + mode + ",auto,1"]
        onExited: {
            getMonitors.request("hypr-monitors");
        }
    }

    // Apply HDR on: include bitdepth,10,cm,hdr,sdrbrightness,sdrsaturation
    // Apply HDR off: standard mode line only
    Process {
        id: setHdr
        property string monKeyword: ""
        command: ["hyprctl", "keyword", "monitor", monKeyword]
        onExited: {
            getMonitors.request("hypr-monitors");
        }
    }

    // Apply a specific refresh rate (keeps current resolution + position)
    Process {
        id: setRefresh
        property string monKeyword: ""
        command: ["hyprctl", "keyword", "monitor", monKeyword]
        onExited: {
            getMonitors.request("hypr-monitors");
        }
    }

    // Night-light: enable via hyprsunset -t <temp>, disable via pkill hyprsunset
    Process {
        id: applyNightLight
        property var pendingCmd: []
        command: pendingCmd
    }

    function applyHdr(enabled) {
        if (root.monitors.length <= root.selectedMonitor)
            return;
        let mon = root.monitors[root.selectedMonitor];
        let name = mon.name;
        let w = mon.width;
        let h = mon.height;
        let rr = mon.refreshRate.toFixed(6);
        let scale = mon.scale.toFixed(1);
        let x = mon.x;
        let y = mon.y;
        let keyword;
        if (enabled) {
            keyword = name + "," + w + "x" + h + "@" + rr + "," + x + "x" + y + "," + scale + ",vrr,1,bitdepth,10,cm,hdr,sdrbrightness,1.2,sdrsaturation,1.05";
        } else {
            keyword = name + "," + w + "x" + h + "@" + rr + "," + x + "x" + y + "," + scale;
        }
        setHdr.monKeyword = keyword;
        setHdr.running = true;
    }

    function applyRefreshRate(hz) {
        if (root.monitors.length <= root.selectedMonitor)
            return;
        let mon = root.monitors[root.selectedMonitor];
        let name = mon.name;
        let w = mon.width;
        let h = mon.height;
        let x = mon.x;
        let y = mon.y;
        let scale = mon.scale.toFixed(1);
        let keyword = name + "," + w + "x" + h + "@" + hz + "," + x + "x" + y + "," + scale;
        setRefresh.monKeyword = keyword;
        setRefresh.running = true;
    }

    function applyNightLightSetting(enabled, temp) {
        if (enabled) {
            applyNightLight.pendingCmd = ["hyprsunset", "-t", temp.toString()];
        } else {
            applyNightLight.pendingCmd = ["pkill", "hyprsunset"];
        }
        applyNightLight.running = true;
    }

    Component.onCompleted: {
        getMonitors.request("hypr-monitors");
    }

    onVisibleChanged: {
        if (visible) {
            getMonitors.request("hypr-monitors");
        }
    }

    function focusFirst() {
        monitorList.forceActiveFocus();
    }

    ColumnLayout {
        id: contentColumn
        anchors.fill: parent
        anchors.margins: Theme.padding
        spacing: 32

        Text {
            text: "Displays"
            font.pixelSize: Theme.fontBody
            font.bold: true
            color: Theme.textPrimary
        }

        // HDR live-state status line (read-only, driven by daemon-read hdr field)
        Text {
            visible: root.monitors.length > root.selectedMonitor
            text: {
                if (root.monitors.length <= root.selectedMonitor)
                    return "";
                let mon = root.monitors[root.selectedMonitor];
                return "HDR: " + (mon.hdr ? "Active (" + mon.currentFormat + ")" : "Inactive");
            }
            font.pixelSize: Theme.fontSmall
            color: Theme.textSecondary
        }

        // Monitor list
        SettingsList {
            id: monitorList
            // rowStride:200 preserves the original Math.min(count*200,400) formula
            // exactly; true geometric stride is delegate 180 + spacing 16 = 196.
            rowStride: 200
            maxHeight: 400
            spacing: 16
            model: root.monitors
            focus: true

            KeyNavigation.down: hdrToggleScope

            delegate: Rectangle {
                required property int index
                required property var modelData
                width: monitorList.width
                height: 180
                radius: 16
                color: monitorList.currentIndex === index && monitorList.activeFocus ? Theme.surfaceHover : Theme.surface
                border.width: 2
                border.color: Theme.surfaceBorder

                Behavior on color {
                    ColorAnimation {
                        duration: 150
                    }
                }

                property bool isSel: monitorList.currentIndex === index && monitorList.activeFocus

                ColumnLayout {
                    anchors.fill: parent
                    anchors.leftMargin: 24
                    anchors.rightMargin: 24
                    anchors.topMargin: 20
                    anchors.bottomMargin: 20
                    spacing: 8

                    RowLayout {
                        Layout.fillWidth: true
                        spacing: 16

                        Text {
                            text: modelData.name
                            font.pixelSize: Theme.fontBody
                            font.bold: true
                            color: Theme.textPrimary
                        }

                        Text {
                            text: modelData.description
                            font.pixelSize: Theme.fontSmall
                            color: Theme.textSecondary
                            elide: Text.ElideRight
                            Layout.fillWidth: true
                        }
                    }

                    RowLayout {
                        spacing: 32

                        Text {
                            text: "Resolution: " + modelData.width + "x" + modelData.height
                            font.pixelSize: Theme.fontSmall
                            color: Theme.textSecondary
                        }

                        Text {
                            text: "Refresh: " + modelData.refreshRate.toFixed(1) + " Hz"
                            font.pixelSize: Theme.fontSmall
                            color: Theme.textSecondary
                        }

                        Text {
                            text: "Scale: " + modelData.scale.toFixed(1) + "x"
                            font.pixelSize: Theme.fontSmall
                            color: Theme.textSecondary
                        }
                    }

                    RowLayout {
                        spacing: 32

                        Text {
                            text: "Position: " + modelData.x + "," + modelData.y
                            font.pixelSize: Theme.fontSmall
                            color: Theme.textSecondary
                        }

                        Text {
                            text: "DPMS: " + (modelData.dpmsStatus ? "On" : "Off")
                            font.pixelSize: Theme.fontSmall
                            color: Theme.textSecondary
                        }

                        Text {
                            text: "VRR: " + (modelData.vrr ? "On" : "Off")
                            font.pixelSize: Theme.fontSmall
                            color: Theme.textSecondary
                        }
                    }
                }

                MouseArea {
                    anchors.fill: parent
                    hoverEnabled: true
                    cursorShape: Qt.PointingHandCursor
                    onClicked: {
                        monitorList.currentIndex = index;
                        monitorList.forceActiveFocus();
                        root.selectedMonitor = index;
                    }
                }
            }

            Keys.onReturnPressed: {
                root.selectedMonitor = currentIndex;
                hdrToggleScope.forceActiveFocus();
            }
        }

        // HDR toggle
        Text {
            text: "HDR"
            font.pixelSize: Theme.fontBody
            font.bold: true
            color: Theme.textPrimary
            visible: root.monitors.length > 0
        }

        FocusScope {
            id: hdrToggleScope
            Layout.fillWidth: true
            Layout.preferredHeight: 80
            visible: root.monitors.length > 0

            KeyNavigation.up: monitorList
            KeyNavigation.down: scaleRow

            Keys.onReturnPressed: {
                SettingsStore.setHdrEnabled(!SettingsStore.hdrEnabled);
                root.applyHdr(SettingsStore.hdrEnabled);
            }

            SettingsButton {
                id: hdrBtn
                width: 160
                height: 72
                text: SettingsStore.hdrEnabled ? "On" : "Off"
                color: SettingsStore.hdrEnabled ? Theme.sidebarActive : (hdrToggleScope.activeFocus ? Theme.surfaceHover : Theme.surface)
                border.width: hdrToggleScope.activeFocus ? 2 : 1
                border.color: hdrToggleScope.activeFocus ? Theme.focusBorder : Theme.surfaceBorder

                onActivated: {
                    SettingsStore.setHdrEnabled(!SettingsStore.hdrEnabled);
                    root.applyHdr(SettingsStore.hdrEnabled);
                }

                MouseArea {
                    anchors.fill: parent
                    hoverEnabled: true
                    cursorShape: Qt.PointingHandCursor
                    onClicked: {
                        hdrToggleScope.forceActiveFocus();
                        hdrBtn.activated();
                    }
                }
            }
        }

        // Scale controls
        Text {
            text: "Scale"
            font.pixelSize: Theme.fontBody
            font.bold: true
            color: Theme.textPrimary
            visible: root.monitors.length > 0
        }

        SettingsButtonGroup {
            id: scaleRow
            visible: root.monitors.length > 0
            options: [
                {
                    label: "0.5x",
                    value: 0.5
                },
                {
                    label: "1x",
                    value: 1.0
                },
                {
                    label: "1.25x",
                    value: 1.25
                },
                {
                    label: "1.5x",
                    value: 1.5
                },
                {
                    label: "1.75x",
                    value: 1.75
                },
                {
                    label: "2x",
                    value: 2.0
                }
            ]
            isCurrentOption: function (opt) {
                return root.monitors.length > root.selectedMonitor && Math.abs(root.monitors[root.selectedMonitor].scale - opt.value) < 0.05;
            }
            onValueSelected: function (opt) {
                if (root.monitors.length > root.selectedMonitor) {
                    setScale.monName = root.monitors[root.selectedMonitor].name;
                    setScale.scaleVal = opt.value;
                    setScale.running = true;
                }
            }

            KeyNavigation.up: hdrToggleScope
            KeyNavigation.down: modeDropdownScope
        }

        // Resolution / Mode dropdown
        Text {
            text: "Resolution"
            font.pixelSize: Theme.fontBody
            font.bold: true
            color: Theme.textPrimary
            visible: root.monitors.length > 0
        }

        SettingsDropdown {
            id: modeDropdownScope
            visible: root.monitors.length > 0
            maxHeight: 600

            property var allModes: root.monitors.length > root.selectedMonitor ? root.monitors[root.selectedMonitor].availableModes : []
            property var modes: {
                let seen = {};
                let result = [];
                for (let i = 0; i < allModes.length; i++) {
                    let m = allModes[i];
                    let res = m.split("@")[0];
                    if (!seen[res]) {
                        seen[res] = true;
                        result.push(m);
                    }
                }
                return result;
            }
            property string currentMode: {
                if (root.monitors.length <= root.selectedMonitor)
                    return "";
                let mon = root.monitors[root.selectedMonitor];
                return mon.width + "x" + mon.height + "@" + mon.refreshRate.toFixed(6);
            }

            function formatMode(mode) {
                let parts = mode.split("@");
                let res = parts[0] || mode;
                let hz = parts.length > 1 ? parseFloat(parts[1]).toFixed(2) + " Hz" : "";
                return res + (hz ? "  @  " + hz : "");
            }

            model: modes
            displayText: formatMode(currentMode)
            isCurrentItem: function (item) {
                if (root.monitors.length <= root.selectedMonitor)
                    return false;
                let mon = root.monitors[root.selectedMonitor];
                let res = item.split("@")[0];
                return res === (mon.width + "x" + mon.height);
            }
            itemLabel: function (item) {
                return formatMode(item);
            }
            onItemSelected: function (item) {
                if (root.monitors.length > root.selectedMonitor) {
                    setMode.monName = root.monitors[root.selectedMonitor].name;
                    setMode.mode = item;
                    setMode.running = true;
                }
            }

            KeyNavigation.up: scaleRow
            KeyNavigation.down: refreshDropdownScope
        }

        // Refresh Rate dropdown (separate from resolution)
        Text {
            text: "Refresh Rate"
            font.pixelSize: Theme.fontBody
            font.bold: true
            color: Theme.textPrimary
            visible: root.monitors.length > 0
        }

        SettingsDropdown {
            id: refreshDropdownScope
            visible: root.monitors.length > 0

            property var refreshRates: {
                if (root.monitors.length <= root.selectedMonitor)
                    return [];
                let mon = root.monitors[root.selectedMonitor];
                let currentRes = mon.width + "x" + mon.height;
                let seen = {};
                let result = [];
                for (let i = 0; i < mon.availableModes.length; i++) {
                    let m = mon.availableModes[i];
                    let parts = m.split("@");
                    if (parts.length < 2)
                        continue;
                    let res = parts[0];
                    if (res !== currentRes)
                        continue;
                    let hz = parseFloat(parts[1]).toFixed(2);
                    if (!seen[hz]) {
                        seen[hz] = true;
                        result.push({
                            hz: hz,
                            mode: m
                        });
                    }
                }
                return result;
            }
            property real currentHz: root.monitors.length > root.selectedMonitor ? root.monitors[root.selectedMonitor].refreshRate : 0

            model: refreshRates
            displayText: currentHz.toFixed(2) + " Hz"
            isCurrentItem: function (item) {
                return Math.abs(parseFloat(item.hz) - currentHz) < 0.5;
            }
            itemLabel: function (item) {
                return item.hz + " Hz";
            }
            onItemSelected: function (item) {
                root.applyRefreshRate(item.hz);
            }

            KeyNavigation.up: modeDropdownScope
            KeyNavigation.down: nightLightToggleScope
        }

        // Night-light / Color temperature
        Text {
            text: "Night Light"
            font.pixelSize: Theme.fontBody
            font.bold: true
            color: Theme.textPrimary
        }

        FocusScope {
            id: nightLightToggleScope
            Layout.fillWidth: true
            Layout.preferredHeight: 80

            KeyNavigation.up: refreshDropdownScope
            KeyNavigation.down: nightLightTempScope

            Keys.onReturnPressed: {
                SettingsStore.setNightLightEnabled(!SettingsStore.nightLightEnabled);
                root.applyNightLightSetting(SettingsStore.nightLightEnabled, SettingsStore.nightLightTemp);
            }

            SettingsButton {
                id: nightLightBtn
                width: 160
                height: 72
                text: SettingsStore.nightLightEnabled ? "On" : "Off"
                color: SettingsStore.nightLightEnabled ? Theme.sidebarActive : (nightLightToggleScope.activeFocus ? Theme.surfaceHover : Theme.surface)
                border.width: nightLightToggleScope.activeFocus ? 2 : 1
                border.color: nightLightToggleScope.activeFocus ? Theme.focusBorder : Theme.surfaceBorder

                onActivated: {
                    SettingsStore.setNightLightEnabled(!SettingsStore.nightLightEnabled);
                    root.applyNightLightSetting(SettingsStore.nightLightEnabled, SettingsStore.nightLightTemp);
                }

                MouseArea {
                    anchors.fill: parent
                    hoverEnabled: true
                    cursorShape: Qt.PointingHandCursor
                    onClicked: {
                        nightLightToggleScope.forceActiveFocus();
                        nightLightBtn.activated();
                    }
                }
            }
        }

        // Night-light color temperature dropdown
        SettingsDropdown {
            id: nightLightTempScope

            property var tempPresets: [
                {
                    label: "Off (disable)",
                    value: 0
                },
                {
                    label: "Very Warm (3500K)",
                    value: 3500
                },
                {
                    label: "Warm (4000K)",
                    value: 4000
                },
                {
                    label: "Neutral (4500K)",
                    value: 4500
                },
                {
                    label: "Cool (5500K)",
                    value: 5500
                },
                {
                    label: "Daylight (6500K)",
                    value: 6500
                }
            ]

            model: tempPresets
            displayText: {
                let t = SettingsStore.nightLightTemp;
                for (let i = 0; i < tempPresets.length; i++) {
                    if (tempPresets[i].value === t)
                        return tempPresets[i].label;
                }
                return t + "K";
            }
            isCurrentItem: function (item) {
                return item.value === SettingsStore.nightLightTemp;
            }
            itemLabel: function (item) {
                return item.label;
            }
            onItemSelected: function (item) {
                if (item.value === 0) {
                    SettingsStore.setNightLightEnabled(false);
                    root.applyNightLightSetting(false, SettingsStore.nightLightTemp);
                } else {
                    SettingsStore.setNightLightTemp(item.value);
                    if (SettingsStore.nightLightEnabled)
                        root.applyNightLightSetting(true, item.value);
                }
            }

            KeyNavigation.up: nightLightToggleScope
            KeyNavigation.down: overscanScope
        }

        Text {
            text: "Night light requires hyprsunset to be installed."
            font.pixelSize: Theme.fontHint
            color: Theme.textMuted
        }

        // Overscan / safe-area
        Text {
            text: "Overscan"
            font.pixelSize: Theme.fontBody
            font.bold: true
            color: Theme.textPrimary
        }

        SettingsButtonGroup {
            id: overscanScope
            options: [
                {
                    label: "0%",
                    value: 0
                },
                {
                    label: "2%",
                    value: 2
                },
                {
                    label: "4%",
                    value: 4
                },
                {
                    label: "6%",
                    value: 6
                },
                {
                    label: "8%",
                    value: 8
                },
                {
                    label: "10%",
                    value: 10
                }
            ]
            isCurrentOption: function (opt) {
                return SettingsStore.overscan === opt.value;
            }
            onValueSelected: function (opt) {
                SettingsStore.setOverscan(opt.value);
            }

            KeyNavigation.up: nightLightTempScope
            KeyNavigation.down: autoDimToggleScope
        }

        Text {
            text: "Overscan drives the shell safe-area margin (applied at next restart)."
            font.pixelSize: Theme.fontHint
            color: Theme.textMuted
        }

        // Auto-Dim (OLED burn-in protection, #143)
        Text {
            text: "Auto-Dim"
            font.pixelSize: Theme.fontBody
            font.bold: true
            color: Theme.textPrimary
        }

        // Enable / disable toggle
        FocusScope {
            id: autoDimToggleScope
            Layout.fillWidth: true
            Layout.preferredHeight: 80

            KeyNavigation.up: overscanScope
            // #143: skip the disabled delay row — jump straight to modeList when auto-dim is off
            KeyNavigation.down: SettingsStore.autoDimEnabled ? autoDimDelayScope : modeList

            Keys.onReturnPressed: {
                SettingsStore.setAutoDimEnabled(!SettingsStore.autoDimEnabled);
            }

            SettingsButton {
                id: autoDimBtn
                width: 160
                height: 72
                text: SettingsStore.autoDimEnabled ? "On" : "Off"
                color: SettingsStore.autoDimEnabled ? Theme.sidebarActive : (autoDimToggleScope.activeFocus ? Theme.surfaceHover : Theme.surface)
                border.width: autoDimToggleScope.activeFocus ? 2 : 1
                border.color: autoDimToggleScope.activeFocus ? Theme.focusBorder : Theme.surfaceBorder

                onActivated: SettingsStore.setAutoDimEnabled(!SettingsStore.autoDimEnabled)

                MouseArea {
                    anchors.fill: parent
                    hoverEnabled: true
                    cursorShape: Qt.PointingHandCursor
                    onClicked: {
                        autoDimToggleScope.forceActiveFocus();
                        autoDimBtn.activated();
                    }
                }
            }
        }

        // Delay selector — 1 / 2 / 5 / 10 minutes
        Text {
            text: "Dim Delay"
            font.pixelSize: Theme.fontBody
            font.bold: true
            color: Theme.textPrimary
            opacity: SettingsStore.autoDimEnabled ? 1.0 : 0.4
        }

        SettingsButtonGroup {
            id: autoDimDelayScope
            // #143: exclude from nav chain when disabled so D-pad focus cannot enter
            activeFocusOnTab: SettingsStore.autoDimEnabled
            enabled: SettingsStore.autoDimEnabled
            options: [
                {
                    label: "1 min",
                    value: 1
                },
                {
                    label: "2 min",
                    value: 2
                },
                {
                    label: "5 min",
                    value: 5
                },
                {
                    label: "10 min",
                    value: 10
                }
            ]
            isCurrentOption: function (opt) {
                return SettingsStore.autoDimDelayMinutes === opt.value;
            }
            onValueSelected: function (opt) {
                if (SettingsStore.autoDimEnabled)
                    SettingsStore.setAutoDimDelayMinutes(opt.value);
            }

            KeyNavigation.up: autoDimToggleScope
            KeyNavigation.down: modeList
        }

        Text {
            text: "Dims the display after inactivity. Any input restores full brightness immediately."
            font.pixelSize: Theme.fontHint
            color: Theme.textMuted
        }

        // Appearance — Theme Mode
        Text {
            text: "Appearance"
            font.pixelSize: Theme.fontBody
            font.bold: true
            color: Theme.textPrimary
        }

        RowLayout {
            id: modeList
            Layout.alignment: Qt.AlignHCenter
            spacing: 40
            focus: false

            property var modes: [
                {
                    id: "auto",
                    icon: "◐",
                    label: "Auto",
                    desc: "Follows time of day"
                },
                {
                    id: "light",
                    icon: "☀",
                    label: "Light",
                    desc: "Light background"
                },
                {
                    id: "dark",
                    icon: "☽",
                    label: "Dark",
                    desc: "OLED optimized"
                }
            ]

            property int currentIndex: {
                for (var i = 0; i < modeList.modes.length; i++) {
                    if (modeList.modes[i].id === Theme.themeMode)
                        return i;
                }
                return 0;
            }

            property int focusIndex: 0

            Keys.onLeftPressed: event => {
                if (focusIndex > 0)
                    focusIndex--;
                else
                    event.accepted = false;
            }
            Keys.onRightPressed: {
                if (focusIndex < modeList.modes.length - 1)
                    focusIndex++;
            }
            Keys.onReturnPressed: {
                Theme.setThemeMode(modeList.modes[focusIndex].id);
            }
            Keys.onUpPressed: {
                // #143: skip the disabled delay row — land on toggle when auto-dim is off
                if (SettingsStore.autoDimEnabled)
                    autoDimDelayScope.forceActiveFocus();
                else
                    autoDimToggleScope.forceActiveFocus();
            }

            Repeater {
                model: modeList.modes

                Rectangle {
                    required property var modelData
                    required property int index
                    width: 400
                    height: 280
                    radius: Theme.cardRadius
                    color: Theme.surface
                    clip: true
                    border.width: modeList.focusIndex === index && modeList.activeFocus ? 4 : 2
                    border.color: modeList.focusIndex === index && modeList.activeFocus ? Theme.focusBorder : Theme.surfaceBorder

                    Behavior on border.color {
                        ColorAnimation {
                            duration: 150
                        }
                    }

                    Rectangle {
                        anchors.fill: parent
                        radius: parent.radius
                        color: Theme.surfaceHover
                        visible: modeList.focusIndex === index && modeList.activeFocus
                    }

                    Rectangle {
                        anchors.top: parent.top
                        anchors.right: parent.right
                        anchors.margins: 16
                        width: appliedLabel.implicitWidth + 28
                        height: appliedLabel.implicitHeight + 14
                        radius: height / 2
                        color: Theme.online
                        visible: Theme.themeMode === modelData.id
                        z: 1

                        Text {
                            id: appliedLabel
                            anchors.centerIn: parent
                            text: "✓ Active"
                            font.pixelSize: Theme.fontCaption
                            font.bold: true
                            color: Theme.textOnDark
                        }
                    }

                    ColumnLayout {
                        anchors.fill: parent
                        anchors.margins: 32
                        spacing: 12

                        Item {
                            Layout.fillHeight: true
                        }

                        Text {
                            text: modelData.icon
                            font.pixelSize: Theme.fontTitle
                            color: Theme.themeMode === modelData.id ? Theme.online : Theme.textPrimary
                            Layout.alignment: Qt.AlignHCenter
                        }

                        Text {
                            text: modelData.label
                            font.pixelSize: Theme.fontBody
                            font.bold: true
                            color: Theme.textPrimary
                            Layout.alignment: Qt.AlignHCenter
                        }

                        Text {
                            text: modelData.desc
                            font.pixelSize: Theme.fontSmall
                            color: Theme.textSecondary
                            Layout.alignment: Qt.AlignHCenter
                            Layout.maximumWidth: parent.width
                            horizontalAlignment: Text.AlignHCenter
                            wrapMode: Text.WordWrap
                        }

                        Item {
                            Layout.fillHeight: true
                        }
                    }

                    MouseArea {
                        anchors.fill: parent
                        hoverEnabled: true
                        cursorShape: Qt.PointingHandCursor
                        onClicked: {
                            modeList.focusIndex = index;
                            modeList.forceActiveFocus();
                            Theme.setThemeMode(modelData.id);
                        }
                    }
                }
            }
        }

        Text {
            text: "Current: " + Theme.themeMode.charAt(0).toUpperCase() + Theme.themeMode.slice(1)
            font.pixelSize: Theme.fontHint
            color: Theme.textSecondary
            Layout.alignment: Qt.AlignHCenter
        }

        Text {
            text: "A: Open/apply  |  B: Close dropdown  |  HDR note: applies live via hyprctl"
            font.pixelSize: Theme.fontHint
            color: Theme.textSecondary
            Layout.alignment: Qt.AlignHCenter
        }
    }
}
