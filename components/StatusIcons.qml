import QtQuick
import QtQuick.Layouts
import Quickshell.Io

FocusScope {
    id: root
    implicitWidth: row.implicitWidth
    implicitHeight: row.implicitHeight

    signal settingsRequested()
    signal focusDownRequested()

    property string ipAddress: "..."
    readonly property int _iconSize: 64
    property int currentIndex: 0
    // NOTE: _iconCount must match the number of icon containers in the RowLayout
    // below (Settings=0, Theme=1, Network=2, Volume=3). Update this value when
    // adding or removing icon containers.
    readonly property int _iconCount: 4

    Process {
        id: ipProcess
        command: ["hostname", "-I"]
        stdout: SplitParser {
            onRead: (line) => { root.ipAddress = line.trim().split(" ")[0] || "No IP" }
        }
    }

    Timer {
        interval: 30000
        running: true
        repeat: true
        triggeredOnStart: true
        onTriggered: { if (!ipProcess.running) ipProcess.running = true }
    }

    // Reset currentIndex to 0 when gaining focus
    onActiveFocusChanged: {
        if (activeFocus) currentIndex = 0
    }

    Keys.onLeftPressed: {
        // RightToLeft layout: visual left = higher index
        if (currentIndex < _iconCount - 1) currentIndex++
    }
    Keys.onRightPressed: {
        // RightToLeft layout: visual right = lower index
        if (currentIndex > 0) currentIndex--
    }
    Keys.onDownPressed: root.focusDownRequested()
    Keys.onEscapePressed: root.settingsRequested()
    Keys.onReturnPressed: {
        switch (currentIndex) {
        case 0: root.settingsRequested(); break
        case 1:
            if (Theme.themeMode === "auto") Theme.setThemeMode("light")
            else if (Theme.themeMode === "light") Theme.setThemeMode("dark")
            else Theme.setThemeMode("auto")
            break
        // Network and volume are display-only, no action
        }
    }

    RowLayout {
        id: row
        spacing: 16
        layoutDirection: Qt.RightToLeft

        // Settings (index 0)
        Rectangle {
            Layout.preferredWidth: root._iconSize
            Layout.preferredHeight: root._iconSize
            radius: root._iconSize / 2
            color: settingsMA.containsMouse ? Theme.surfaceHover : "transparent"
            border.width: root.activeFocus && root.currentIndex === 0 ? 2 : 0
            border.color: Theme.focusBorder
            Behavior on color { ColorAnimation { duration: 150 } }

            Text {
                anchors.centerIn: parent
                text: "⚙"
                font.pixelSize: Theme.fontBody
                color: (settingsMA.containsMouse || (root.activeFocus && root.currentIndex === 0))
                           ? Theme.textPrimary : Theme.textMuted
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
            color: themeMA.containsMouse ? Theme.surfaceHover : "transparent"
            border.width: root.activeFocus && root.currentIndex === 1 ? 2 : 0
            border.color: Theme.focusBorder
            Behavior on color { ColorAnimation { duration: 150 } }

            Text {
                anchors.centerIn: parent
                text: Theme.themeMode === "dark" ? "☽" :
                      Theme.themeMode === "light" ? "☀" : "◐"
                font.pixelSize: Theme.fontBody
                color: (themeMA.containsMouse || (root.activeFocus && root.currentIndex === 1))
                           ? Theme.textPrimary : Theme.textMuted
            }

            MouseArea {
                id: themeMA
                anchors.fill: parent
                hoverEnabled: true
                cursorShape: Qt.PointingHandCursor
                onClicked: {
                    if (Theme.themeMode === "auto") Theme.setThemeMode("light")
                    else if (Theme.themeMode === "light") Theme.setThemeMode("dark")
                    else Theme.setThemeMode("auto")
                }
            }
        }

        // Network (index 2)
        Rectangle {
            Layout.preferredWidth: root._iconSize
            Layout.preferredHeight: root._iconSize
            radius: root._iconSize / 2
            color: "transparent"
            border.width: root.activeFocus && root.currentIndex === 2 ? 2 : 0
            border.color: Theme.focusBorder

            Text {
                anchors.centerIn: parent
                text: root.ipAddress !== "..." && root.ipAddress !== "No IP" ? "⛁" : "⚠"
                font.pixelSize: Theme.fontBody
                color: root.ipAddress !== "..." && root.ipAddress !== "No IP"
                           ? Theme.textMuted : Theme.warning
            }
        }

        // Volume (index 3)
        Rectangle {
            Layout.preferredWidth: root._iconSize
            Layout.preferredHeight: root._iconSize
            radius: root._iconSize / 2
            color: "transparent"
            border.width: root.activeFocus && root.currentIndex === 3 ? 2 : 0
            border.color: Theme.focusBorder

            Text {
                anchors.centerIn: parent
                text: "♫"
                font.pixelSize: Theme.fontBody
                color: Theme.textMuted
            }
        }
    }
}
