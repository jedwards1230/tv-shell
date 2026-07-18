import QtQuick
import QtQuick.Layouts
import "../components"
import "../components/lib"

// Server picker for the Steam widget's config page (Widgets ▸ Steam): shows
// WHICH tv-shell-host sidecar the daemon is actively checking and switches
// between the configured `[[steam.hosts]]` entries from the couch — no SSH.
// Reads the roster via the `steam-hosts` IPC ({status,active,hosts:[{name,
// host}]}); a selection fires `steam-set-host <name>` (persisted daemon-side
// as `steamServer` in settings.json), so the next `steam-library` poll and
// health probe — and the Wake card, which targets the reply's `host` — all
// follow the new server. Collapses entirely when the daemon reports no hosts
// (widget unconfigured or daemon unreachable); with a single host it is a
// read-only confirmation of what's being checked.
FocusScope {
    id: root

    // The control above this section (the last manifest control row) — Up from
    // the chips returns focus there. Wired by WidgetConfig's Loader, mirroring
    // MoonlightSettings' embedded upTarget.
    property Item upTarget: null

    // {name, host} roster + active name from the latest `steam-hosts` reply.
    property var hosts: []
    property string active: ""
    readonly property bool available: root.hosts.length > 0

    visible: root.available
    implicitHeight: root.available ? col.implicitHeight : 0

    function refresh() {
        hostsReq.request("steam-hosts");
    }
    Component.onCompleted: root.refresh()

    // The active entry's host part (IP/hostname), for the caption.
    function _activeHostAddr() {
        for (var i = 0; i < root.hosts.length; i++) {
            if (root.hosts[i].name === root.active)
                return root.hosts[i].host || "";
        }
        return "";
    }

    Keys.onUpPressed: if (root.upTarget)
        root.upTarget.forceActiveFocus()

    SocketClient {
        id: hostsReq
        onResponseReceived: response => {
            let d = null;
            try {
                d = JSON.parse(response);
            } catch (e) {
                console.warn("[SteamServerSettings] unparseable steam-hosts reply");
                return;
            }
            if (!d || d.status !== "ok") {
                root.hosts = [];
                root.active = "";
                return;
            }
            root.hosts = d.hosts || [];
            root.active = d.active || "";
        }
        onRequestFailed: console.warn("[SteamServerSettings] steam-hosts request failed")
    }

    SocketClient {
        id: setReq
        // Re-read the roster so `active` reflects what the daemon persisted
        // (an `error:*` reply — e.g. a raced config reload — reverts the
        // optimistic selection below).
        onResponseReceived: () => root.refresh()
        onRequestFailed: console.warn("[SteamServerSettings] steam-set-host request failed")
    }

    ColumnLayout {
        id: col
        anchors.left: parent.left
        anchors.right: parent.right
        spacing: Units.spacingSM

        RowLayout {
            Layout.fillWidth: true
            spacing: Units.spacingLG

            Text {
                text: "Server"
                font.pixelSize: Theme.fontBody
                color: Theme.textSecondary
                Layout.alignment: Qt.AlignVCenter
                Layout.preferredWidth: Units.gridUnit * 8
            }

            SettingsButtonGroup {
                id: chips
                focus: true
                Layout.alignment: Qt.AlignVCenter
                options: {
                    let out = [];
                    for (var i = 0; i < root.hosts.length; i++)
                        out.push({
                            "label": root.hosts[i].name || root.hosts[i].host || "?",
                            "value": root.hosts[i].name
                        });
                    return out;
                }
                isCurrentOption: opt => opt.value === root.active
                onValueSelected: opt => {
                    // Optimistic: reflect the choice immediately; the set reply
                    // triggers a roster re-read that confirms (or reverts) it.
                    root.active = opt.value;
                    setReq.request("steam-set-host", opt.value);
                }
            }

            Item {
                Layout.fillWidth: true
            }
        }

        Text {
            // e.g. "Checking desktop-2 (192.168.8.153)" — the address is what
            // the Wake card will target while this server is down.
            text: root.active !== "" ? ("Checking " + root.active + (root._activeHostAddr() !== "" ? " (" + root._activeHostAddr() + ")" : "")) : ""
            visible: root.active !== ""
            font.pixelSize: Theme.fontSmall
            color: Theme.textMuted
            Layout.leftMargin: Units.gridUnit * 8 + Units.spacingLG
        }
    }
}
