import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import Quickshell
import Quickshell.Io
import "../components"
import "../components/lib"

// System/About page (#128): displays OS, kernel, hostname, and uptime, plus a
// storage free-space readout (folded in from the former standalone Storage
// page). Values are now read from the daemon via IPC (#164):
//   sys-status     -> {os, kernel, hostname, uptime}
//   storage-status -> [{mount, size, used, avail, pct}, …]
SettingsPageBase {
    id: root
    hintText: "Updates automatically"

    property string osName: ""
    property string kernelVersion: ""
    property string hostname: ""
    property string uptime: ""
    property bool loading: false

    property var storageMounts: []
    property bool storageLoading: false

    // Live hardware telemetry (#235) — driven by the daemon `sys-metrics` IPC.
    property real cpuPct: 0
    property real memUsed: 0
    property real memTotal: 0
    property int memPct: 0
    property real load1: 0
    property var temps: []
    property bool metricsLoaded: false

    function focusFirst() {
        // No interactive controls — the page auto-refreshes. Focus the page root
        // so B/Back still returns to the sidebar.
        root.forceActiveFocus();
    }

    // Daemon IPC — sys-status (#164)
    SocketClient {
        id: getSysStatus
        onResponseReceived: line => {
            try {
                let obj = JSON.parse(line);
                root.osName = obj.os || "";
                root.kernelVersion = obj.kernel || "";
                root.hostname = obj.hostname || "";
                root.uptime = obj.uptime || "";
            } catch (e) {
                console.log("SystemSettings: failed to parse sys-status:", e);
            }
            root.loading = false;
        }
        onRequestFailed: {
            root.loading = false;
        }
    }

    // Daemon IPC — storage-status (#164). Returns raw bytes; formatted in the
    // delegate as human-readable using Qt's locale number formatting.
    SocketClient {
        id: getStorageStatus
        onResponseReceived: line => {
            try {
                let mounts = JSON.parse(line);
                root.storageMounts = Array.isArray(mounts) ? mounts : [];
            } catch (e) {
                console.log("SystemSettings: failed to parse storage-status:", e);
                root.storageMounts = [];
            }
            root.storageLoading = false;
        }
        onRequestFailed: {
            root.storageLoading = false;
        }
    }

    // Daemon IPC — sys-metrics (#235). Live CPU/memory/load/temps; updates in
    // place via the 1s Timer below (no loading flash on periodic polls).
    SocketClient {
        id: getSysMetrics
        onResponseReceived: line => {
            try {
                let obj = JSON.parse(line);
                root.cpuPct = obj.cpuPct || 0;
                root.memUsed = obj.memUsed || 0;
                root.memTotal = obj.memTotal || 0;
                root.memPct = obj.memPct || 0;
                root.load1 = obj.load1 || 0;
                root.temps = Array.isArray(obj.temps) ? obj.temps : [];
                root.metricsLoaded = true;
            } catch (e) {
                console.log("SystemSettings: failed to parse sys-metrics:", e);
            }
        }
    }

    Component.onCompleted: {
        root.loading = true;
        getSysStatus.request("sys-status");
        root.storageLoading = true;
        getStorageStatus.request("storage-status");
        getSysMetrics.request("sys-metrics");
    }

    // Live refresh — re-poll every second while visible so uptime ticks and
    // everything else stays current. No loading flag on the periodic poll, so
    // values update in place without flashing "Loading…".
    Timer {
        interval: 1000
        repeat: true
        running: root.visible
        onTriggered: {
            getSysStatus.request("sys-status");
            getStorageStatus.request("storage-status");
            getSysMetrics.request("sys-metrics");
        }
    }

    // Format raw bytes as a human-readable GiB/MiB string (shared by the
    // Hardware and Storage sections). 0 → "0 B"; negative/unknown → "".
    function fmtBytes(n) {
        if (n < 0)
            return "";
        if (n === 0)
            return "0 B";
        if (n >= 1073741824)
            return (n / 1073741824).toFixed(1) + " GiB";
        if (n >= 1048576)
            return (n / 1048576).toFixed(1) + " MiB";
        return (n / 1024).toFixed(1) + " KiB";
    }

    // Single content column (a child of the base's content slot). It is NOT
    // anchors-filled — the SettingsPageBase scaffold supplies the page padding
    // and the trailing spacer + HintBar (via hintText).
    ColumnLayout {
        id: contentColumn
        Layout.fillWidth: true
        spacing: Units.spacingLG

        SectionHeader {
            text: "About This System"
        }

        ColumnLayout {
            spacing: 14
            Layout.fillWidth: true

            Repeater {
                model: [
                    {
                        label: "Operating System",
                        value: root.loading ? "Loading…" : (root.osName || "Unknown")
                    },
                    {
                        label: "Kernel",
                        value: root.loading ? "Loading…" : (root.kernelVersion || "Unknown")
                    },
                    {
                        label: "Hostname",
                        value: root.loading ? "Loading…" : (root.hostname || "Unknown")
                    },
                    {
                        label: "Uptime",
                        value: root.loading ? "Loading…" : (root.uptime || "Unknown")
                    }
                ]

                delegate: RowLayout {
                    required property var modelData
                    Layout.fillWidth: true
                    spacing: 24

                    Text {
                        text: modelData.label
                        font.pixelSize: Theme.fontBody
                        color: Theme.textSecondary
                        // minimumWidth guarantees the label column never shrinks
                        // into the value (was colliding: "Operating SystemCachyOS").
                        Layout.preferredWidth: 320
                        Layout.minimumWidth: 320
                    }

                    Text {
                        text: modelData.value
                        font.pixelSize: Theme.fontBody
                        color: Theme.textPrimary
                        Layout.fillWidth: true
                        wrapMode: Text.WordWrap
                    }
                }
            }
        }

        // Hardware — live CPU / memory / load / temperatures (#235)
        SectionHeader {
            text: "Hardware"
        }

        // Three stat cards side by side: CPU, Memory, Load. Each owns its bar in
        // its own row, so nothing overlaps and the section fills the width.
        RowLayout {
            Layout.fillWidth: true
            spacing: 16

            // --- CPU card ---
            StatCard {
                label: "CPU Usage"
                value: root.metricsLoaded ? root.cpuPct.toFixed(1) + "%" : "—"
                barProgress: root.cpuPct / 100
                barHighColor: root.cpuPct >= 90 ? Theme.crimson : Theme.ember
            }

            // --- Memory card ---
            StatCard {
                label: "Memory"
                value: root.metricsLoaded && root.memTotal > 0 ? root.memPct + "%" : "—"
                subtext: root.metricsLoaded && root.memTotal > 0 ? root.fmtBytes(root.memUsed) + " / " + root.fmtBytes(root.memTotal) : ""
                barProgress: root.memPct / 100
                barHighColor: root.memPct >= 90 ? Theme.crimson : Theme.ember
            }

            // --- Load card ---
            StatCard {
                label: "Load Average"
                value: root.metricsLoaded ? root.load1.toFixed(2) : "—"
                subtext: "1-minute average"
            }
        }

        // Temperatures — wrapping pills so the row fills the width and adapts to
        // however many sensors the host exposes (CPU/GPU sorted first).
        Text {
            text: "Temperatures"
            font.pixelSize: Theme.fontBody
            font.bold: true
            color: Theme.textSecondary
            visible: root.temps.length > 0
        }

        Flow {
            Layout.fillWidth: true
            spacing: 12

            Repeater {
                model: root.temps

                delegate: Rectangle {
                    required property var modelData
                    radius: 12
                    color: Theme.surface
                    border.width: 2
                    border.color: Theme.surfaceBorder
                    implicitWidth: tempPillRow.implicitWidth + 36
                    implicitHeight: 60

                    RowLayout {
                        id: tempPillRow
                        anchors.centerIn: parent
                        spacing: 14

                        Text {
                            text: modelData.label
                            font.pixelSize: Theme.fontSmall
                            color: Theme.textSecondary
                        }
                        Text {
                            text: modelData.celsius.toFixed(1) + " °C"
                            font.pixelSize: Theme.fontSmall
                            font.bold: true
                            color: modelData.celsius >= 90 ? Theme.crimson : Theme.textPrimary
                        }
                    }
                }
            }
        }

        Text {
            visible: root.metricsLoaded && root.temps.length === 0
            text: "No temperature sensors reported"
            font.pixelSize: Theme.fontSmall
            color: Theme.textMuted
        }

        // Storage — free-space readout
        SectionHeader {
            text: "Storage"
        }

        SettingsList {
            id: storageMountList
            rowStride: 88
            maxHeight: 600
            minRows: 1
            spacing: 8
            interactive: false
            model: root.storageLoading ? [
                {
                    mount: "Loading…",
                    size: 0,
                    used: 0,
                    avail: 0,
                    pct: 0
                }
            ] : (root.storageMounts.length > 0 ? root.storageMounts : [
                    {
                        mount: "No filesystems found",
                        size: 0,
                        used: 0,
                        avail: 0,
                        pct: 0
                    }
                ])

            delegate: Rectangle {
                required property var modelData
                width: parent ? parent.width : 0
                height: 80
                radius: Units.radiusMD
                color: Theme.surface
                border.width: 2
                border.color: Theme.surfaceBorder

                // Format raw bytes to a human-readable string (GiB/MiB/KiB).
                // Exactly 0 renders "0 B" (a legitimate value, e.g. a full
                // filesystem with 0 bytes free); negative/unknown renders "".
                function fmtBytes(n) {
                    if (n < 0)
                        return "";
                    if (n === 0)
                        return "0 B";
                    if (n >= 1073741824)
                        return (n / 1073741824).toFixed(1) + " GiB";
                    if (n >= 1048576)
                        return (n / 1048576).toFixed(1) + " MiB";
                    return (n / 1024).toFixed(1) + " KiB";
                }

                RowLayout {
                    anchors.fill: parent
                    anchors.leftMargin: 24
                    anchors.rightMargin: 24
                    spacing: 24

                    Text {
                        text: modelData.mount
                        font.pixelSize: Theme.fontBody
                        color: Theme.textPrimary
                        Layout.preferredWidth: 400
                        elide: Text.ElideRight
                    }

                    Text {
                        text: {
                            // Placeholder rows (Loading… / No filesystems found)
                            // carry size === 0; suppress the readout for those.
                            // A real filesystem always has a positive total.
                            if (modelData.size <= 0)
                                return "";
                            let avail = fmtBytes(modelData.avail);
                            let size = fmtBytes(modelData.size);
                            return avail + " free / " + size + " (" + modelData.pct + "% used)";
                        }
                        font.pixelSize: Theme.fontBody
                        color: Theme.textSecondary
                        Layout.fillWidth: true
                    }
                }
            }
        }
    }
}
