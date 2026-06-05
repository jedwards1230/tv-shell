import QtQuick
import QtQuick.Layouts
import Quickshell
import Quickshell.Io

// System/About page (#128): displays OS, kernel, hostname, and uptime
// from the daemon's sys-status IPC command.
FocusScope {
    id: root

    property string osName: ""
    property string kernelVersion: ""
    property string hostname: ""
    property string uptime: ""
    property bool loading: false

    function focusFirst() {
        refreshBtn.forceActiveFocus();
    }

    Process {
        id: sysInfo
        command: ["bash", "-c",
            "printf '{\"os\":\"%s\",\"kernel\":\"%s\",\"hostname\":\"%s\",\"uptime\":\"%s\"}' " +
            "\"$(grep '^NAME=' /etc/os-release 2>/dev/null | cut -d= -f2 | tr -d '\"' || echo Unknown)\" " +
            "\"$(uname -r)\" " +
            "\"$(hostname)\" " +
            "\"$(uptime -p 2>/dev/null || uptime | sed 's/.*up //' | cut -d, -f1)\""]
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
    }

    ColumnLayout {
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
                    { label: "Operating System", value: root.loading ? "Loading…" : (root.osName || "Unknown") },
                    { label: "Kernel", value: root.loading ? "Loading…" : (root.kernelVersion || "Unknown") },
                    { label: "Hostname", value: root.loading ? "Loading…" : (root.hostname || "Unknown") },
                    { label: "Uptime", value: root.loading ? "Loading…" : (root.uptime || "Unknown") }
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

        Item { Layout.fillHeight: true }
    }
}
