import QtQuick
import Quickshell.Io

// Horizontal row of quick-action icons, navigable left-to-right. Reused in
// the top-right status strip (HomeScreen) and the navigation drawer. Icons
// are laid out left→right (index 0 = leftmost); Power is the last/rightmost
// action. Overflowing icons scroll horizontally and the selected icon is
// always kept in view, so the row stays usable as actions are added.
FocusScope {
    id: root

    signal settingsRequested
    signal notificationCenterRequested
    signal powerRequested
    signal focusDownRequested
    signal focusUpRequested

    property int currentIndex: 0
    property int iconSize: 64
    property int imgSize: 32
    readonly property int _spacing: 12
    // Cap the visible width to enable the scroll carousel. Defaults to
    // effectively unbounded (top-right has room for all icons).
    property int maxContentWidth: 100000
    // HomeScreen keeps the legacy "Escape opens Settings" affordance; the
    // drawer sets this false so Escape/B bubbles up to close the drawer.
    property bool escapeRequestsSettings: true

    // Must match the number of icon containers below
    // (Notifications=0, Settings=1, Theme=2, Network=3, Volume=4, Power=5)
    readonly property int _iconCount: 6

    implicitWidth: Math.min(iconRow.implicitWidth, maxContentWidth)
    implicitHeight: iconSize

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

    Component.onCompleted: iconThemeProbe.running = true

    function _ensureVisible() {
        var left = currentIndex * (iconSize + _spacing);
        var right = left + iconSize;
        if (left < flick.contentX)
            flick.contentX = left;
        else if (right > flick.contentX + flick.width)
            flick.contentX = right - flick.width;
    }

    onCurrentIndexChanged: _ensureVisible()
    onWidthChanged: _ensureVisible()

    // Keyboard navigation (LTR: Left lowers index, Right raises it).
    Keys.onLeftPressed: {
        if (currentIndex > 0)
            currentIndex--;
    }
    Keys.onRightPressed: {
        if (currentIndex < _iconCount - 1)
            currentIndex++;
    }
    Keys.onDownPressed: root.focusDownRequested()
    Keys.onUpPressed: root.focusUpRequested()
    Keys.onEscapePressed: {
        if (root.escapeRequestsSettings)
            root.settingsRequested();
    }
    Keys.onReturnPressed: root._activate(currentIndex)

    function _activate(index) {
        switch (index) {
        case 0:
            root.notificationCenterRequested();
            break;
        case 1:
            root.settingsRequested();
            break;
        case 2:
            if (Theme.themeMode === "auto")
                Theme.setThemeMode("light");
            else if (Theme.themeMode === "light")
                Theme.setThemeMode("dark");
            else
                Theme.setThemeMode("auto");
            break;
        case 5:
            root.powerRequested();
            break;
        }
    }

    Connections {
        target: Theme
        function onMouseModeChanged() {
            if (Theme.mouseMode)
                return;
            if (notifMA.containsMouse) {
                root.currentIndex = 0;
                root.forceActiveFocus();
            } else if (settingsMA.containsMouse) {
                root.currentIndex = 1;
                root.forceActiveFocus();
            } else if (themeMA.containsMouse) {
                root.currentIndex = 2;
                root.forceActiveFocus();
            } else if (networkMA.containsMouse) {
                root.currentIndex = 3;
                root.forceActiveFocus();
            } else if (volumeMA.containsMouse) {
                root.currentIndex = 4;
                root.forceActiveFocus();
            } else if (powerMA.containsMouse) {
                root.currentIndex = 5;
                root.forceActiveFocus();
            }
        }
    }

    Flickable {
        id: flick
        anchors.fill: parent
        contentWidth: iconRow.width
        contentHeight: height
        clip: true
        interactive: false
        boundsBehavior: Flickable.StopAtBounds

        Behavior on contentX {
            NumberAnimation {
                duration: 150
                easing.type: Easing.OutCubic
            }
        }

        Row {
            id: iconRow
            height: parent.height
            spacing: root._spacing

            // Notifications (index 0)
            Rectangle {
                width: root.iconSize
                height: root.iconSize
                radius: root.iconSize / 2
                color: notifMA.containsMouse && Theme.mouseMode ? Theme.surfaceHover : "transparent"
                border.width: root.activeFocus && !Theme.mouseMode && root.currentIndex === 0 ? 3 : 0
                border.color: Theme.focusBorder
                Behavior on color {
                    ColorAnimation {
                        duration: 150
                    }
                }

                Image {
                    id: notifIcon
                    anchors.centerIn: parent
                    source: root._iconBase ? "file://" + root._iconBase + "/actions/22/" + (NotificationManager.unreadCount > 0 ? "notification-active.svg" : "notification-inactive.svg") : ""
                    sourceSize: Qt.size(root.imgSize, root.imgSize)
                    width: root.imgSize
                    height: root.imgSize
                    fillMode: Image.PreserveAspectFit
                    visible: status === Image.Ready
                }
                Text {
                    anchors.centerIn: parent
                    text: "\u{1F514}"
                    font.pixelSize: root.imgSize
                    color: notifMA.containsMouse && Theme.mouseMode ? Theme.textPrimary : Theme.textMuted
                    visible: notifIcon.status !== Image.Ready
                }

                // Badge
                Rectangle {
                    visible: NotificationManager.unreadCount > 0
                    width: 20
                    height: 20
                    radius: 10
                    color: Theme.crimson
                    anchors.top: parent.top
                    anchors.right: parent.right
                    anchors.topMargin: 6
                    anchors.rightMargin: 6

                    Text {
                        anchors.centerIn: parent
                        text: NotificationManager.unreadCount > 9 ? "9+" : NotificationManager.unreadCount.toString()
                        font.pixelSize: 11
                        font.bold: true
                        color: Theme.textOnDark
                    }
                }

                MouseArea {
                    id: notifMA
                    anchors.fill: parent
                    hoverEnabled: true
                    cursorShape: Qt.PointingHandCursor
                    onClicked: root.notificationCenterRequested()
                }
            }

            // Settings (index 1)
            Rectangle {
                width: root.iconSize
                height: root.iconSize
                radius: root.iconSize / 2
                color: settingsMA.containsMouse && Theme.mouseMode ? Theme.surfaceHover : "transparent"
                border.width: root.activeFocus && !Theme.mouseMode && root.currentIndex === 1 ? 3 : 0
                border.color: Theme.focusBorder
                Behavior on color {
                    ColorAnimation {
                        duration: 150
                    }
                }

                Image {
                    id: settingsIcon
                    anchors.centerIn: parent
                    source: root._iconBase ? "file://" + root._iconBase + "/actions/22/configure.svg" : ""
                    sourceSize: Qt.size(root.imgSize, root.imgSize)
                    width: root.imgSize
                    height: root.imgSize
                    fillMode: Image.PreserveAspectFit
                    visible: status === Image.Ready
                }
                Text {
                    anchors.centerIn: parent
                    text: "⚙"
                    font.pixelSize: root.imgSize
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

            // Theme toggle (index 2)
            Rectangle {
                width: root.iconSize
                height: root.iconSize
                radius: root.iconSize / 2
                color: themeMA.containsMouse && Theme.mouseMode ? Theme.surfaceHover : "transparent"
                border.width: root.activeFocus && !Theme.mouseMode && root.currentIndex === 2 ? 3 : 0
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
                    sourceSize: Qt.size(root.imgSize, root.imgSize)
                    width: root.imgSize
                    height: root.imgSize
                    fillMode: Image.PreserveAspectFit
                    visible: status === Image.Ready
                }
                Text {
                    anchors.centerIn: parent
                    text: Theme.themeMode === "dark" ? "☽" : Theme.themeMode === "light" ? "☀" : "◐"
                    font.pixelSize: root.imgSize
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

            // Network (index 3)
            Rectangle {
                width: root.iconSize
                height: root.iconSize
                radius: root.iconSize / 2
                color: networkMA.containsMouse && Theme.mouseMode ? Theme.surfaceHover : "transparent"
                border.width: root.activeFocus && !Theme.mouseMode && root.currentIndex === 3 ? 3 : 0
                border.color: Theme.focusBorder
                Behavior on color {
                    ColorAnimation {
                        duration: 150
                    }
                }

                property string _netIconPath: {
                    if (!root._iconBase)
                        return "";
                    if (NetworkManager.connected)
                        return "file://" + root._iconBase + "/status/22/network-wired.svg";
                    return "file://" + root._iconBase + "/actions/22/network-disconnect.svg";
                }

                Image {
                    id: netIcon
                    anchors.centerIn: parent
                    source: parent._netIconPath
                    sourceSize: Qt.size(root.imgSize, root.imgSize)
                    width: root.imgSize
                    height: root.imgSize
                    fillMode: Image.PreserveAspectFit
                    visible: status === Image.Ready
                }
                Text {
                    anchors.centerIn: parent
                    text: NetworkManager.connected ? "⛁" : "⚠"
                    font.pixelSize: root.imgSize
                    color: NetworkManager.connected ? Theme.textMuted : Theme.warning
                    visible: netIcon.status !== Image.Ready
                }
                MouseArea {
                    id: networkMA
                    anchors.fill: parent
                    hoverEnabled: true
                }
            }

            // Volume (index 4)
            Rectangle {
                width: root.iconSize
                height: root.iconSize
                radius: root.iconSize / 2
                color: volumeMA.containsMouse && Theme.mouseMode ? Theme.surfaceHover : "transparent"
                border.width: root.activeFocus && !Theme.mouseMode && root.currentIndex === 4 ? 3 : 0
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
                    sourceSize: Qt.size(root.imgSize, root.imgSize)
                    width: root.imgSize
                    height: root.imgSize
                    fillMode: Image.PreserveAspectFit
                    visible: status === Image.Ready
                }
                Text {
                    anchors.centerIn: parent
                    text: "♫"
                    font.pixelSize: root.imgSize
                    color: Theme.textMuted
                    visible: volIcon.status !== Image.Ready
                }
                MouseArea {
                    id: volumeMA
                    anchors.fill: parent
                    hoverEnabled: true
                }
            }

            // Power (index 5)
            Rectangle {
                width: root.iconSize
                height: root.iconSize
                radius: root.iconSize / 2
                color: powerMA.containsMouse && Theme.mouseMode ? Theme.surfaceHover : "transparent"
                border.width: root.activeFocus && !Theme.mouseMode && root.currentIndex === 5 ? 3 : 0
                border.color: Theme.focusBorder
                Behavior on color {
                    ColorAnimation {
                        duration: 150
                    }
                }

                Image {
                    id: powerIcon
                    anchors.centerIn: parent
                    source: root._iconBase ? "file://" + root._iconBase + "/actions/22/system-shutdown.svg" : ""
                    sourceSize: Qt.size(root.imgSize, root.imgSize)
                    width: root.imgSize
                    height: root.imgSize
                    fillMode: Image.PreserveAspectFit
                    visible: status === Image.Ready
                }
                Text {
                    anchors.centerIn: parent
                    text: "⏻"
                    font.pixelSize: root.imgSize
                    color: powerMA.containsMouse && Theme.mouseMode ? Theme.textPrimary : Theme.textMuted
                    visible: powerIcon.status !== Image.Ready
                }
                MouseArea {
                    id: powerMA
                    anchors.fill: parent
                    hoverEnabled: true
                    cursorShape: Qt.PointingHandCursor
                    onClicked: root.powerRequested()
                }
            }
        }
    }
}
