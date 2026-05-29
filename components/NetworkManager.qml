pragma Singleton
import QtQuick
import Quickshell.Io

// Shared network state. Lives as a singleton so multiple QuickActions rows
// (top-right status strip + navigation drawer) read the same IP without each
// polling independently or emitting duplicate connect/disconnect notifications.
Item {
    id: root

    property string ipAddress: "..."
    readonly property bool connected: ipAddress !== "..." && ipAddress !== "No IP"

    property bool _initialized: false
    property bool _wasConnected: false

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
        if (!_initialized) {
            _initialized = true;
            _wasConnected = connected;
            return;
        }
        if (_wasConnected && !connected) {
            NotificationManager.warn("network", "Network Disconnected");
        } else if (!_wasConnected && connected) {
            NotificationManager.info("network", "Network Connected", ipAddress);
        }
        _wasConnected = connected;
    }
}
