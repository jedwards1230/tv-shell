pragma Singleton
import QtQuick
import Quickshell.Io

// Shared network state. Lives as a singleton so multiple QuickActions rows
// (top-right status strip + navigation drawer) read the same state without each
// polling independently or emitting duplicate connect/disconnect notifications.
//
// Source of truth is the daemon's `net-status` IPC + the live `net:connectivity`
// event stream — NOT `hostname -I` (#200). The old IP-string heuristic cached a
// "no network" state when `hostname -I` was empty/late at startup and never
// refreshed, so the home glyph showed "no network" while the box was fully
// online. We now read the daemon's NetworkManager-backed connectivity directly
// and re-query on every `net:*` event.
Item {
    id: root

    // Primary connection IPv4 (or "" when offline). Kept for the connect/
    // disconnect notification body and any address consumers.
    property string ipAddress: ""
    // Daemon connectivity word: "full" | "limited" | "portal" | "none" | "unknown".
    property string connectivity: "unknown"
    // Primary link type from the daemon (e.g. "802-3-ethernet", "802-11-wireless").
    property string primaryType: ""

    // Connected = the daemon reports FULL connectivity. limited/portal are a
    // captive-portal / degraded link, treated as not-fully-connected for the
    // home glyph (a wired box reports "full" — that was the #200 regression).
    readonly property bool connected: connectivity === "full"
    readonly property bool wired: primaryType === "802-3-ethernet"

    property bool _initialized: false
    property bool _wasConnected: false

    function refresh() {
        netStatus.request("net-status");
    }

    // --- net-status request/response (read-only daemon IPC) ---
    SocketClient {
        id: netStatus
        onResponseReceived: line => {
            try {
                let obj = JSON.parse(line);
                root.connectivity = obj.connectivity || "unknown";
                root.primaryType = obj.primaryType || "";
                root.ipAddress = obj.ipv4 || "";
            } catch (e) {
                console.log("NetworkManager: failed to parse net-status:", e);
            }
        }
    }

    // --- live net-change events ---
    // The daemon broadcasts net:connectivity:<state> / net:wifi:<json> /
    // net:primary:<id> on the subscribe stream (see docs/IPC_PROTOCOL.md). Any
    // net: line means the picture moved — re-query the authoritative snapshot.
    SocketClient {
        id: netEvents
        subscribe: true
        onLineReceived: line => {
            if (line.indexOf("net:") === 0)
                root.refresh();
        }
    }

    // Periodic backstop in case an event is missed; events drive the fast path.
    Timer {
        interval: 30000
        running: true
        repeat: true
        triggeredOnStart: true
        onTriggered: root.refresh()
    }

    Component.onCompleted: {
        netEvents.start();
        root.refresh();
    }

    // Connect/disconnect notifications, keyed off the connectivity transition
    // (not the IP string). The first resolution just seeds _wasConnected.
    onConnectedChanged: {
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
