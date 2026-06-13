import QtQuick
import QtQuick.Layouts
import Quickshell.Io
import "lib"

FocusScope {
    id: root
    implicitHeight: pwrMainCol.implicitHeight + 2 * Theme.padding

    property string confirmAction: ""
    // Whether logind reports suspend is available (queried via the daemon).
    // Defaults true so the Sleep button is enabled until told otherwise.
    property bool canSuspend: true

    // Suspend is routed through the input daemon's logind-over-zbus backbone
    // (Phase 3). Reboot/poweroff remain one-shot `systemctl` actions — they are
    // not system-state reads and have no daemon equivalent in scope.
    //
    // Suspend/CanSuspend go through the daemon over a native Quickshell socket
    // (SocketClient, #97) — the python3 socket shim was retired in Phase 8.

    Process {
        id: powerOff
        command: ["systemctl", "poweroff"]
    }
    Process {
        id: rebootCmd
        command: ["systemctl", "reboot"]
    }
    // logind Suspend (false = no interactive polkit prompt) via the daemon.
    SocketClient {
        id: suspendCmd
    }
    // End session: send intent home → shell.qml onIntentHome: returnToShell()
    SocketClient {
        id: endSessionCmd
    }
    // Query logind CanSuspend so the Sleep button reflects availability.
    SocketClient {
        id: canSuspendProc
        onResponseReceived: response => {
            let t = response.trim();
            if (t === "yes")
                root.canSuspend = true;
            else if (t === "no")
                root.canSuspend = false;
            // "error" leaves the optimistic default untouched.
        }
    }

    Component.onCompleted: canSuspendProc.request("power-can-suspend")

    onVisibleChanged: {
        if (visible) {
            root.confirmAction = "";
            canSuspendProc.request("power-can-suspend");
        }
    }

    function focusFirst() {
        sleepTimerScope.forceActiveFocus();
    }

    ColumnLayout {
        id: pwrMainCol
        anchors.fill: parent
        anchors.margins: Theme.padding
        spacing: 32

        SectionHeader {
            text: "Power Settings"
        }

        // Power settings controls — sleep timer, wake-on-controller, end session
        ColumnLayout {
            Layout.alignment: Qt.AlignLeft
            Layout.fillWidth: true
            spacing: 24

            // Sleep Timer row
            RowLayout {
                Layout.fillWidth: true
                spacing: 24

                Text {
                    text: "Sleep Timer"
                    font.pixelSize: Theme.fontSmall
                    color: Theme.textSecondary
                    Layout.fillWidth: true
                }

                FocusButton {
                    id: sleepTimerScope
                    KeyNavigation.down: wakeOnControllerScope
                    text: SettingsStore.sleepTimerMinutes === 0 ? "Off" : SettingsStore.sleepTimerMinutes + " min"
                    onActivated: {
                        var steps = [0, 5, 10, 15, 30, 60];
                        var idx = steps.indexOf(SettingsStore.sleepTimerMinutes);
                        var next = steps[(idx + 1) % steps.length];
                        SettingsStore.setSleepTimerMinutes(next);
                    }
                }
            }

            Text {
                text: "Suspends after this idle time"
                font.pixelSize: Theme.fontHint
                color: Theme.textSecondary
                leftPadding: 0
            }

            // Wake on controller row
            RowLayout {
                Layout.fillWidth: true
                spacing: 24

                Text {
                    text: "Wake on controller"
                    font.pixelSize: Theme.fontSmall
                    color: Theme.textSecondary
                    Layout.fillWidth: true
                }

                FocusButton {
                    id: wakeOnControllerScope
                    KeyNavigation.up: sleepTimerScope
                    KeyNavigation.down: endSessionScope
                    text: SettingsStore.wakeOnController ? "On" : "Off"
                    fillActive: SettingsStore.wakeOnController
                    fillColor: Theme.sidebarActive
                    onActivated: SettingsStore.setWakeOnController(!SettingsStore.wakeOnController)
                }
            }

            // End session row
            RowLayout {
                Layout.fillWidth: true
                spacing: 24

                ColumnLayout {
                    spacing: 2
                    Layout.fillWidth: true

                    Text {
                        text: "End session"
                        font.pixelSize: Theme.fontSmall
                        color: Theme.textSecondary
                    }

                    Text {
                        text: "Return to shell (Home + B)"
                        font.pixelSize: Theme.fontHint
                        color: Theme.textSecondary
                    }
                }

                FocusButton {
                    id: endSessionScope
                    KeyNavigation.up: wakeOnControllerScope
                    KeyNavigation.down: suspendScope
                    text: "End"
                    onActivated: endSessionCmd.request("intent home")
                }
            }
        }

        Item {
            Layout.fillHeight: true
            Layout.maximumHeight: 100
        }

        SectionHeader {
            text: "Power Actions"
            block: false
            Layout.alignment: Qt.AlignHCenter
        }

        // Power buttons - large and centered
        ColumnLayout {
            Layout.alignment: Qt.AlignHCenter
            spacing: 32

            FocusScope {
                id: suspendScope
                Layout.preferredWidth: 500
                Layout.preferredHeight: 120
                focus: true
                activeFocusOnTab: true

                KeyNavigation.up: endSessionScope
                KeyNavigation.down: restartScope

                Rectangle {
                    anchors.fill: parent
                    radius: 24
                    color: parent.activeFocus ? Theme.gold : Theme.surface
                    border.width: parent.activeFocus ? 0 : 2
                    border.color: Theme.surfaceHover

                    Behavior on color {
                        ColorAnimation {
                            duration: 150
                        }
                    }

                    ColumnLayout {
                        anchors.centerIn: parent
                        spacing: 4

                        Text {
                            text: "Sleep"
                            font.pixelSize: Theme.fontTitle
                            font.bold: true
                            color: suspendScope.activeFocus ? Theme.textOnDark : Theme.textPrimary
                            Layout.alignment: Qt.AlignHCenter
                        }

                        Text {
                            text: root.canSuspend ? "Suspend to RAM" : "Suspend unavailable"
                            font.pixelSize: Theme.fontSmall
                            color: suspendScope.activeFocus ? Theme.textOnDarkMuted : Theme.textSecondary
                            Layout.alignment: Qt.AlignHCenter
                        }
                    }

                    MouseArea {
                        anchors.fill: parent
                        hoverEnabled: true
                        cursorShape: Qt.PointingHandCursor
                        onClicked: {
                            suspendScope.forceActiveFocus();
                            if (root.canSuspend)
                                root.confirmAction = "suspend";
                        }
                    }
                }

                Keys.onReturnPressed: {
                    if (root.canSuspend)
                        root.confirmAction = "suspend";
                }
            }

            FocusScope {
                id: restartScope
                Layout.preferredWidth: 500
                Layout.preferredHeight: 120
                activeFocusOnTab: true

                KeyNavigation.up: suspendScope
                KeyNavigation.down: shutdownScope

                Rectangle {
                    anchors.fill: parent
                    radius: 24
                    color: parent.activeFocus ? Theme.ember : Theme.surface
                    border.width: parent.activeFocus ? 0 : 2
                    border.color: Theme.surfaceHover

                    Behavior on color {
                        ColorAnimation {
                            duration: 150
                        }
                    }

                    ColumnLayout {
                        anchors.centerIn: parent
                        spacing: 4

                        Text {
                            text: "Restart"
                            font.pixelSize: Theme.fontTitle
                            font.bold: true
                            color: restartScope.activeFocus ? Theme.textOnDark : Theme.textPrimary
                            Layout.alignment: Qt.AlignHCenter
                        }

                        Text {
                            text: "Reboot the system"
                            font.pixelSize: Theme.fontSmall
                            color: restartScope.activeFocus ? Theme.textOnDarkMuted : Theme.textSecondary
                            Layout.alignment: Qt.AlignHCenter
                        }
                    }

                    MouseArea {
                        anchors.fill: parent
                        hoverEnabled: true
                        cursorShape: Qt.PointingHandCursor
                        onClicked: {
                            restartScope.forceActiveFocus();
                            root.confirmAction = "restart";
                        }
                    }
                }

                Keys.onReturnPressed: {
                    root.confirmAction = "restart";
                }
            }

            FocusScope {
                id: shutdownScope
                Layout.preferredWidth: 500
                Layout.preferredHeight: 120
                activeFocusOnTab: true

                KeyNavigation.up: restartScope

                Rectangle {
                    anchors.fill: parent
                    radius: 24
                    color: parent.activeFocus ? Theme.crimson : Theme.surface
                    border.width: parent.activeFocus ? 0 : 2
                    border.color: Theme.surfaceHover

                    Behavior on color {
                        ColorAnimation {
                            duration: 150
                        }
                    }

                    ColumnLayout {
                        anchors.centerIn: parent
                        spacing: 4

                        Text {
                            text: "Shutdown"
                            font.pixelSize: Theme.fontTitle
                            font.bold: true
                            color: shutdownScope.activeFocus ? Theme.textOnDark : Theme.textPrimary
                            Layout.alignment: Qt.AlignHCenter
                        }

                        Text {
                            text: "Power off the system"
                            font.pixelSize: Theme.fontSmall
                            color: shutdownScope.activeFocus ? Theme.textOnDarkMuted : Theme.textSecondary
                            Layout.alignment: Qt.AlignHCenter
                        }
                    }

                    MouseArea {
                        anchors.fill: parent
                        hoverEnabled: true
                        cursorShape: Qt.PointingHandCursor
                        onClicked: {
                            shutdownScope.forceActiveFocus();
                            root.confirmAction = "shutdown";
                        }
                    }
                }

                Keys.onReturnPressed: {
                    root.confirmAction = "shutdown";
                }
            }
        }

        Item {
            Layout.fillHeight: true
        }

        HintBar {
            text: "A: Select  |  Use with caution"
        }
    }

    // Confirmation dialog
    Rectangle {
        anchors.fill: parent
        color: Qt.rgba(0, 0, 0, 0.7)
        visible: root.confirmAction !== ""

        MouseArea {
            anchors.fill: parent
            onClicked: {
                root.confirmAction = "";
            }
        }

        Rectangle {
            anchors.centerIn: parent
            width: 700
            height: 350
            radius: Units.radiusXL
            color: Theme.surface

            ColumnLayout {
                anchors.centerIn: parent
                spacing: 32

                Text {
                    text: {
                        switch (root.confirmAction) {
                        case "suspend":
                            return "Sleep this system?";
                        case "restart":
                            return "Restart this system?";
                        case "shutdown":
                            return "Shut down this system?";
                        default:
                            return "";
                        }
                    }
                    font.pixelSize: Theme.fontTitle
                    font.bold: true
                    color: Theme.textPrimary
                    Layout.alignment: Qt.AlignHCenter
                }

                RowLayout {
                    Layout.alignment: Qt.AlignHCenter
                    spacing: 32

                    FocusButton {
                        id: confirmYesScope
                        KeyNavigation.right: confirmNoScope
                        text: "Yes"
                        onActivated: executeAction()
                    }

                    FocusButton {
                        id: confirmNoScope
                        focus: root.confirmAction !== ""
                        KeyNavigation.left: confirmYesScope
                        text: "Cancel"
                        onActivated: root.confirmAction = ""
                        Keys.onEscapePressed: root.confirmAction = ""
                    }
                }
            }
        }

        Keys.onEscapePressed: {
            root.confirmAction = "";
        }
    }

    function executeAction() {
        switch (root.confirmAction) {
        case "suspend":
            suspendCmd.request("power-suspend");
            break;
        case "restart":
            rebootCmd.running = true;
            break;
        case "shutdown":
            powerOff.running = true;
            break;
        }
        root.confirmAction = "";
    }
}
