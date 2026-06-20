import QtQuick
import Quickshell.Io
import "lib"

BaseCard {
    id: root

    required property var target
    property string appName: ""
    property bool isOnline: false
    property string shellState: "idle"
    property bool hasActiveSession: false
    property string activeAppName: ""
    // When true (home server cards), append the default profile (`target.app`)
    // to the server name — e.g. "Desktop — Steam Big Picture" — so the card shows
    // what A launches. Suppressed when the profile matches the name (no
    // "Desktop — Desktop"). Per-app Library cards (appName set) are unaffected.
    property bool showProfile: false

    label: root.appName !== "" ? root.appName : ((root.target.name || "Unknown") + (root.showProfile && root.target.app && root.target.app !== root.target.name ? " — " + root.target.app : ""))

    // activeAppName comes from the remote Sunshine server — sanitize before it
    // reaches the AT-SPI description: strip control chars and cap length so a
    // hostile/garbled server name can't inject into the accessibility tree (#112).
    readonly property string _safeActiveAppName: root.activeAppName.replace(/[\x00-\x1f]/g, "").substring(0, 80)
    Accessible.description: (root.isOnline ? "Online" : "Offline") + (root.hasActiveSession ? ", session active: " + root._safeActiveAppName : "")

    // Reachability via the daemon's net-ping IPC (count 1). Fail-soft: an
    // unreachable host returns reachable:false — the card just shows Offline.
    SocketClient {
        id: pingCheck
        onResponseReceived: line => {
            try {
                root.isOnline = JSON.parse(line).reachable === true;
            } catch (e) {
                root.isOnline = false;
            }
        }
        onRequestFailed: root.isOnline = false
    }

    Timer {
        interval: 10000
        running: true
        repeat: true
        triggeredOnStart: true
        onTriggered: pingCheck.request("net-ping", root.target.host || "127.0.0.1")
    }

    // Resolve the running app name via the daemon's `sunshine-status` IPC command
    // (see docs/IPC_PROTOCOL.md) + the local Moonlight config. The daemon owns the
    // Sunshine /serverinfo HTTPS fetch and returns {status,online,paired,currentApp,...},
    // where currentApp is the busy game id (or "" when idle). The per-card poll
    // runs through a `ServiceMonitor` (poll mode, on the shared service-health
    // vocabulary); the game-id -> friendly-name resolution stays in MoonlightConf,
    // preserving the prior contract (idle / a resolved name / "Unknown App").
    property string _pendingGameId: ""

    MoonlightConf {
        id: moonlightConf
        onLoaded: {
            // Conf (re)loaded after a busy reply we couldn't resolve yet.
            if (root._pendingGameId !== "") {
                let name = moonlightConf.nameFor(root._pendingGameId);
                root.hasActiveSession = true;
                root.activeAppName = name !== "" ? name : "Unknown App";
                root._pendingGameId = "";
            }
        }
    }

    // Sunshine session poll. Poll mode (no healthKey): every 10s while the card
    // is idle and its host pings, fetch `sunshine-status` and resolve the busy
    // game id to a friendly name. `active` gates polling exactly as the old
    // hand-rolled Timer did; anything other than an `ok` reply is treated as
    // idle (offline / unreachable / malformed), matching the prior fallback.
    ServiceMonitor {
        id: sessionMon
        dataCommand: "sunshine-status " + (root.target.host || "127.0.0.1") + " " + (root.target.sunshinePort || "47990")
        dataIntervalMs: 10000
        active: root.shellState === "idle" && root.isOnline
        onUpdated: {
            if (!sessionMon.ok || !sessionMon.data) {
                root.hasActiveSession = false;
                root.activeAppName = "";
                return;
            }
            let gameId = String(sessionMon.data.currentApp || "");
            if (gameId === "" || gameId === "0") {
                root.hasActiveSession = false;
                root.activeAppName = "";
                return;
            }
            // Busy: resolve the id to a name. Try the cached map first; if it is
            // empty/unloaded, (re)read the conf and resolve in onLoaded.
            let name = moonlightConf.nameFor(gameId);
            if (name !== "" || moonlightConf._loaded) {
                root.hasActiveSession = true;
                root.activeAppName = name !== "" ? name : "Unknown App";
            } else {
                root._pendingGameId = gameId;
                moonlightConf.load();
            }
        }
    }

    Text {
        anchors.fill: parent
        text: root.appName !== "" ? root.appName.charAt(0).toUpperCase() : "\u{1F3AE}"
        font.pixelSize: 96
        font.bold: root.appName !== ""
        color: root.appName !== "" ? Theme.textSecondary : Theme.textPrimary
        horizontalAlignment: Text.AlignHCenter
        verticalAlignment: Text.AlignVCenter
    }

    Rectangle {
        id: sessionBadge
        visible: root.hasActiveSession && (root.appName === "" || root.activeAppName === root.appName || root.activeAppName === (root.target.app || ""))
        height: 24
        radius: 12
        color: Theme.online
        // a11y: width grows leftward to fit icon glyph + label; anchor stays top-right
        width: badgeRow.implicitWidth + 16
        anchors.top: parent.top
        anchors.right: parent.right
        anchors.topMargin: 8
        anchors.rightMargin: 8

        // a11y: icon glyph + text label — state conveyed via shape AND label,
        // not green fill alone (colorblind-safe dual cue).
        Row {
            id: badgeRow
            anchors.centerIn: parent
            spacing: 4

            Text {
                text: "●"
                font.pixelSize: 10
                color: "#ffffff"
                anchors.verticalCenter: parent.verticalCenter
            }

            Text {
                id: badgeText
                text: "LIVE"
                font.pixelSize: 12
                font.bold: true
                color: "#ffffff"
                anchors.verticalCenter: parent.verticalCenter
            }
        }
    }
}
