import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import Quickshell
import Quickshell.Io
import "lib"

// System/About page (#128): displays OS, kernel, hostname, and uptime, plus a
// storage free-space readout (folded in from the former standalone Storage
// page). Values are now read from the daemon via IPC (#164):
//   sys-status     -> {os, kernel, hostname, uptime}
//   storage-status -> [{mount, size, used, avail, pct}, …]
FocusScope {
    id: root
    implicitHeight: contentColumn.implicitHeight + 2 * Theme.padding

    property string osName: ""
    property string kernelVersion: ""
    property string hostname: ""
    property string uptime: ""
    property bool loading: false

    property var storageMounts: []
    property bool storageLoading: false

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

    Component.onCompleted: {
        root.loading = true;
        getSysStatus.request("sys-status");
        root.storageLoading = true;
        getStorageStatus.request("storage-status");
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
        }
    }

    ColumnLayout {
        id: contentColumn
        anchors.fill: parent
        anchors.margins: Theme.padding
        spacing: 32

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
                radius: 16
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

        HintBar {
            text: "Updates automatically"
        }
    }
}
