import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import Quickshell
import Quickshell.Io

// System/About page (#128): displays OS, kernel, hostname, and uptime
// from the daemon's sys-status IPC command. Also shows storage free-space
// readout (folded in from the former standalone Storage page).
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
        refreshBtn.forceActiveFocus();
    }

    Process {
        id: dfProc
        command: ["bash", "-c", "df -h --output=target,size,used,avail,pcent 2>/dev/null | tail -n +2 | " + "awk 'NF>=5{printf \"{\\\"mount\\\":\\\"%s\\\",\\\"size\\\":\\\"%s\\\",\\\"used\\\":\\\"%s\\\",\\\"avail\\\":\\\"%s\\\",\\\"pct\\\":\\\"%s\\\"}\\n\",$1,$2,$3,$4,$5}'"]
        stdout: SplitParser {
            onRead: line => {
                try {
                    let obj = JSON.parse(line.trim());
                    // Skip pseudo and system filesystems
                    if (obj.mount && !obj.mount.startsWith("/sys") && !obj.mount.startsWith("/proc") && !obj.mount.startsWith("/dev/pts") && !obj.mount.startsWith("/run/user")) {
                        let arr = root.storageMounts.slice();
                        arr.push(obj);
                        root.storageMounts = arr;
                    }
                } catch (e) {}
            }
        }
        onExited: {
            root.storageLoading = false;
        }
    }

    Process {
        id: sysInfo
        command: ["bash", "-c", "printf '{\"os\":\"%s\",\"kernel\":\"%s\",\"hostname\":\"%s\",\"uptime\":\"%s\"}' " + "\"$(grep '^NAME=' /etc/os-release 2>/dev/null | cut -d= -f2 | tr -d '\"' || echo Unknown)\" " + "\"$(uname -r)\" " + "\"$(hostname)\" " + "\"$(uptime -p 2>/dev/null || uptime | sed 's/.*up //' | cut -d, -f1)\""]
        stdout: SplitParser {
            onRead: line => {
                try {
                    let obj = JSON.parse(line.trim());
                    root.osName = obj.os || "";
                    root.kernelVersion = obj.kernel || "";
                    root.hostname = obj.hostname || "";
                    root.uptime = obj.uptime || "";
                } catch (e) {}
                root.loading = false;
            }
        }
    }

    Component.onCompleted: {
        root.loading = true;
        sysInfo.running = true;
        root.storageLoading = true;
        dfProc.running = true;
    }

    ColumnLayout {
        id: contentColumn
        anchors.fill: parent
        anchors.margins: Theme.padding
        spacing: 48

        Text {
            text: "About This System"
            font.pixelSize: Theme.fontBody
            font.bold: true
            color: Theme.textPrimary
        }

        ColumnLayout {
            spacing: 24
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
                        Layout.preferredWidth: 300
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

        FocusScope {
            id: refreshBtn
            width: refreshBtnInner.implicitWidth
            height: refreshBtnInner.implicitHeight
            focus: true

            SettingsButton {
                id: refreshBtnInner
                text: "Refresh"
                activeFocusOnTab: false
                onActivated: {
                    root.loading = true;
                    sysInfo.running = true;
                }
            }

            Keys.onPressed: event => {
                if (event.key === Qt.Key_Return || event.key === Qt.Key_Space) {
                    root.loading = true;
                    sysInfo.running = true;
                    event.accepted = true;
                }
            }
        }

        // Storage — free-space readout
        Text {
            text: "Storage"
            font.pixelSize: Theme.fontBody
            font.bold: true
            color: Theme.textPrimary
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
                    size: "",
                    used: "",
                    avail: "",
                    pct: ""
                }
            ] : (root.storageMounts.length > 0 ? root.storageMounts : [
                    {
                        mount: "No filesystems found",
                        size: "",
                        used: "",
                        avail: "",
                        pct: ""
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
                        text: modelData.avail ? (modelData.avail + " free / " + modelData.size + " (" + modelData.pct + " used)") : ""
                        font.pixelSize: Theme.fontBody
                        color: Theme.textSecondary
                        Layout.fillWidth: true
                    }
                }
            }
        }

        FocusScope {
            id: storageRefreshBtn
            width: storageRefreshBtnInner.implicitWidth
            height: storageRefreshBtnInner.implicitHeight

            SettingsButton {
                id: storageRefreshBtnInner
                text: "Refresh"
                activeFocusOnTab: false
                onActivated: {
                    root.storageLoading = true;
                    root.storageMounts = [];
                    dfProc.running = true;
                }
            }

            Keys.onPressed: event => {
                if (event.key === Qt.Key_Return || event.key === Qt.Key_Space) {
                    root.storageLoading = true;
                    root.storageMounts = [];
                    dfProc.running = true;
                    event.accepted = true;
                }
            }
        }
    }
}
