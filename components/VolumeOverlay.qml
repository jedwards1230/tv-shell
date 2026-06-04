import QtQuick
import QtQuick.Layouts
import Quickshell.Io

// Compact volume quick-overlay. Controller-navigable, 10-foot sized.
// Mirrors PowerOverlay/NotificationCenter pattern:
//   FocusScope + visible:opened + DimmedBackdrop + onOpenedChanged forces focus.
// Left/Right adjusts volume ±5%; A/Return toggles mute; Up/Down cycles output
// device; B/Escape closes.
FocusScope {
    id: root

    property bool opened: false

    // --- Audio state ---
    property int volume: 50
    property bool muted: false
    property var sinks: []
    property int defaultSinkIndex: -1
    property int _sinkCursor: 0    // which sink row has controller focus

    visible: opened
    anchors.fill: parent
    focus: opened

    onOpenedChanged: {
        if (opened) {
            root.forceActiveFocus();
            getVolume.running = true;
            listSinks.running = true;
        }
    }

    // --- wpctl processes (mirrored from AudioSettings.qml) ---

    Process {
        id: getVolume
        command: ["wpctl", "get-volume", "@DEFAULT_AUDIO_SINK@"]
        stdout: SplitParser {
            onRead: line => {
                let parts = line.trim().split(" ");
                if (parts.length >= 2)
                    root.volume = Math.round(parseFloat(parts[1]) * 100);
                root.muted = line.indexOf("[MUTED]") >= 0;
            }
        }
    }

    Process {
        id: setVolume
        property string level: "50%"
        command: ["wpctl", "set-volume", "@DEFAULT_AUDIO_SINK@", level]
        onExited: getVolume.running = true
    }

    Process {
        id: toggleMute
        command: ["wpctl", "set-mute", "@DEFAULT_AUDIO_SINK@", "toggle"]
        onExited: getVolume.running = true
    }

    Process {
        id: listSinks
        command: ["bash", "-c", "wpctl status | sed -n '/Audio/,/Video/p' | sed -n '/Sinks:/,/Sources:/p' | grep -v 'Sinks:\\|Sources:\\|^$'"]
        stdout: SplitParser {
            property var collected: []
            onRead: line => {
                let cleaned = line.replace(/[│├└─┐┘┌┬┴┤┼]/g, " ");
                let isDefault = cleaned.indexOf("*") >= 0;
                let match = cleaned.match(/\*?\s*(\d+)\.\s+(.+?)(?:\s+\[vol:.+\])?\s*$/);
                if (match) {
                    let entry = {
                        id: parseInt(match[1]),
                        name: match[2].trim(),
                        isDefault: isDefault
                    };
                    collected.push(entry);
                    if (isDefault)
                        root.defaultSinkIndex = collected.length - 1;
                }
            }
        }
        onExited: {
            root.sinks = listSinks.stdout.collected;
            listSinks.stdout.collected = [];
            // Sync cursor to default sink
            if (root.defaultSinkIndex >= 0)
                root._sinkCursor = root.defaultSinkIndex;
        }
    }

    Process {
        id: setDefaultSink
        property int sinkId: 0
        command: ["wpctl", "set-default", String(sinkId)]
        onExited: {
            listSinks.running = true;
            refreshTimer.start();
        }
    }

    Timer {
        id: refreshTimer
        interval: 500
        onTriggered: getVolume.running = true
    }

    // --- Key handling ---
    Keys.onPressed: event => {
        if (event.key === Qt.Key_Left) {
            root.volume = Math.max(0, root.volume - 5);
            setVolume.level = root.volume + "%";
            setVolume.running = true;
        } else if (event.key === Qt.Key_Right) {
            root.volume = Math.min(100, root.volume + 5);
            setVolume.level = root.volume + "%";
            setVolume.running = true;
        } else if (event.key === Qt.Key_Return || event.key === Qt.Key_Enter) {
            // A on a non-default sink row sets it as default; on volume bar toggles mute.
            if (root.sinks.length > 0 && root._sinkCursor >= 0 && !root.sinks[root._sinkCursor].isDefault) {
                setDefaultSink.sinkId = root.sinks[root._sinkCursor].id;
                setDefaultSink.running = true;
            } else {
                toggleMute.running = true;
            }
        } else if (event.key === Qt.Key_Up) {
            if (root.sinks.length > 0 && root._sinkCursor > 0)
                root._sinkCursor--;
        } else if (event.key === Qt.Key_Down) {
            if (root.sinks.length > 0 && root._sinkCursor < root.sinks.length - 1)
                root._sinkCursor++;
        } else if (event.key === Qt.Key_Escape || (event.key === Qt.Key_B && !event.modifiers)) {
            root.opened = false;
        }
        // Modal: block all keys from reaching items below.
        event.accepted = true;
    }

    // Backdrop
    DimmedBackdrop {
        dimLevel: 0.8
        onClicked: root.opened = false
    }

    // === Centered panel ===
    Rectangle {
        anchors.centerIn: parent
        width: Math.min(Units.gridUnit * 24, parent.width * 0.6)
        height: overlayColumn.implicitHeight + Units.gridUnit * 2
        radius: Units.radiusLG
        color: Theme.surface
        border.width: Units.borderMedium
        border.color: Theme.surfaceBorder

        ColumnLayout {
            id: overlayColumn
            anchors {
                left: parent.left
                right: parent.right
                top: parent.top
                margins: Units.gridUnit
            }
            spacing: Units.spacingLG

            // --- Title ---
            Text {
                Layout.alignment: Qt.AlignHCenter
                text: "Volume"
                font.pixelSize: Theme.fontTitle
                font.bold: true
                color: Theme.textPrimary
            }

            // --- Volume bar ---
            Rectangle {
                Layout.fillWidth: true
                height: Units.gridUnit * 1.5
                radius: height / 2
                color: Theme.surfaceHover

                Rectangle {
                    width: parent.width * (root.volume / 100)
                    height: parent.height
                    radius: parent.radius
                    color: root.muted ? Theme.textSecondary : (Theme.darkMode ? Theme.ember : Theme.navy)

                    Behavior on width {
                        NumberAnimation {
                            duration: 80
                        }
                    }
                }

                Text {
                    anchors.centerIn: parent
                    text: root.muted ? "MUTED" : root.volume + "%"
                    font.pixelSize: Theme.fontBody
                    font.bold: true
                    color: root.volume > 40 && !root.muted ? Theme.textOnDark : Theme.textPrimary
                }
            }

            // --- Mute toggle hint ---
            Text {
                Layout.alignment: Qt.AlignHCenter
                text: root.muted ? "▶  Press A to unmute" : "⏸  Press A to mute"
                font.pixelSize: Theme.fontHint
                color: root.muted ? Theme.warning : Theme.textMuted
            }

            // --- Output device list ---
            Rectangle {
                Layout.fillWidth: true
                height: 2
                color: Theme.surfaceBorder
                visible: root.sinks.length > 0
            }

            Text {
                visible: root.sinks.length > 0
                text: "Output Device"
                font.pixelSize: Theme.fontSmall
                font.bold: true
                color: Theme.textSecondary
            }

            Repeater {
                model: root.sinks

                Rectangle {
                    required property int index
                    required property var modelData
                    Layout.fillWidth: true
                    height: Units.gridUnit * 1.6
                    radius: Units.radiusMD
                    color: {
                        if (modelData.isDefault)
                            return Theme.sidebarActive;
                        if (root._sinkCursor === index && root.activeFocus)
                            return Theme.surfaceHover;
                        return Theme.cardBackground;
                    }
                    border.width: (modelData.isDefault || root._sinkCursor === index) ? Units.borderMedium : Units.borderThin
                    border.color: modelData.isDefault ? Theme.focusBorder : Theme.surfaceBorder

                    Behavior on color {
                        ColorAnimation {
                            duration: 100
                        }
                    }

                    RowLayout {
                        anchors {
                            fill: parent
                            leftMargin: Units.spacingLG
                            rightMargin: Units.spacingLG
                        }
                        spacing: Units.spacingSM

                        Text {
                            text: modelData.isDefault ? "▶" : " "
                            font.pixelSize: Theme.fontHint
                            color: Theme.focusBorder
                        }
                        Text {
                            Layout.fillWidth: true
                            text: modelData.name
                            font.pixelSize: Theme.fontHint
                            color: modelData.isDefault ? Theme.textOnDark : Theme.textPrimary
                            elide: Text.ElideRight
                        }
                        Text {
                            visible: root._sinkCursor === index && !modelData.isDefault
                            text: "A: select"
                            font.pixelSize: Theme.fontHint
                            color: Theme.textMuted
                        }
                    }

                    MouseArea {
                        anchors.fill: parent
                        hoverEnabled: true
                        cursorShape: Qt.PointingHandCursor
                        onEntered: root._sinkCursor = index
                        onClicked: {
                            root._sinkCursor = index;
                            setDefaultSink.sinkId = modelData.id;
                            setDefaultSink.running = true;
                        }
                    }
                }
            }

            // --- Hint bar ---
            Text {
                Layout.alignment: Qt.AlignHCenter
                text: "◀▶ Volume    A: Mute    ▲▼ Output    B: Close"
                font.pixelSize: Theme.fontHint
                color: Theme.textMuted
            }
        }
    }
}
