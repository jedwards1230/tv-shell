import Quickshell.Io
import QtQuick

Item {
    id: root

    property bool systemOn: false
    property bool waking: false
    property string shellState: ""
    property bool _initialized: false

    function wake() {
        if (waking || avWake.running || avWakeCooldown.running) return
        if (!systemOn) waking = true
        avWake.running = true
        avWakeCooldown.restart()
    }

    function forceWake() {
        avWake.running = true
    }

    Process {
        id: avStatusCheck
        command: ["/usr/local/bin/living-room-cec", "status"]
        stdout: SplitParser {
            onRead: (line) => {
                var match = line.match(/^\s*(AVR)\s*:\s*(\S+)/i)
                if (match) {
                    root.systemOn = (match[2].toLowerCase() === "on")
                }
            }
        }
    }

    Timer {
        id: avStatusPoll
        interval: 30000
        running: root.shellState === "idle"
        repeat: true
        triggeredOnStart: true
        onTriggered: { if (!avStatusCheck.running) avStatusCheck.running = true }
    }

    Timer {
        id: avWakeCooldown
        interval: 30000
    }

    Process {
        id: avWake
        command: ["/usr/local/bin/living-room-cec", "on"]
        onExited: (exitCode) => {
            root.waking = false
            if (exitCode === 0 && !avStatusCheck.running)
                avStatusCheck.running = true
        }
    }

    Timer {
        id: avWakeTimeout
        interval: 20000
        running: root.waking
        onTriggered: { root.waking = false }
    }

    onSystemOnChanged: {
        if (!root._initialized) {
            root._initialized = true
            return
        }
        if (systemOn)
            NotificationManager.notify("AV System On", "", {icon: "📺", source: "av"})
        else
            NotificationManager.notify("AV System Off", "", {icon: "📺", source: "av"})
    }
}
