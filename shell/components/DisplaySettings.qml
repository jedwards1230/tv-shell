import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import Quickshell.Io

FocusScope {
    id: root

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
        contentFlick.contentY = 0;
        monitorList.forceActiveFocus();
    }

    Flickable {
        id: contentFlick
        anchors.fill: parent
        contentWidth: width
        contentHeight: contentColumn.implicitHeight
        interactive: false
        clip: true
        boundsBehavior: Flickable.StopAtBounds

        ScrollBar.vertical: ScrollBar {
            policy: contentFlick.contentHeight > contentFlick.height ? ScrollBar.AlwaysOn : ScrollBar.AlwaysOff
        }

        Behavior on contentY {
            NumberAnimation {
                duration: 150
                easing.type: Easing.OutCubic
            }
        }

        function ensureVisible(item) {
            if (!item)
                return;
            var p = item.mapToItem(contentFlick.contentItem, 0, 0);
            if (p.y < contentFlick.contentY)
                contentFlick.contentY = Math.max(0, p.y - 24);
            else if (p.y + item.height > contentFlick.contentY + contentFlick.height)
                contentFlick.contentY = Math.min(contentFlick.contentHeight - contentFlick.height, p.y + item.height - contentFlick.height + 24);
        }

        ColumnLayout {
            id: contentColumn
            width: contentFlick.width
            anchors.top: parent.top
            anchors.left: parent.left
            anchors.right: parent.right
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

                onActiveFocusChanged: if (activeFocus)
                    contentFlick.ensureVisible(this)

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

                onActiveFocusChanged: if (activeFocus)
                    contentFlick.ensureVisible(this)

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

            FocusScope {
                id: scaleRow
                Layout.fillWidth: true
                Layout.preferredHeight: 96
                visible: root.monitors.length > 0

                KeyNavigation.up: hdrToggleScope
                KeyNavigation.down: modeDropdownScope

                onActiveFocusChanged: if (activeFocus)
                    contentFlick.ensureVisible(this)

                property var scales: [0.5, 1.0, 1.25, 1.5, 1.75, 2.0]
                property int selectedScale: {
                    if (root.monitors.length <= root.selectedMonitor)
                        return 1;
                    let s = root.monitors[root.selectedMonitor].scale;
                    for (let i = 0; i < scales.length; i++) {
                        if (Math.abs(scales[i] - s) < 0.05)
                            return i;
                    }
                    return 1;
                }
                property int focusedIndex: selectedScale

                Keys.onLeftPressed: {
                    if (focusedIndex > 0)
                        focusedIndex--;
                }
                Keys.onRightPressed: {
                    if (focusedIndex < scales.length - 1)
                        focusedIndex++;
                }
                Keys.onReturnPressed: {
                    if (root.monitors.length > root.selectedMonitor) {
                        setScale.monName = root.monitors[root.selectedMonitor].name;
                        setScale.scaleVal = scales[focusedIndex];
                        setScale.running = true;
                    }
                }

                RowLayout {
                    anchors.fill: parent
                    spacing: 16

                    Repeater {
                        model: scaleRow.scales

                        FocusScope {
                            id: scaleScope
                            required property var modelData
                            required property int index
                            width: scaleBtn.width
                            height: scaleBtn.height

                            SettingsButton {
                                id: scaleBtn
                                text: parent.modelData + "x"
                                anchors.fill: parent

                                property bool isCurrent: root.monitors.length > root.selectedMonitor && Math.abs(root.monitors[root.selectedMonitor].scale - parent.modelData) < 0.05
                                property bool isFocused: scaleRow.activeFocus && scaleRow.focusedIndex === scaleScope.index

                                color: isCurrent ? Theme.sidebarActive : isFocused ? Theme.surfaceHover : Theme.surface
                                border.width: isFocused ? 2 : 1
                                border.color: isFocused ? Theme.focusBorder : Theme.surfaceBorder

                                // Applies this card's own scale — covers AT-SPI press and
                                // mouse click. Directional focus + Return is owned by
                                // scaleRow (focusedIndex), which drives the keyboard path.
                                onActivated: {
                                    if (root.monitors.length > root.selectedMonitor) {
                                        setScale.monName = root.monitors[root.selectedMonitor].name;
                                        setScale.scaleVal = scaleScope.modelData;
                                        setScale.running = true;
                                    }
                                }

                                MouseArea {
                                    anchors.fill: parent
                                    hoverEnabled: true
                                    cursorShape: Qt.PointingHandCursor
                                    onClicked: {
                                        scaleRow.forceActiveFocus();
                                        scaleRow.focusedIndex = scaleScope.index;
                                        scaleBtn.activated();
                                    }
                                }
                            }
                        }
                    }
                }
            }

            // Resolution / Mode dropdown
            Text {
                text: "Resolution"
                font.pixelSize: Theme.fontBody
                font.bold: true
                color: Theme.textPrimary
                visible: root.monitors.length > 0
            }

            FocusScope {
                id: modeDropdownScope
                Layout.fillWidth: true
                Layout.preferredHeight: modeDropdownOpen ? Math.min(modeDropdownList.count * 72 + 80, 600) : 80
                visible: root.monitors.length > 0
                focus: false

                property bool modeDropdownOpen: false
                property var allModes: root.monitors.length > root.selectedMonitor ? root.monitors[root.selectedMonitor].availableModes : []
                // Resolution dropdown shows all unique resolutions (WxH part)
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

                KeyNavigation.up: scaleRow
                KeyNavigation.down: refreshDropdownScope

                onActiveFocusChanged: if (activeFocus)
                    contentFlick.ensureVisible(this)

                Behavior on Layout.preferredHeight {
                    NumberAnimation {
                        duration: 200
                        easing.type: Easing.OutCubic
                    }
                }

                Rectangle {
                    id: modeDropdownHeader
                    width: parent.width
                    height: 80
                    radius: 16
                    color: modeDropdownScope.activeFocus && !modeDropdownScope.modeDropdownOpen ? Theme.surfaceHover : Theme.surface
                    border.width: 2
                    border.color: Theme.surfaceBorder

                    Behavior on color {
                        ColorAnimation {
                            duration: 150
                        }
                    }

                    RowLayout {
                        anchors.fill: parent
                        anchors.leftMargin: 24
                        anchors.rightMargin: 24
                        spacing: 16

                        Text {
                            text: modeDropdownScope.formatMode(modeDropdownScope.currentMode)
                            font.pixelSize: Theme.fontSmall
                            color: Theme.textPrimary
                            Layout.fillWidth: true
                        }

                        Text {
                            text: "(current)"
                            font.pixelSize: Theme.fontHint
                            color: Theme.textMuted
                        }

                        Text {
                            text: modeDropdownScope.modeDropdownOpen ? "▲" : "▼"
                            font.pixelSize: Theme.fontSmall
                            color: Theme.textSecondary
                        }
                    }

                    MouseArea {
                        anchors.fill: parent
                        cursorShape: Qt.PointingHandCursor
                        onClicked: {
                            modeDropdownScope.forceActiveFocus();
                            modeDropdownScope.modeDropdownOpen = !modeDropdownScope.modeDropdownOpen;
                        }
                    }
                }

                ListView {
                    id: modeDropdownList
                    anchors.top: modeDropdownHeader.bottom
                    anchors.topMargin: 8
                    width: parent.width
                    height: parent.height - modeDropdownHeader.height - 8
                    spacing: 4
                    clip: true
                    visible: modeDropdownScope.modeDropdownOpen
                    model: modeDropdownScope.modes
                    keyNavigationEnabled: true
                    highlightFollowsCurrentItem: true
                    highlightMoveDuration: 100

                    delegate: Rectangle {
                        required property int index
                        required property var modelData
                        width: modeDropdownList.width
                        height: 68
                        radius: 12

                        property bool isCurrent: {
                            if (root.monitors.length <= root.selectedMonitor)
                                return false;
                            let mon = root.monitors[root.selectedMonitor];
                            let res = modelData.split("@")[0];
                            return res === (mon.width + "x" + mon.height);
                        }

                        color: {
                            if (isCurrent)
                                return Theme.sidebarActive;
                            if (modeDropdownList.currentIndex === index && modeDropdownList.activeFocus)
                                return Theme.surfaceHover;
                            return Theme.cardBackground;
                        }
                        border.width: isCurrent ? 2 : 1
                        border.color: isCurrent ? Theme.focusBorder : Theme.surfaceBorder

                        Behavior on color {
                            ColorAnimation {
                                duration: 150
                            }
                        }

                        Text {
                            anchors.centerIn: parent
                            text: modeDropdownScope.formatMode(modelData) + (isCurrent ? "  (current)" : "")
                            font.pixelSize: Theme.fontSmall
                            color: isCurrent ? Theme.textOnDark : Theme.textPrimary
                        }

                        MouseArea {
                            anchors.fill: parent
                            cursorShape: Qt.PointingHandCursor
                            onClicked: {
                                modeDropdownList.currentIndex = index;
                                modeDropdownList.forceActiveFocus();
                            }
                            onDoubleClicked: {
                                if (root.monitors.length > root.selectedMonitor) {
                                    setMode.monName = root.monitors[root.selectedMonitor].name;
                                    setMode.mode = modelData;
                                    setMode.running = true;
                                    modeDropdownScope.modeDropdownOpen = false;
                                }
                            }
                        }
                    }

                    Keys.onReturnPressed: {
                        if (currentIndex >= 0 && root.monitors.length > root.selectedMonitor) {
                            let modes = modeDropdownScope.modes;
                            if (currentIndex < modes.length) {
                                setMode.monName = root.monitors[root.selectedMonitor].name;
                                setMode.mode = modes[currentIndex];
                                setMode.running = true;
                                modeDropdownScope.modeDropdownOpen = false;
                            }
                        }
                    }

                    Keys.onEscapePressed: {
                        modeDropdownScope.modeDropdownOpen = false;
                        modeDropdownScope.forceActiveFocus();
                    }
                }

                Keys.onReturnPressed: {
                    if (!modeDropdownOpen) {
                        modeDropdownOpen = true;
                        modeDropdownList.forceActiveFocus();
                    }
                }

                Keys.onEscapePressed: {
                    if (modeDropdownOpen) {
                        modeDropdownOpen = false;
                    } else {
                        event.accepted = false;
                    }
                }
            }

            // Refresh Rate dropdown (separate from resolution)
            Text {
                text: "Refresh Rate"
                font.pixelSize: Theme.fontBody
                font.bold: true
                color: Theme.textPrimary
                visible: root.monitors.length > 0
            }

            FocusScope {
                id: refreshDropdownScope
                Layout.fillWidth: true
                Layout.preferredHeight: refreshDropdownOpen ? Math.min(refreshDropdownList.count * 72 + 80, 400) : 80
                visible: root.monitors.length > 0
                focus: false

                property bool refreshDropdownOpen: false
                // Filter availableModes to those matching current resolution (WxH)
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

                KeyNavigation.up: modeDropdownScope
                KeyNavigation.down: nightLightToggleScope

                onActiveFocusChanged: if (activeFocus)
                    contentFlick.ensureVisible(this)

                Behavior on Layout.preferredHeight {
                    NumberAnimation {
                        duration: 200
                        easing.type: Easing.OutCubic
                    }
                }

                Rectangle {
                    id: refreshDropdownHeader
                    width: parent.width
                    height: 80
                    radius: 16
                    color: refreshDropdownScope.activeFocus && !refreshDropdownScope.refreshDropdownOpen ? Theme.surfaceHover : Theme.surface
                    border.width: 2
                    border.color: Theme.surfaceBorder

                    Behavior on color {
                        ColorAnimation {
                            duration: 150
                        }
                    }

                    RowLayout {
                        anchors.fill: parent
                        anchors.leftMargin: 24
                        anchors.rightMargin: 24
                        spacing: 16

                        Text {
                            text: refreshDropdownScope.currentHz.toFixed(2) + " Hz"
                            font.pixelSize: Theme.fontSmall
                            color: Theme.textPrimary
                            Layout.fillWidth: true
                        }

                        Text {
                            text: "(current)"
                            font.pixelSize: Theme.fontHint
                            color: Theme.textMuted
                        }

                        Text {
                            text: refreshDropdownScope.refreshDropdownOpen ? "▲" : "▼"
                            font.pixelSize: Theme.fontSmall
                            color: Theme.textSecondary
                        }
                    }

                    MouseArea {
                        anchors.fill: parent
                        cursorShape: Qt.PointingHandCursor
                        onClicked: {
                            refreshDropdownScope.forceActiveFocus();
                            refreshDropdownScope.refreshDropdownOpen = !refreshDropdownScope.refreshDropdownOpen;
                        }
                    }
                }

                ListView {
                    id: refreshDropdownList
                    anchors.top: refreshDropdownHeader.bottom
                    anchors.topMargin: 8
                    width: parent.width
                    height: parent.height - refreshDropdownHeader.height - 8
                    spacing: 4
                    clip: true
                    visible: refreshDropdownScope.refreshDropdownOpen
                    model: refreshDropdownScope.refreshRates
                    keyNavigationEnabled: true
                    highlightFollowsCurrentItem: true
                    highlightMoveDuration: 100

                    delegate: Rectangle {
                        required property int index
                        required property var modelData
                        width: refreshDropdownList.width
                        height: 68
                        radius: 12

                        property bool isCurrent: Math.abs(parseFloat(modelData.hz) - refreshDropdownScope.currentHz) < 0.5

                        color: {
                            if (isCurrent)
                                return Theme.sidebarActive;
                            if (refreshDropdownList.currentIndex === index && refreshDropdownList.activeFocus)
                                return Theme.surfaceHover;
                            return Theme.cardBackground;
                        }
                        border.width: isCurrent ? 2 : 1
                        border.color: isCurrent ? Theme.focusBorder : Theme.surfaceBorder

                        Behavior on color {
                            ColorAnimation {
                                duration: 150
                            }
                        }

                        Text {
                            anchors.centerIn: parent
                            text: modelData.hz + " Hz" + (isCurrent ? "  (current)" : "")
                            font.pixelSize: Theme.fontSmall
                            color: isCurrent ? Theme.textOnDark : Theme.textPrimary
                        }

                        MouseArea {
                            anchors.fill: parent
                            cursorShape: Qt.PointingHandCursor
                            onClicked: {
                                refreshDropdownList.currentIndex = index;
                                refreshDropdownList.forceActiveFocus();
                            }
                            onDoubleClicked: {
                                root.applyRefreshRate(modelData.hz);
                                refreshDropdownScope.refreshDropdownOpen = false;
                            }
                        }
                    }

                    Keys.onReturnPressed: {
                        if (currentIndex >= 0) {
                            let rates = refreshDropdownScope.refreshRates;
                            if (currentIndex < rates.length) {
                                root.applyRefreshRate(rates[currentIndex].hz);
                                refreshDropdownScope.refreshDropdownOpen = false;
                            }
                        }
                    }

                    Keys.onEscapePressed: {
                        refreshDropdownScope.refreshDropdownOpen = false;
                        refreshDropdownScope.forceActiveFocus();
                    }
                }

                Keys.onReturnPressed: {
                    if (!refreshDropdownOpen) {
                        refreshDropdownOpen = true;
                        refreshDropdownList.forceActiveFocus();
                    }
                }

                Keys.onEscapePressed: {
                    if (refreshDropdownOpen) {
                        refreshDropdownOpen = false;
                    } else {
                        event.accepted = false;
                    }
                }
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

                onActiveFocusChanged: if (activeFocus)
                    contentFlick.ensureVisible(this)

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
            FocusScope {
                id: nightLightTempScope
                Layout.fillWidth: true
                Layout.preferredHeight: nightLightTempOpen ? Math.min(nightLightTempList.count * 72 + 80, 400) : 80

                property bool nightLightTempOpen: false
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

                KeyNavigation.up: nightLightToggleScope
                KeyNavigation.down: overscanScope

                onActiveFocusChanged: if (activeFocus)
                    contentFlick.ensureVisible(this)

                Behavior on Layout.preferredHeight {
                    NumberAnimation {
                        duration: 200
                        easing.type: Easing.OutCubic
                    }
                }

                Rectangle {
                    id: nightLightTempHeader
                    width: parent.width
                    height: 80
                    radius: 16
                    color: nightLightTempScope.activeFocus && !nightLightTempScope.nightLightTempOpen ? Theme.surfaceHover : Theme.surface
                    border.width: 2
                    border.color: Theme.surfaceBorder

                    Behavior on color {
                        ColorAnimation {
                            duration: 150
                        }
                    }

                    RowLayout {
                        anchors.fill: parent
                        anchors.leftMargin: 24
                        anchors.rightMargin: 24
                        spacing: 16

                        Text {
                            text: {
                                let t = SettingsStore.nightLightTemp;
                                for (let i = 0; i < nightLightTempScope.tempPresets.length; i++) {
                                    if (nightLightTempScope.tempPresets[i].value === t)
                                        return nightLightTempScope.tempPresets[i].label;
                                }
                                return t + "K";
                            }
                            font.pixelSize: Theme.fontSmall
                            color: Theme.textPrimary
                            Layout.fillWidth: true
                        }

                        Text {
                            text: nightLightTempScope.nightLightTempOpen ? "▲" : "▼"
                            font.pixelSize: Theme.fontSmall
                            color: Theme.textSecondary
                        }
                    }

                    MouseArea {
                        anchors.fill: parent
                        cursorShape: Qt.PointingHandCursor
                        onClicked: {
                            nightLightTempScope.forceActiveFocus();
                            nightLightTempScope.nightLightTempOpen = !nightLightTempScope.nightLightTempOpen;
                        }
                    }
                }

                ListView {
                    id: nightLightTempList
                    anchors.top: nightLightTempHeader.bottom
                    anchors.topMargin: 8
                    width: parent.width
                    height: parent.height - nightLightTempHeader.height - 8
                    spacing: 4
                    clip: true
                    visible: nightLightTempScope.nightLightTempOpen
                    model: nightLightTempScope.tempPresets
                    keyNavigationEnabled: true
                    highlightFollowsCurrentItem: true
                    highlightMoveDuration: 100

                    delegate: Rectangle {
                        required property int index
                        required property var modelData
                        width: nightLightTempList.width
                        height: 68
                        radius: 12

                        property bool isCurrent: modelData.value === SettingsStore.nightLightTemp

                        color: {
                            if (isCurrent)
                                return Theme.sidebarActive;
                            if (nightLightTempList.currentIndex === index && nightLightTempList.activeFocus)
                                return Theme.surfaceHover;
                            return Theme.cardBackground;
                        }
                        border.width: isCurrent ? 2 : 1
                        border.color: isCurrent ? Theme.focusBorder : Theme.surfaceBorder

                        Behavior on color {
                            ColorAnimation {
                                duration: 150
                            }
                        }

                        Text {
                            anchors.centerIn: parent
                            text: modelData.label + (isCurrent ? "  (current)" : "")
                            font.pixelSize: Theme.fontSmall
                            color: isCurrent ? Theme.textOnDark : Theme.textPrimary
                        }

                        MouseArea {
                            anchors.fill: parent
                            cursorShape: Qt.PointingHandCursor
                            onClicked: {
                                nightLightTempList.currentIndex = index;
                                nightLightTempList.forceActiveFocus();
                            }
                            onDoubleClicked: {
                                if (modelData.value === 0) {
                                    SettingsStore.setNightLightEnabled(false);
                                    root.applyNightLightSetting(false, SettingsStore.nightLightTemp);
                                } else {
                                    SettingsStore.setNightLightTemp(modelData.value);
                                    if (SettingsStore.nightLightEnabled)
                                        root.applyNightLightSetting(true, modelData.value);
                                }
                                nightLightTempScope.nightLightTempOpen = false;
                            }
                        }
                    }

                    Keys.onReturnPressed: {
                        if (currentIndex >= 0) {
                            let presets = nightLightTempScope.tempPresets;
                            if (currentIndex < presets.length) {
                                let preset = presets[currentIndex];
                                if (preset.value === 0) {
                                    SettingsStore.setNightLightEnabled(false);
                                    root.applyNightLightSetting(false, SettingsStore.nightLightTemp);
                                } else {
                                    SettingsStore.setNightLightTemp(preset.value);
                                    if (SettingsStore.nightLightEnabled)
                                        root.applyNightLightSetting(true, preset.value);
                                }
                                nightLightTempScope.nightLightTempOpen = false;
                            }
                        }
                    }

                    Keys.onEscapePressed: {
                        nightLightTempScope.nightLightTempOpen = false;
                        nightLightTempScope.forceActiveFocus();
                    }
                }

                Keys.onReturnPressed: {
                    if (!nightLightTempOpen) {
                        nightLightTempOpen = true;
                        nightLightTempList.forceActiveFocus();
                    }
                }

                Keys.onEscapePressed: {
                    if (nightLightTempOpen) {
                        nightLightTempOpen = false;
                    } else {
                        event.accepted = false;
                    }
                }
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

            FocusScope {
                id: overscanScope
                Layout.fillWidth: true
                Layout.preferredHeight: 96

                KeyNavigation.up: nightLightTempScope

                onActiveFocusChanged: if (activeFocus)
                    contentFlick.ensureVisible(this)

                property var overscanOptions: [0, 2, 4, 6, 8, 10]
                property int selectedIndex: {
                    let v = SettingsStore.overscan;
                    for (let i = 0; i < overscanOptions.length; i++) {
                        if (overscanOptions[i] === v)
                            return i;
                    }
                    return 0;
                }
                property int focusedIndex: selectedIndex

                Keys.onLeftPressed: {
                    if (focusedIndex > 0)
                        focusedIndex--;
                }
                Keys.onRightPressed: {
                    if (focusedIndex < overscanOptions.length - 1)
                        focusedIndex++;
                }
                Keys.onReturnPressed: {
                    SettingsStore.setOverscan(overscanOptions[focusedIndex]);
                }

                RowLayout {
                    anchors.fill: parent
                    spacing: 16

                    Repeater {
                        model: overscanScope.overscanOptions

                        FocusScope {
                            id: overscanOptScope
                            required property var modelData
                            required property int index
                            width: overscanBtn.width
                            height: overscanBtn.height

                            SettingsButton {
                                id: overscanBtn
                                text: parent.modelData + "%"
                                anchors.fill: parent

                                property bool isCurrent: SettingsStore.overscan === parent.modelData
                                property bool isFocused: overscanScope.activeFocus && overscanScope.focusedIndex === overscanOptScope.index

                                color: isCurrent ? Theme.sidebarActive : isFocused ? Theme.surfaceHover : Theme.surface
                                border.width: isFocused ? 2 : 1
                                border.color: isFocused ? Theme.focusBorder : Theme.surfaceBorder

                                onActivated: {
                                    SettingsStore.setOverscan(overscanOptScope.modelData);
                                }

                                MouseArea {
                                    anchors.fill: parent
                                    hoverEnabled: true
                                    cursorShape: Qt.PointingHandCursor
                                    onClicked: {
                                        overscanScope.forceActiveFocus();
                                        overscanScope.focusedIndex = overscanOptScope.index;
                                        overscanBtn.activated();
                                    }
                                }
                            }
                        }
                    }
                }
            }

            Text {
                text: "Overscan drives the shell safe-area margin (applied at next restart)."
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

                onActiveFocusChanged: if (activeFocus)
                    contentFlick.ensureVisible(this)

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
}
