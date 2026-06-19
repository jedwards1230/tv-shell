import QtQuick
import QtQuick.Layouts
import "lib"

// Status-only Moonlight session indicator (icon + text). Renders whether the
// single configured Moonlight target currently has a live session to resume,
// driven by the daemon's `sunshine-status <host> <port>` probe (the same one the
// server cards use: {online,paired,currentApp}). A non-empty `currentApp` ⇒ "In
// session"; otherwise "No session" (muted).
//
// IMPORTANT: this is a glance affordance, not a control — it is NOT focusable,
// NOT selectable, has no key handler, and is skipped by the focus chain. It just
// reads state and paints it.
RowLayout {
    id: root

    // The single configured Moonlight target ({host, sunshinePort, ...}). Null ⇒
    // nothing to probe; the indicator hides.
    property var target: null

    spacing: Units.spacingSM
    visible: root.target !== null

    // True when the probe reports a live session on the target.
    property bool _inSession: false

    // Poll `sunshine-status` for the configured target. Anything other than an
    // `ok` reply with a non-empty/non-zero currentApp is treated as "no session"
    // (offline / unreachable / idle), matching the StreamCard probe semantics.
    ServiceMonitor {
        id: sessionMon
        dataCommand: "sunshine-status " + ((root.target && root.target.host) || "127.0.0.1") + " " + ((root.target && root.target.sunshinePort) || "47990")
        dataIntervalMs: 10000
        active: root.target !== null
        onUpdated: {
            if (!sessionMon.ok || !sessionMon.data) {
                root._inSession = false;
                return;
            }
            let gameId = String(sessionMon.data.currentApp || "");
            root._inSession = gameId !== "" && gameId !== "0";
        }
    }

    Text {
        text: root._inSession ? "●" : "○"  // ● filled / ○ hollow
        font.pixelSize: Theme.fontSmall
        color: root._inSession ? Theme.online : Theme.textMuted
        Layout.alignment: Qt.AlignVCenter
    }

    Text {
        text: root._inSession ? "In session" : "No session"
        font.pixelSize: Theme.fontBody
        font.bold: root._inSession
        color: root._inSession ? Theme.textPrimary : Theme.textMuted
        Layout.alignment: Qt.AlignVCenter
    }
}
