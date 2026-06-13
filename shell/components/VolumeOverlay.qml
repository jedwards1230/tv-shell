import QtQuick
import QtQuick.Layouts
import Quickshell.Io
import "lib"

// Compact volume quick-popover, anchored beside the originating QuickAction
// glyph (#118). Controller-navigable, 10-foot sized. Mirrors NetworkOverlay's
// anchored-popover pattern: FocusScope + visible:opened + light scrim +
// onOpenedChanged forces focus; the panel positions itself relative to
// anchorRect (scene-root coords) and clamps fully on-screen.
//   Left/Right adjusts volume ±5%; A/Return toggles mute (or selects a sink
//   when the output list is expanded); Down expands the output list, Up/Down
//   move within it; B/Escape closes (collapses the list first if expanded).
FocusScope {
    id: root

    property bool opened: false

    // Scene-root rect {x, y, w, h} of the glyph that opened this popover.
    property var anchorRect: null

    // --- Audio state ---
    property int volume: 50
    property bool muted: false
    property var sinks: []
    property int defaultSinkIndex: -1

    // Output list collapsed by default (keeps the popover compact); expands to
    // the full sink switcher on demand — and only ever opens on A (never on a
    // directional key), matching the frontend dropdown rule.
    property bool _outputExpanded: false
    property int _sinkCursor: 0    // which sink row has controller focus
    // Which control row has focus in compact mode: 0 = volume bar (Left/Right
    // adjusts, A mutes), 1 = output selector (A expands). Directional keys only
    // MOVE focus between rows; they never open the dropdown.
    property int _focusRow: 0

    visible: opened
    anchors.fill: parent
    focus: opened

    function openAt(rect) {
        root.anchorRect = rect;
        root.opened = true;
        Qt.callLater(function () {
            root.forceActiveFocus();
        });
    }

    onOpenedChanged: {
        if (opened) {
            Qt.callLater(function () {
                root.forceActiveFocus();
            });
            root._outputExpanded = false;
            root._focusRow = 0;
            getVolume.running = true;
            listSinks.running = true;
        }
    }

    function _currentSinkName() {
        if (root.defaultSinkIndex >= 0 && root.defaultSinkIndex < root.sinks.length)
            return root.sinks[root.defaultSinkIndex].name;
        return "No output device";
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
        if (root._outputExpanded) {
            // Output list mode: Up/Down move cursor, A selects, B/Left collapses.
            if (event.key === Qt.Key_Up) {
                if (root._sinkCursor > 0)
                    root._sinkCursor--;
            } else if (event.key === Qt.Key_Down) {
                if (root._sinkCursor < root.sinks.length - 1)
                    root._sinkCursor++;
            } else if (event.key === Qt.Key_Return || event.key === Qt.Key_Enter) {
                if (root._sinkCursor >= 0 && root._sinkCursor < root.sinks.length) {
                    setDefaultSink.sinkId = root.sinks[root._sinkCursor].id;
                    setDefaultSink.running = true;
                }
                root._outputExpanded = false;
            } else if (event.key === Qt.Key_Left || event.key === Qt.Key_Escape || (event.key === Qt.Key_B && !event.modifiers)) {
                root._outputExpanded = false;
            }
        } else {
            // Compact mode. Directional keys only MOVE focus between rows; the
            // output dropdown opens on A only (never on a directional key).
            if (event.key === Qt.Key_Down) {
                // Move focus to the output selector row (if there is one).
                if (root._focusRow === 0 && root.sinks.length > 0)
                    root._focusRow = 1;
            } else if (event.key === Qt.Key_Up) {
                // Move focus back to the volume bar.
                if (root._focusRow === 1)
                    root._focusRow = 0;
            } else if (event.key === Qt.Key_Left) {
                if (root._focusRow === 0) {
                    root.volume = Math.max(0, root.volume - 5);
                    setVolume.level = root.volume + "%";
                    setVolume.running = true;
                }
            } else if (event.key === Qt.Key_Right) {
                if (root._focusRow === 0) {
                    root.volume = Math.min(100, root.volume + 5);
                    setVolume.level = root.volume + "%";
                    setVolume.running = true;
                }
            } else if (event.key === Qt.Key_Return || event.key === Qt.Key_Enter) {
                if (root._focusRow === 1) {
                    // A on the focused output selector → open the dropdown.
                    if (root.sinks.length > 0) {
                        root._sinkCursor = root.defaultSinkIndex >= 0 ? root.defaultSinkIndex : 0;
                        root._outputExpanded = true;
                    }
                } else {
                    // A on the volume bar → toggle mute.
                    toggleMute.running = true;
                }
            } else if (event.key === Qt.Key_Escape || (event.key === Qt.Key_B && !event.modifiers)) {
                root.opened = false;
            }
        }
        // Modal: block all keys from reaching items below.
        event.accepted = true;
    }

    AnchoredPopover {
        anchorRect: root.anchorRect
        panelWidth: Units.gridUnit * 22
        onDismissed: root.opened = false

        // --- Title ---
        Text {
            Layout.alignment: Qt.AlignHCenter
            text: "Volume"
            font.pixelSize: Theme.fontBody
            font.bold: true
            color: Theme.textPrimary
        }

        // --- Volume bar ---
        Rectangle {
            Layout.fillWidth: true
            height: Units.gridUnit * 1.5
            radius: height / 2
            color: Theme.surfaceHover
            // Focus ring when the volume bar row has controller focus.
            border.width: root.activeFocus && root._focusRow === 0 && !root._outputExpanded ? Units.borderMedium : 0
            border.color: Theme.focusBorder

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
                font.pixelSize: Theme.fontHint
                font.bold: true
                color: root.volume > 40 && !root.muted ? Theme.textOnDark : Theme.textPrimary
            }
        }

        // --- Mute toggle hint ---
        Text {
            Layout.alignment: Qt.AlignHCenter
            visible: !root._outputExpanded && root._focusRow === 0
            text: root.muted ? "A: unmute" : "A: mute"
            font.pixelSize: Theme.fontHint
            color: root.muted ? Theme.warning : Theme.textMuted
        }

        // --- Output: collapsed current row (default) ---
        Rectangle {
            id: outputRow
            Layout.fillWidth: true
            height: Units.gridUnit * 1.6
            radius: Units.radiusMD
            visible: !root._outputExpanded && root.sinks.length > 0

            // Highlighted only when the output selector row has focus.
            readonly property bool rowFocused: root.activeFocus && root._focusRow === 1 && !root._outputExpanded

            color: rowFocused ? Theme.surfaceHover : Theme.cardBackground
            border.width: rowFocused ? Units.borderMedium : Units.borderThin
            border.color: rowFocused ? Theme.focusBorder : Theme.surfaceBorder

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
                    Layout.fillWidth: true
                    text: "Output: " + root._currentSinkName()
                    font.pixelSize: Theme.fontHint
                    color: Theme.textPrimary
                    elide: Text.ElideRight
                }
                Text {
                    visible: outputRow.rowFocused
                    text: "A: switch"
                    font.pixelSize: Theme.fontHint
                    color: Theme.textMuted
                }
                Text {
                    text: "▾"
                    font.pixelSize: Theme.fontHint
                    color: Theme.textSecondary
                }
            }

            MouseArea {
                anchors.fill: parent
                hoverEnabled: true
                cursorShape: Qt.PointingHandCursor
                onClicked: {
                    // A click opens the dropdown (open-on-click rule).
                    if (root.sinks.length > 0) {
                        root._focusRow = 1;
                        root._sinkCursor = root.defaultSinkIndex >= 0 ? root.defaultSinkIndex : 0;
                        root._outputExpanded = true;
                    }
                }
            }
        }

        // --- Output: expanded sink list ---
        Text {
            visible: root._outputExpanded
            text: "Output Device"
            font.pixelSize: Theme.fontHint
            font.bold: true
            color: Theme.textSecondary
        }

        Repeater {
            model: root._outputExpanded ? root.sinks : []

            Rectangle {
                required property int index
                required property var modelData
                Layout.fillWidth: true
                height: Units.gridUnit * 1.5
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
                }

                MouseArea {
                    anchors.fill: parent
                    hoverEnabled: true
                    cursorShape: Qt.PointingHandCursor
                    onEntered: root._sinkCursor = index
                    onClicked: {
                        setDefaultSink.sinkId = modelData.id;
                        setDefaultSink.running = true;
                        root._outputExpanded = false;
                    }
                }
            }
        }

        // --- Hint bar ---
        Text {
            Layout.alignment: Qt.AlignHCenter
            text: {
                if (root._outputExpanded)
                    return "▲▼ Select    A: Switch    B: Back";
                if (root._focusRow === 1)
                    return "A: Open output    ▲ Back    B: Close";
                return "◀▶ Volume    A: Mute    ▼ Output    B: Close";
            }
            font.pixelSize: Theme.fontHint
            color: Theme.textMuted
        }
    }
}
