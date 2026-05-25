import QtQuick
import QtQuick.Layouts
import Quickshell.Io

Row {
    id: root
    spacing: 24
    layoutDirection: Qt.RightToLeft

    signal settingsRequested()

    // IP address for network indicator
    property string ipAddress: "..."

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

    // Settings gear
    Text {
        text: "⚙"
        font.pixelSize: Theme.fontBody
        color: settingsMA.containsMouse ? Theme.textPrimary : Theme.textMuted
        opacity: settingsMA.containsMouse ? 1.0 : 0.6
        Behavior on opacity { NumberAnimation { duration: 150 } }
        Behavior on color { ColorAnimation { duration: 150 } }

        MouseArea {
            id: settingsMA
            anchors.fill: parent
            hoverEnabled: true
            cursorShape: Qt.PointingHandCursor
            onClicked: root.settingsRequested()
        }
    }

    // Theme mode toggle (auto / light / dark)
    Text {
        text: Theme.themeMode === "dark" ? "☽" :
              Theme.themeMode === "light" ? "☀" : "◐"
        font.pixelSize: Theme.fontBody
        color: themeMA.containsMouse ? Theme.textPrimary : Theme.textMuted
        opacity: themeMA.containsMouse ? 1.0 : 0.6
        Behavior on opacity { NumberAnimation { duration: 150 } }
        Behavior on color { ColorAnimation { duration: 150 } }

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

    // Network indicator
    Text {
        text: root.ipAddress !== "..." && root.ipAddress !== "No IP" ? "⛁" : "⚠"
        font.pixelSize: Theme.fontBody
        color: root.ipAddress !== "..." && root.ipAddress !== "No IP"
                   ? Theme.textMuted : Theme.warning
        opacity: 0.6
    }

    // Volume indicator
    Text {
        text: "♫"
        font.pixelSize: Theme.fontBody
        color: Theme.textMuted
        opacity: 0.6
    }
}
