import QtQuick
import QtQuick.Layouts
import Quickshell.Io

FocusScope {
    id: root

    signal settingsRequested
    signal focusDownRequested

    property string ipAddress: "..."
    property bool _wasConnected: false
    property bool _networkInitialized: false
    readonly property int _iconSize: 64
    readonly property int _imgSize: 32
    // Must match the number of icon containers below (Settings=0, Theme=1, Network=2, Volume=3)
    readonly property int _iconCount: 4
    property int currentIndex: 0

    implicitWidth: iconRow.implicitWidth
    implicitHeight: iconRow.implicitHeight

    property string _iconBase: ""

    Process {
        id: iconThemeProbe
        command: ["bash", "-c", "for d in /usr/share/icons/breeze /usr/share/icons/Adwaita /usr/share/icons/hicolor; do [ -d \"$d\" ] && echo \"$d\" && exit; done; echo ''"]
        stdout: SplitParser {
            onRead: line => {
                root._iconBase = line.trim();
            }
        }
    }

    Component.onCompleted: {
        iconThemeProbe.running = true;
    }

    Process {
        id: ipProcess
        command: ["hostname", "-I"]
        stdout: SplitParser {
            onRead: line => {
                root.ipAddress = line.trim().split(" ")[0] || "No IP";
            }
        }
    }

    Timer {
        interval: 30000
        running: true
        repeat: true
        triggeredOnStart: true
        onTriggered: {
            if (!ipProcess.running)
                ipProcess.running = true;
        }
    }

    onIpAddressChanged: {
        var connected = ipAddress !== "..." && ipAddress !== "No IP"
        if (!root._networkInitialized) {
            root._networkInitialized = true
            root._wasConnected = connected
            return
        }
        if (root._wasConnected && !connected) {
            NotificationManager.notify("Network Disconnected", "", {icon: "📶", level: "warning", source: "network"})
        } else if (!root._wasConnected && connected) {
            NotificationManager.notify("Network Connected", ipAddress, {icon: "📶", source: "network"})
        }
        root._wasConnected = connected
    }

    // Keyboard navigation (RTL layout: Right moves to lower index, Left to higher)
    Keys.onRightPressed: {
        if (currentIndex > 0)
            currentIndex--;
    }
    Keys.onLeftPressed: {
        if (currentIndex < _iconCount - 1)
            currentIndex++;
    }
    Keys.onDownPressed: root.focusDownRequested()
    Keys.onEscapePressed: root.settingsRequested()
    Keys.onReturnPressed: {
        switch (currentIndex) {
        case 0:
            root.settingsRequested();
            break;
        case 1:
            if (Theme.themeMode === "auto")
                Theme.setThemeMode("light");
            else if (Theme.themeMode === "light")
                Theme.setThemeMode("dark");
            else
                Theme.setThemeMode("auto");
            break;
        }
    }

    Connections {
        target: Theme
        function onMouseModeChanged() {
            if (Theme.mouseMode)
                return;
            if (settingsMA.containsMouse) {
                root.currentIndex = 0;
                root.forceActiveFocus();
            } else if (themeMA.containsMouse) {
                root.currentIndex = 1;
                root.forceActiveFocus();
            } else if (networkMA.containsMouse) {
                root.currentIndex = 2;
                root.forceActiveFocus();
            } else if (volumeMA.containsMouse) {
                root.currentIndex = 3;
                root.forceActiveFocus();
            }
        }
    }

    RowLayout {
        id: iconRow
        anchors.fill: parent
        spacing: 12
        layoutDirection: Qt.RightToLeft

        // Settings (index 0)
        Rectangle {
            Layout.preferredWidth: root._iconSize
            Layout.preferredHeight: root._iconSize
            radius: root._iconSize / 2
            color: settingsMA.containsMouse && Theme.mouseMode ? Theme.surfaceHover : "transparent"
            border.width: root.activeFocus && !Theme.mouseMode && root.currentIndex === 0 ? 3 : 0
            border.color: Theme.focusBorder
            Behavior on color {
                ColorAnimation {
                    duration: 150
                }
            }

            Image {
                id: settingsIcon
                anchors.centerIn: parent
                source: root._iconBase ? "file://" + root._iconBase + "/apps/32/preferences-system.svg" : ""
                sourceSize: Qt.size(root._imgSize, root._imgSize)
                width: root._imgSize
                height: root._imgSize
                fillMode: Image.PreserveAspectFit
                visible: status === Image.Ready
            }
            Text {
                anchors.centerIn: parent
                text: "⚙"
                font.pixelSize: root._imgSize
                color: settingsMA.containsMouse && Theme.mouseMode ? Theme.textPrimary : Theme.textMuted
                visible: settingsIcon.status !== Image.Ready
            }
            MouseArea {
                id: settingsMA
                anchors.fill: parent
                hoverEnabled: true
                cursorShape: Qt.PointingHandCursor
                onClicked: root.settingsRequested()
            }
        }

        // Theme toggle (index 1)
        Rectangle {
            Layout.preferredWidth: root._iconSize
            Layout.preferredHeight: root._iconSize
            radius: root._iconSize / 2
            color: themeMA.containsMouse && Theme.mouseMode ? Theme.surfaceHover : "transparent"
            border.width: root.activeFocus && !Theme.mouseMode && root.currentIndex === 1 ? 3 : 0
            border.color: Theme.focusBorder
            Behavior on color {
                ColorAnimation {
                    duration: 150
                }
            }

            property string _themeIconPath: {
                if (!root._iconBase)
                    return "";
                if (Theme.themeMode === "dark")
                    return "file://" + root._iconBase + "/applets/48/weather-clear-night.svg";
                if (Theme.themeMode === "light")
                    return "file://" + root._iconBase + "/applets/48/weather-clear.svg";
                return "file://" + root._iconBase + "/actions/22/brightness-high.svg";
            }

            Image {
                id: themeIcon
                anchors.centerIn: parent
                source: parent._themeIconPath
                sourceSize: Qt.size(root._imgSize, root._imgSize)
                width: root._imgSize
                height: root._imgSize
                fillMode: Image.PreserveAspectFit
                visible: status === Image.Ready
            }
            Text {
                anchors.centerIn: parent
                text: Theme.themeMode === "dark" ? "☽" : Theme.themeMode === "light" ? "☀" : "◐"
                font.pixelSize: root._imgSize
                color: themeMA.containsMouse && Theme.mouseMode ? Theme.textPrimary : Theme.textMuted
                visible: themeIcon.status !== Image.Ready
            }
            MouseArea {
                id: themeMA
                anchors.fill: parent
                hoverEnabled: true
                cursorShape: Qt.PointingHandCursor
                onClicked: {
                    if (Theme.themeMode === "auto")
                        Theme.setThemeMode("light");
                    else if (Theme.themeMode === "light")
                        Theme.setThemeMode("dark");
                    else
                        Theme.setThemeMode("auto");
                }
            }
        }

        // Network (index 2)
        Rectangle {
            Layout.preferredWidth: root._iconSize
            Layout.preferredHeight: root._iconSize
            radius: root._iconSize / 2
            color: networkMA.containsMouse && Theme.mouseMode ? Theme.surfaceHover : "transparent"
            border.width: root.activeFocus && !Theme.mouseMode && root.currentIndex === 2 ? 3 : 0
            border.color: Theme.focusBorder
            Behavior on color {
                ColorAnimation {
                    duration: 150
                }
            }

            property bool _connected: root.ipAddress !== "..." && root.ipAddress !== "No IP"
            property string _netIconPath: {
                if (!root._iconBase)
                    return "";
                if (_connected)
                    return "file://" + root._iconBase + "/status/22/network-wired.svg";
                return "file://" + root._iconBase + "/actions/22/network-disconnect.svg";
            }

            Image {
                id: netIcon
                anchors.centerIn: parent
                source: parent._netIconPath
                sourceSize: Qt.size(root._imgSize, root._imgSize)
                width: root._imgSize
                height: root._imgSize
                fillMode: Image.PreserveAspectFit
                visible: status === Image.Ready
            }
            Text {
                anchors.centerIn: parent
                text: parent._connected ? "⛁" : "⚠"
                font.pixelSize: root._imgSize
                color: parent._connected ? Theme.textMuted : Theme.warning
                visible: netIcon.status !== Image.Ready
            }
            MouseArea {
                id: networkMA
                anchors.fill: parent
                hoverEnabled: true
            }
        }

        // Volume (index 3)
        Rectangle {
            Layout.preferredWidth: root._iconSize
            Layout.preferredHeight: root._iconSize
            radius: root._iconSize / 2
            color: volumeMA.containsMouse && Theme.mouseMode ? Theme.surfaceHover : "transparent"
            border.width: root.activeFocus && !Theme.mouseMode && root.currentIndex === 3 ? 3 : 0
            border.color: Theme.focusBorder
            Behavior on color {
                ColorAnimation {
                    duration: 150
                }
            }

            Image {
                id: volIcon
                anchors.centerIn: parent
                source: root._iconBase ? "file://" + root._iconBase + "/status/22/audio-volume-high.svg" : ""
                sourceSize: Qt.size(root._imgSize, root._imgSize)
                width: root._imgSize
                height: root._imgSize
                fillMode: Image.PreserveAspectFit
                visible: status === Image.Ready
            }
            Text {
                anchors.centerIn: parent
                text: "♫"
                font.pixelSize: root._imgSize
                color: Theme.textMuted
                visible: volIcon.status !== Image.Ready
            }
            MouseArea {
                id: volumeMA
                anchors.fill: parent
                hoverEnabled: true
            }
        }
    }
}
