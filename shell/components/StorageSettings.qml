import QtQuick
import QtQuick.Layouts
import Quickshell
import Quickshell.Io

// Storage free-space page (#129): lists mounted filesystems and their
// available space via `df -h`.
FocusScope {
    id: root

    property var mounts: []
    property bool loading: false

    function focusFirst() {
        refreshBtn.forceActiveFocus();
    }

    Process {
        id: dfProc
        command: ["bash", "-c",
            "df -h --output=target,size,used,avail,pcent 2>/dev/null | tail -n +2 | " +
            "awk 'NF>=5{printf \"{\\\"mount\\\":\\\"%s\\\",\\\"size\\\":\\\"%s\\\",\\\"used\\\":\\\"%s\\\",\\\"avail\\\":\\\"%s\\\",\\\"pct\\\":\\\"%s\\\"}\\n\",$1,$2,$3,$4,$5}'"]
        stdout: SplitParser {
            onRead: line => {
                try {
                    let obj = JSON.parse(line.trim());
                    // Skip pseudo and system filesystems
                    if (obj.mount && !obj.mount.startsWith("/sys") && !obj.mount.startsWith("/proc") && !obj.mount.startsWith("/dev/pts") && !obj.mount.startsWith("/run/user")) {
                        let arr = root.mounts.slice();
                        arr.push(obj);
                        root.mounts = arr;
                    }
                } catch (e) {}
            }
        }
        onExited: {
            root.loading = false;
        }
    }

    Component.onCompleted: {
        root.loading = true;
        root.mounts = [];
        dfProc.running = true;
    }

    ColumnLayout {
        anchors.fill: parent
        anchors.margins: Theme.padding
        spacing: 48

        Text {
            text: "Storage"
            font.pixelSize: Theme.fontBody
            font.bold: true
            color: Theme.textPrimary
        }

        SettingsList {
            id: mountList
            Layout.fillWidth: true
            rowCount: root.loading ? 1 : Math.max(root.mounts.length, 1)

            Repeater {
                model: root.loading ? [{ mount: "Loading…", size: "", used: "", avail: "", pct: "" }] : (root.mounts.length > 0 ? root.mounts : [{ mount: "No filesystems found", size: "", used: "", avail: "", pct: "" }])

                delegate: RowLayout {
                    required property var modelData
                    Layout.fillWidth: true
                    spacing: 24
                    height: 80

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
                    root.mounts = [];
                    dfProc.running = true;
                }
            }

            Keys.onPressed: event => {
                if (event.key === Qt.Key_Return || event.key === Qt.Key_Space) {
                    root.loading = true;
                    root.mounts = [];
                    dfProc.running = true;
                    event.accepted = true;
                }
            }
        }

        Item { Layout.fillHeight: true }
    }
}
