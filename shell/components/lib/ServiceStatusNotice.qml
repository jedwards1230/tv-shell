import QtQuick
import QtQuick.Layouts
import "../"

// Graceful "server-side issue" placeholder for a widget whose remote service is
// configured but not serving. Pairs with `ServiceMonitor`: a widget shows this
// instead of collapsing to nothing when `monitor.degraded` is true, so the user
// learns the server is down rather than wondering why the widget vanished.
//
// Visible only for the degraded states (`unreachable` / `error`); `ok` and
// `disabled` are the host's responsibility (render data / collapse).
RowLayout {
    id: notice

    // Human-facing service name, e.g. "Plex".
    property string serviceName: "Service"
    // A `ServiceMonitor.status` value.
    property string status: "unreachable"

    readonly property bool _unreachable: status === "unreachable"
    readonly property color _accent: _unreachable ? Theme.offline : Theme.gold

    visible: status === "unreachable" || status === "error"
    spacing: Units.spacingMD

    // Status dot — matches the StatusPill "bad"/"warn" colour language.
    Rectangle {
        Layout.preferredWidth: Math.round(Theme.fontBody * 0.7)
        Layout.preferredHeight: Math.round(Theme.fontBody * 0.7)
        Layout.alignment: Qt.AlignVCenter
        radius: width / 2
        color: notice._accent
    }

    ColumnLayout {
        spacing: Units.spacingXS

        Text {
            text: notice.serviceName + " unavailable"
            font.pixelSize: Theme.fontBody
            font.bold: true
            color: Theme.textPrimary
        }

        Text {
            // Unreachable → transient/server-side; error → needs attention.
            text: notice._unreachable ? "The server isn't responding right now." : "Couldn't authenticate — check the configuration."
            font.pixelSize: Theme.fontSmall
            color: Theme.textSecondary
        }
    }
}
