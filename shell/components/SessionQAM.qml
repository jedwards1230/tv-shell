import QtQuick
import QtQuick.Layouts

// SessionQAM — right-edge "Quick Access Menu" drawer (#218, epic #133 Phase 2).
//
// A true QAM: a right-side drawer with a 3-tab bar (Now Playing · Quick
// Settings · Notifications), opened by the `overlay:session` intent. "B is
// back one level" (content → tab bar → close), matching the Drawer base.
//
// Design deviation from #133's locked mock (grounded in the daemon input
// model): the mock specs LB/RB shoulder tabs, but in shell mode the daemon
// binds LB/RB to mouse left/right click (input.rs handle_shell), so the
// shoulders never reach QML as key events. Tabs switch on Left/Right while the
// tab bar row is focused instead — the same _focusRow idiom VolumeOverlay uses.
//
// The audio half (volume + mute + output sink switcher) is lifted verbatim
// from VolumeOverlay/AudioSettings (the wpctl Process blocks); the Audio Output
// hero card is the expand-in-place sink list. Now-Playing reuses MediaWidget.
Drawer {
    id: root
    edge: "right"
    drawerWidth: Units.gridUnit * 22

    // Host opens the Notification Center (reuses the existing surface rather
    // than reimplementing a list inside the QAM).
    signal notificationCenterRequested

    // 0 = Now Playing, 1 = Quick Settings, 2 = Notifications
    property int _tab: 1
    // 0 = tab bar focused, 1 = tab content focused
    property int _focusRow: 0

    // --- Quick Settings UI state ---
    property bool _outputExpanded: false
    property int _sinkCursor: 0
    // Quick Settings sub-row: 0 = volume bar, 1 = output selector.
    property int _csRow: 0

    readonly property var _tabs: [
        {
            label: "Now Playing",
            icon: "♪"
        },
        {
            label: "Quick Settings",
            icon: "⚙"
        },
        {
            label: "Notifications",
            icon: "◉"
        }
    ]

    // Open the QAM at the default (Quick Settings) tab with the tab bar focused.
    function open() {
        root._tab = 1;
        root._focusRow = 0;
        root._outputExpanded = false;
        root._csRow = 0;
        root.opened = true;
        audioCtl.refresh();
        Qt.callLater(() => contentRoot.forceActiveFocus());
    }

    // Close via the same path as B/Escape (the host's onClosed unmaps + restores
    // focus). Lets the trigger intent toggle the QAM shut, not just open it.
    function close() {
        root.closed();
    }

    onOpenedChanged: {
        // open() owns the state reset + process starts; this only restores focus
        // as a safety net if `opened` is ever set true without going through it.
        if (opened)
            Qt.callLater(() => contentRoot.forceActiveFocus());
    }

    // Move focus from the tab bar into the active tab's content.
    function _enterContent() {
        if (root._tab === 0) {
            // Now Playing: hand focus to MediaWidget only if there's a player.
            if (mediaWidget.hasPlayer) {
                root._focusRow = 1;
                mediaWidget.forceActiveFocus();
            }
        } else if (root._tab === 1) {
            root._focusRow = 1;
            root._csRow = 0;
            contentRoot.forceActiveFocus();
        } else {
            root._focusRow = 1;
            contentRoot.forceActiveFocus();
        }
    }

    // Back one level: content → tab bar.
    function _returnToTabs() {
        root._outputExpanded = false;
        root._focusRow = 0;
        contentRoot.forceActiveFocus();
    }

    AudioController {
        id: audioCtl
        onSinkCursorSync: idx => {
            root._sinkCursor = idx;
        }
    }

    // === Content ===
    Item {
        id: contentRoot
        anchors.fill: parent
        focus: true

        Keys.onPressed: event => {
            if (root._focusRow === 0) {
                // --- Tab bar ---
                if (event.key === Qt.Key_Left) {
                    root._tab = Math.max(0, root._tab - 1);
                } else if (event.key === Qt.Key_Right) {
                    root._tab = Math.min(root._tabs.length - 1, root._tab + 1);
                } else if (event.key === Qt.Key_Down || event.key === Qt.Key_Return || event.key === Qt.Key_Enter) {
                    root._enterContent();
                } else if (event.key === Qt.Key_Escape || (event.key === Qt.Key_B && !event.modifiers)) {
                    root.closed();
                }
            } else if (root._tab === 1) {
                // --- Quick Settings content ---
                if (root._outputExpanded) {
                    if (event.key === Qt.Key_Up) {
                        if (root._sinkCursor > 0)
                            root._sinkCursor--;
                    } else if (event.key === Qt.Key_Down) {
                        if (root._sinkCursor < audioCtl.sinks.length - 1)
                            root._sinkCursor++;
                    } else if (event.key === Qt.Key_Return || event.key === Qt.Key_Enter) {
                        if (root._sinkCursor >= 0 && root._sinkCursor < audioCtl.sinks.length) {
                            audioCtl.setDefaultSinkById(audioCtl.sinks[root._sinkCursor].id);
                        }
                        root._outputExpanded = false;
                    } else if (event.key === Qt.Key_Left || event.key === Qt.Key_Escape || (event.key === Qt.Key_B && !event.modifiers)) {
                        root._outputExpanded = false;
                    }
                } else if (root._csRow === 0) {
                    // Volume bar row.
                    if (event.key === Qt.Key_Left) {
                        audioCtl.setVolumeLevel(audioCtl.volume - 5);
                    } else if (event.key === Qt.Key_Right) {
                        audioCtl.setVolumeLevel(audioCtl.volume + 5);
                    } else if (event.key === Qt.Key_Return || event.key === Qt.Key_Enter) {
                        audioCtl.toggleMuteState();
                    } else if (event.key === Qt.Key_Down) {
                        if (audioCtl.sinks.length > 0)
                            root._csRow = 1;
                    } else if (event.key === Qt.Key_Up) {
                        root._returnToTabs();
                    } else if (event.key === Qt.Key_Escape || (event.key === Qt.Key_B && !event.modifiers)) {
                        root._returnToTabs();
                    }
                } else {
                    // Output selector row.
                    if (event.key === Qt.Key_Return || event.key === Qt.Key_Enter) {
                        if (audioCtl.sinks.length > 0) {
                            root._sinkCursor = audioCtl.defaultSinkIndex >= 0 ? audioCtl.defaultSinkIndex : 0;
                            root._outputExpanded = true;
                        }
                    } else if (event.key === Qt.Key_Up) {
                        root._csRow = 0;
                    } else if (event.key === Qt.Key_Escape || (event.key === Qt.Key_B && !event.modifiers)) {
                        root._returnToTabs();
                    }
                }
            } else if (root._tab === 2) {
                // --- Notifications content ---
                if (event.key === Qt.Key_Return || event.key === Qt.Key_Enter) {
                    root.notificationCenterRequested();
                    root.closed();
                } else if (event.key === Qt.Key_Up) {
                    root._returnToTabs();
                } else if (event.key === Qt.Key_Escape || (event.key === Qt.Key_B && !event.modifiers)) {
                    root._returnToTabs();
                }
            }
            // Modal within the drawer panel.
            event.accepted = true;
        }

        ColumnLayout {
            anchors.fill: parent
            anchors.margins: Units.gridUnit
            spacing: Units.spacingLG

            // === Header ===
            Text {
                text: "Quick Access"
                font.pixelSize: Theme.fontTitle
                font.bold: true
                color: Theme.textPrimary
            }

            // === Tab bar ===
            RowLayout {
                Layout.fillWidth: true
                spacing: Units.spacingSM

                Repeater {
                    model: root._tabs
                    Rectangle {
                        required property int index
                        required property var modelData
                        Layout.fillWidth: true
                        height: Units.gridUnit * 2
                        radius: Units.radiusMD
                        readonly property bool active: root._tab === index
                        readonly property bool barFocused: active && root._focusRow === 0
                        color: active ? Theme.sidebarActive : Theme.cardBackground
                        border.width: barFocused ? Units.borderMedium : Units.borderThin
                        border.color: barFocused ? Theme.focusBorder : Theme.surfaceBorder

                        Behavior on color {
                            ColorAnimation {
                                duration: 100
                            }
                        }

                        ColumnLayout {
                            anchors.centerIn: parent
                            spacing: 0
                            Text {
                                Layout.alignment: Qt.AlignHCenter
                                text: modelData.icon
                                font.pixelSize: Theme.fontBody
                                color: active ? Theme.textOnDark : Theme.textSecondary
                            }
                            Text {
                                Layout.alignment: Qt.AlignHCenter
                                text: modelData.label
                                font.pixelSize: Theme.fontHint
                                color: active ? Theme.textOnDark : Theme.textMuted
                                elide: Text.ElideRight
                            }
                        }

                        // Unread badge on the Notifications tab.
                        Rectangle {
                            visible: index === 2 && NotificationManager.unreadCount > 0
                            anchors.top: parent.top
                            anchors.right: parent.right
                            anchors.margins: Units.spacingXS
                            width: Units.gridUnit
                            height: Units.gridUnit
                            radius: width / 2
                            color: Theme.warning
                            Text {
                                anchors.centerIn: parent
                                text: NotificationManager.unreadCount
                                font.pixelSize: Theme.fontHint
                                font.bold: true
                                color: Theme.textOnDark
                            }
                        }

                        MouseArea {
                            anchors.fill: parent
                            cursorShape: Qt.PointingHandCursor
                            onClicked: {
                                root._tab = index;
                                root._focusRow = 0;
                                contentRoot.forceActiveFocus();
                            }
                        }
                    }
                }
            }

            Rectangle {
                Layout.fillWidth: true
                height: Units.borderThin
                color: Theme.surfaceBorder
            }

            // === Tab content ===
            // --- Now Playing ---
            ColumnLayout {
                Layout.fillWidth: true
                visible: root._tab === 0
                spacing: Units.spacingMD

                MediaWidget {
                    id: mediaWidget
                    Layout.fillWidth: true
                    previousRow: null
                    nextRow: null
                    onEscaped: root._returnToTabs()
                }
                Text {
                    Layout.fillWidth: true
                    visible: !mediaWidget.hasPlayer
                    text: "Nothing playing"
                    horizontalAlignment: Text.AlignHCenter
                    font.pixelSize: Theme.fontBody
                    color: Theme.textMuted
                    topPadding: Units.gridUnit * 2
                    bottomPadding: Units.gridUnit * 2
                }
            }

            // --- Quick Settings ---
            ColumnLayout {
                Layout.fillWidth: true
                visible: root._tab === 1
                spacing: Units.spacingMD

                // Audio Output hero card (collapsed current / expand-in-place).
                Text {
                    text: "Audio Output"
                    font.pixelSize: Theme.fontHint
                    font.bold: true
                    color: Theme.textSecondary
                }
                Rectangle {
                    id: outputHero
                    Layout.fillWidth: true
                    Layout.preferredHeight: heroCol.implicitHeight + Units.spacingMD * 2
                    radius: Units.radiusLG
                    readonly property bool rowFocused: root._focusRow === 1 && root._csRow === 1 && !root._outputExpanded
                    color: rowFocused ? Theme.surfaceHover : Theme.cardBackground
                    border.width: (rowFocused || root._outputExpanded) ? Units.borderMedium : Units.borderThin
                    border.color: (rowFocused || root._outputExpanded) ? Theme.focusBorder : Theme.surfaceBorder

                    Behavior on color {
                        ColorAnimation {
                            duration: 100
                        }
                    }

                    ColumnLayout {
                        id: heroCol
                        anchors.left: parent.left
                        anchors.right: parent.right
                        anchors.verticalCenter: parent.verticalCenter
                        anchors.margins: Units.spacingMD
                        spacing: Units.spacingXS

                        // Collapsed: current sink + "A: switch".
                        RowLayout {
                            visible: !root._outputExpanded
                            Layout.fillWidth: true
                            spacing: Units.spacingSM
                            Text {
                                text: "\u{1F50A}"
                                font.pixelSize: Theme.fontBody
                            }
                            Text {
                                Layout.fillWidth: true
                                text: audioCtl.currentSinkName()
                                font.pixelSize: Theme.fontBody
                                color: Theme.textPrimary
                                elide: Text.ElideRight
                            }
                            Text {
                                visible: outputHero.rowFocused
                                text: "A: switch  ▾"
                                font.pixelSize: Theme.fontHint
                                color: Theme.textMuted
                            }
                        }

                        // Expanded: full sink list.
                        Repeater {
                            model: root._outputExpanded ? audioCtl.sinks : []
                            Rectangle {
                                required property int index
                                required property var modelData
                                Layout.fillWidth: true
                                height: Units.gridUnit * 1.6
                                radius: Units.radiusMD
                                color: {
                                    if (modelData.isDefault)
                                        return Theme.sidebarActive;
                                    if (root._sinkCursor === index)
                                        return Theme.surfaceHover;
                                    return Theme.cardBackground;
                                }
                                border.width: (modelData.isDefault || root._sinkCursor === index) ? Units.borderMedium : Units.borderThin
                                border.color: modelData.isDefault ? Theme.focusBorder : Theme.surfaceBorder

                                RowLayout {
                                    anchors.fill: parent
                                    anchors.leftMargin: Units.spacingLG
                                    anchors.rightMargin: Units.spacingLG
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
                                    cursorShape: Qt.PointingHandCursor
                                    onEntered: root._sinkCursor = index
                                    hoverEnabled: true
                                    onClicked: {
                                        audioCtl.setDefaultSinkById(modelData.id);
                                        root._outputExpanded = false;
                                    }
                                }
                            }
                        }
                    }

                    MouseArea {
                        anchors.fill: parent
                        visible: !root._outputExpanded
                        cursorShape: Qt.PointingHandCursor
                        onClicked: {
                            if (audioCtl.sinks.length > 0) {
                                root._focusRow = 1;
                                root._csRow = 1;
                                root._sinkCursor = audioCtl.defaultSinkIndex >= 0 ? audioCtl.defaultSinkIndex : 0;
                                root._outputExpanded = true;
                            }
                        }
                    }
                }

                // Volume bar.
                Text {
                    text: "Volume"
                    font.pixelSize: Theme.fontHint
                    font.bold: true
                    color: Theme.textSecondary
                }
                Rectangle {
                    Layout.fillWidth: true
                    height: Units.gridUnit * 1.6
                    radius: height / 2
                    color: Theme.surfaceHover
                    readonly property bool rowFocused: root._focusRow === 1 && root._csRow === 0 && !root._outputExpanded
                    border.width: rowFocused ? Units.borderMedium : 0
                    border.color: Theme.focusBorder

                    Rectangle {
                        width: parent.width * (audioCtl.volume / 100)
                        height: parent.height
                        radius: parent.radius
                        color: audioCtl.muted ? Theme.textSecondary : (Theme.darkMode ? Theme.ember : Theme.navy)
                        Behavior on width {
                            NumberAnimation {
                                duration: 80
                            }
                        }
                    }
                    Text {
                        anchors.centerIn: parent
                        text: audioCtl.muted ? "MUTED" : audioCtl.volume + "%"
                        font.pixelSize: Theme.fontHint
                        font.bold: true
                        color: audioCtl.volume > 40 && !audioCtl.muted ? Theme.textOnDark : Theme.textPrimary
                    }
                }
            }

            // --- Notifications ---
            ColumnLayout {
                Layout.fillWidth: true
                visible: root._tab === 2
                spacing: Units.spacingMD

                Text {
                    Layout.alignment: Qt.AlignHCenter
                    text: NotificationManager.unreadCount > 0 ? NotificationManager.unreadCount + " unread" : "No new notifications"
                    font.pixelSize: Theme.fontTitle
                    font.bold: true
                    color: NotificationManager.unreadCount > 0 ? Theme.textPrimary : Theme.textMuted
                    topPadding: Units.gridUnit
                }
                Rectangle {
                    Layout.fillWidth: true
                    Layout.preferredHeight: Units.gridUnit * 2.2
                    radius: Units.radiusMD
                    readonly property bool rowFocused: root._focusRow === 1 && root._tab === 2
                    color: rowFocused ? Theme.surfaceHover : Theme.cardBackground
                    border.width: rowFocused ? Units.borderMedium : Units.borderThin
                    border.color: rowFocused ? Theme.focusBorder : Theme.surfaceBorder
                    Behavior on color {
                        ColorAnimation {
                            duration: 100
                        }
                    }
                    RowLayout {
                        anchors.centerIn: parent
                        spacing: Units.spacingSM
                        Text {
                            text: "Open Notification Center"
                            font.pixelSize: Theme.fontBody
                            color: Theme.textPrimary
                        }
                        Text {
                            visible: parent.parent.rowFocused
                            text: "A: open"
                            font.pixelSize: Theme.fontHint
                            color: Theme.textMuted
                        }
                    }
                    MouseArea {
                        anchors.fill: parent
                        cursorShape: Qt.PointingHandCursor
                        onClicked: {
                            root.notificationCenterRequested();
                            root.closed();
                        }
                    }
                }
            }

            // Spacer pushes the footer to the bottom.
            Item {
                Layout.fillWidth: true
                Layout.fillHeight: true
            }

            // === Footer status glyph line ===
            RowLayout {
                Layout.fillWidth: true
                spacing: Units.spacingMD

                RowLayout {
                    spacing: Units.spacingXS
                    Rectangle {
                        Layout.preferredWidth: Units.spacingMD
                        Layout.preferredHeight: Units.spacingMD
                        radius: width / 2
                        color: NetworkManager.connected ? Theme.online : Theme.offline
                    }
                    Text {
                        text: NetworkManager.connected ? "online" : "offline"
                        font.pixelSize: Theme.fontHint
                        color: Theme.textMuted
                    }
                }
                Item {
                    Layout.fillWidth: true
                }
                Text {
                    text: "B: back"
                    font.pixelSize: Theme.fontHint
                    color: Theme.textMuted
                }
            }
        }
    }
}
