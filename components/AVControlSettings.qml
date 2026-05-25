import QtQuick
import QtQuick.Layouts
import Quickshell.Io

Item {
    id: root

    property bool cecAvailable: false
    property bool hasAvScript: false
    property var devices: []
    property string statusText: "Checking CEC availability..."
    property string actionFeedback: ""

    // --- CEC availability check ---
    Process {
        id: checkCec
        command: ["bash", "-c", "which cec-client 2>/dev/null && echo 'available' || echo 'unavailable'"]
        stdout: SplitParser {
            onRead: (line) => {
                if (line.trim() === "available") {
                    root.cecAvailable = true
                    root.statusText = "CEC available — scanning devices..."
                    scanDevices.running = true
                } else {
                    root.cecAvailable = false
                    root.statusText = "CEC tools not installed"
                }
            }
        }
    }

    // --- AV script detection ---
    Process {
        id: checkAvScript
        command: ["bash", "-c", "which living-room-cec 2>/dev/null && echo 'has-script' || echo 'no-script'"]
        stdout: SplitParser {
            onRead: (line) => {
                root.hasAvScript = (line.trim() === "has-script")
            }
        }
    }

    // --- Device scan ---
    Process {
        id: scanDevices
        command: ["bash", "-c", "echo 'scan' | timeout 10 cec-client -s -d 1 2>/dev/null"]
        stdout: SplitParser {
            property string buffer: ""
            onRead: (line) => { buffer += line + "\n" }
        }
        onExited: {
            root.parseDevices(scanDevices.stdout.buffer)
            scanDevices.stdout.buffer = ""
        }
    }

    // --- CEC commands ---
    Process {
        id: wakeCmd
        command: ["bash", "-c", ""]
        onExited: {
            root.actionFeedback = "Wake command sent"
            feedbackTimer.restart()
            refreshAfterAction.restart()
        }
    }

    Process {
        id: sleepCmd
        command: ["bash", "-c", ""]
        onExited: {
            root.actionFeedback = "Sleep command sent"
            feedbackTimer.restart()
            refreshAfterAction.restart()
        }
    }

    Process {
        id: switchInputCmd
        command: ["bash", "-c", "echo 'as' | cec-client -s -d 1 2>/dev/null"]
        onExited: {
            root.actionFeedback = "Input switch command sent"
            feedbackTimer.restart()
        }
    }

    // --- Timers ---
    Timer {
        id: autoRefresh
        interval: 30000
        running: root.visible && root.cecAvailable
        repeat: true
        onTriggered: { scanDevices.running = true }
    }

    Timer {
        id: feedbackTimer
        interval: 3000
        onTriggered: { root.actionFeedback = "" }
    }

    Timer {
        id: refreshAfterAction
        interval: 5000
        onTriggered: { scanDevices.running = true }
    }

    Component.onCompleted: {
        checkCec.running = true
        checkAvScript.running = true
    }

    onVisibleChanged: {
        if (visible) {
            checkCec.running = true
            checkAvScript.running = true
            wakeScope.forceActiveFocus()
        }
    }

    // --- Parse cec-client scan output ---
    function parseDevices(output) {
        var devs = []
        var lines = output.split("\n")
        var current = null

        for (var i = 0; i < lines.length; i++) {
            var line = lines[i]

            // Device header: "device #N:"
            var deviceMatch = line.match(/device\s+#(\d+):/)
            if (deviceMatch) {
                if (current) devs.push(current)
                current = {
                    logicalAddress: parseInt(deviceMatch[1]),
                    name: "Unknown",
                    physicalAddress: "",
                    vendor: "",
                    powerStatus: "unknown",
                    type: ""
                }
                continue
            }

            if (!current) continue

            // OSD String
            var osdMatch = line.match(/osd string\s*:\s*(.+)/i)
            if (osdMatch) {
                current.name = osdMatch[1].trim()
                continue
            }

            // Physical address
            var physMatch = line.match(/address\s*:\s*([0-9.]+)/i)
            if (physMatch) {
                current.physicalAddress = physMatch[1].trim()
                continue
            }

            // Vendor
            var vendorMatch = line.match(/vendor\s*:\s*(.+)/i)
            if (vendorMatch) {
                current.vendor = vendorMatch[1].trim()
                continue
            }

            // Power status
            var powerMatch = line.match(/power status\s*:\s*(.+)/i)
            if (powerMatch) {
                current.powerStatus = powerMatch[1].trim().toLowerCase()
                continue
            }

            // Type
            var typeMatch = line.match(/type\s*:\s*(.+)/i)
            if (typeMatch) {
                current.type = typeMatch[1].trim()
                continue
            }
        }
        if (current) devs.push(current)

        root.devices = devs
        if (devs.length > 0) {
            root.statusText = devs.length + " device" + (devs.length !== 1 ? "s" : "") + " detected"
        } else if (root.cecAvailable) {
            root.statusText = "No CEC devices found"
        }
    }

    function doWake() {
        if (root.hasAvScript) {
            wakeCmd.command = ["living-room-cec", "wake"]
        } else {
            wakeCmd.command = ["bash", "-c", "echo 'on 0' | cec-client -s -d 1 2>/dev/null"]
        }
        wakeCmd.running = true
    }

    function doSleep() {
        if (root.hasAvScript) {
            sleepCmd.command = ["living-room-cec", "sleep"]
        } else {
            sleepCmd.command = ["bash", "-c", "echo 'standby 0' | cec-client -s -d 1 2>/dev/null"]
        }
        sleepCmd.running = true
    }

    function doSwitchInput() {
        switchInputCmd.running = true
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

            Text {
                text: "AV Control"
                font.pixelSize: Theme.fontBody
                font.bold: true
                color: Theme.textPrimary
            }

            Item { Layout.fillWidth: true }

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

                SettingsButton {
                    id: refreshBtn
                    text: "Refresh"
                    focus: parent.activeFocus
                    anchors.fill: parent
                }

                Keys.onReturnPressed: {
                    root.statusText = "Scanning..."
                    scanDevices.running = true
                }

                MouseArea {
                    anchors.fill: parent
                    hoverEnabled: true
                    cursorShape: Qt.PointingHandCursor
                    onClicked: {
                        refreshScope.forceActiveFocus()
                        root.statusText = "Scanning..."
                        scanDevices.running = true
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
                    text: "Install cec-utils for HDMI-CEC device control."
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
                    focus: true
                    activeFocusOnTab: true

                    KeyNavigation.right: sleepScope
                    KeyNavigation.down: deviceListView.count > 0 ? deviceListView : wakeScope

                    Rectangle {
                        anchors.fill: parent
                        radius: 24
                        color: parent.activeFocus ? Theme.online : Theme.surface
                        border.width: parent.activeFocus ? 0 : 2
                        border.color: Theme.surfaceBorder

                        Behavior on color { ColorAnimation { duration: 150 } }

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
                                text: root.hasAvScript ? "Using AV script" : "CEC power on"
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
                                wakeScope.forceActiveFocus()
                                root.doWake()
                            }
                        }
                    }

                    Keys.onReturnPressed: { root.doWake() }
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

                        Behavior on color { ColorAnimation { duration: 150 } }

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
                                text: root.hasAvScript ? "Using AV script" : "CEC standby"
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
                                sleepScope.forceActiveFocus()
                                root.doSleep()
                            }
                        }
                    }

                    Keys.onReturnPressed: { root.doSleep() }
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

                        Behavior on color { ColorAnimation { duration: 150 } }

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
                                switchScope.forceActiveFocus()
                                root.doSwitchInput()
                            }
                        }
                    }

                    Keys.onReturnPressed: { root.doSwitchInput() }
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
                    color: deviceListView.currentIndex === index && deviceListView.activeFocus
                           ? Theme.surfaceHover : Theme.surface
                    border.width: 2
                    border.color: Theme.surfaceBorder

                    Behavior on color { ColorAnimation { duration: 150 } }

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
                                    text: modelData.name
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
                                        if (modelData.powerStatus === "on") return Qt.rgba(0.176, 0.541, 0.306, 0.2)
                                        if (modelData.powerStatus === "standby") return Qt.rgba(0.843, 0.651, 0.294, 0.2)
                                        return Theme.surfaceHover
                                    }

                                    Text {
                                        id: powerLabel
                                        anchors.centerIn: parent
                                        text: {
                                            if (modelData.powerStatus === "on") return "On"
                                            if (modelData.powerStatus === "standby") return "Standby"
                                            return "Unknown"
                                        }
                                        font.pixelSize: Theme.fontSmall - 4
                                        color: {
                                            if (modelData.powerStatus === "on") return Theme.online
                                            if (modelData.powerStatus === "standby") return Theme.gold
                                            return Theme.textSecondary
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
                            deviceListView.currentIndex = index
                            deviceListView.forceActiveFocus()
                        }
                    }
                }
            }
        }

        // Hint bar
        Text {
            text: root.cecAvailable ? "A: Select  |  Auto-refresh every 30s" : "Install: sudo apt install cec-utils  (or equivalent)"
            font.pixelSize: Theme.fontHint
            color: Theme.textSecondary
            Layout.alignment: Qt.AlignHCenter
        }
    }
}
