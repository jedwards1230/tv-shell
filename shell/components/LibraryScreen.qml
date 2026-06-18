import QtQuick
import QtQuick.Layouts

// Secondary "Library" surface for the redesigned home screen (#249). The home
// body now leads with glance/resume content (Continue + New rails); the full
// browse catalog — every Moonlight server and the complete local Applications
// list — lives here, one step
// away behind the home "All Apps" entry. Modeled on SettingsApp's lifecycle:
// toggled by `visible` from ShellLayout, B/Escape emits `closed`, and it owns a
// self-contained focus chain over its rows (same list-driven contract as
// HomeScreen). It re-emits the stream/app intents up to ShellLayout unchanged.
FocusScope {
    id: root

    property var targets: []
    property string shellState: "idle"

    readonly property bool _streamingActive: StreamProviders.active.providerId !== "none"

    signal streamRequested(var target)
    signal streamQuitRequested(var target)
    signal appLaunchRequested(var app)
    signal appFocusRequested(string address)
    signal appCloseRequested(string address)
    // Back-out to the home screen (B/Escape).
    signal closed
    // Any navigation — lets ShellLayout reset the auto-suspend idle timer.
    signal userActivity

    function launchApp(app) {
        root.appLaunchRequested(app);
        RecentsTracker.recordLaunch(app);
    }

    // Ordered focusable regions, top→bottom: the Moonlight servers row, then the
    // local Applications row.
    function _contentRegions() {
        return [moonlightRow, appsRow];
    }

    function _focusFirstVisibleRow() {
        var regions = root._contentRegions();
        for (var i = 0; i < regions.length; i++) {
            if (regions[i] && regions[i].focusFirstChild())
                return;
        }
    }

    // Default landing position when the surface opens: first focusable region,
    // view scrolled to the top.
    function focusDefaultPosition() {
        Qt.callLater(function () {
            scrollView.contentY = 0;
            root._focusFirstVisibleRow();
        });
    }

    function _reanchorFocusIfNeeded() {
        if (!root.activeFocus)
            return;
        if (popoverMenu.activeFocus)
            return;
        let regions = root._contentRegions();
        for (let i = 0; i < regions.length; i++) {
            if (regions[i] && regions[i].regionFocused)
                return;
        }
        Qt.callLater(function () {
            if (root.activeFocus)
                root._focusFirstVisibleRow();
        });
    }

    Timer {
        id: _ensureRowFocusTimer
        interval: 150
        repeat: true
        running: root.activeFocus
        triggeredOnStart: true
        onTriggered: root._reanchorFocusIfNeeded()
    }

    // Opaque scrim so the home screen underneath doesn't bleed through.
    Rectangle {
        anchors.fill: parent
        color: Theme.background
    }

    Flickable {
        id: scrollView
        anchors.fill: parent
        anchors.margins: Theme.padding
        contentHeight: contentColumn.implicitHeight
        clip: true
        boundsBehavior: Flickable.StopAtBounds
        flickableDirection: Flickable.VerticalFlick

        function ensureVisible(item) {
            if (!item)
                return;
            let mapped = item.mapToItem(contentColumn, 0, 0);
            let itemTop = mapped.y;
            let itemBottom = itemTop + item.height;
            if (itemTop < contentY)
                contentY = Math.max(0, itemTop - 24);
            else if (itemBottom > contentY + height)
                contentY = Math.min(contentHeight - height, itemBottom - height + 24);
        }

        Behavior on contentY {
            NumberAnimation {
                duration: 300
                easing.type: Easing.OutCubic
            }
        }

        ColumnLayout {
            id: contentColumn
            width: scrollView.width
            spacing: 24

            // === Header ===
            RowLayout {
                Layout.fillWidth: true
                Layout.preferredHeight: Units.gridUnit * 5
                spacing: 16

                ColumnLayout {
                    Layout.alignment: Qt.AlignVCenter
                    spacing: 6

                    Text {
                        text: "Library"
                        font.pixelSize: Theme.fontHero
                        font.bold: true
                        color: Theme.textPrimary
                    }
                    Text {
                        text: "All apps and streaming targets"
                        font.pixelSize: Theme.fontBody
                        color: Theme.textSecondary
                    }
                }

                Item {
                    Layout.fillWidth: true
                }

                Text {
                    text: "B: Back"
                    font.pixelSize: Theme.fontHint
                    color: Theme.textMuted
                    Layout.alignment: Qt.AlignVCenter
                }
            }

            // === Moonlight Section (one card per server) ===
            Text {
                visible: root._streamingActive
                text: "Moonlight"
                font.pixelSize: Theme.fontTitle
                font.bold: true
                color: Theme.textPrimary
            }

            NavigableRow {
                id: moonlightRow
                visible: root._streamingActive
                Layout.fillWidth: true
                Layout.preferredHeight: visible ? Theme.rowHeight : 0
                keyNavigationWraps: true
                previousRow: null
                nextRow: appsRow
                onActiveFocusChanged: if (activeFocus)
                    scrollView.ensureVisible(this)
                model: root.targets

                delegate: StreamCard {
                    required property int index
                    required property var modelData
                    height: Theme.cardHeight
                    width: Theme.cardWidth
                    target: modelData
                    shellState: root.shellState
                    focus: index === moonlightRow.currentIndex
                    onActivated: root.streamRequested(modelData)
                }

                onContextRequested: {
                    if (currentItem && currentIndex >= 0 && currentIndex < root.targets.length) {
                        let card = currentItem;
                        let target = root.targets[currentIndex];
                        if (card.hasActiveSession) {
                            let pos = card.mapToItem(root, card.width / 2, 0);
                            popoverMenu.targetX = pos.x;
                            popoverMenu.targetY = pos.y;
                            popoverMenu.actions = [
                                {
                                    label: "Resume",
                                    action: function () {
                                        root.streamRequested(target);
                                    }
                                },
                                {
                                    label: "Quit Stream",
                                    action: function () {
                                        root.streamQuitRequested(target);
                                    }
                                }
                            ];
                            popoverMenu.opened = true;
                            popoverMenu.forceActiveFocus();
                        }
                    }
                }
                onEscaped: {
                    root.userActivity();
                    root.closed();
                }
            }

            // === Applications Row ===
            Text {
                text: "Applications"
                font.pixelSize: Theme.fontTitle
                font.bold: true
                color: Theme.textPrimary
            }

            NavigableRow {
                id: appsRow
                Layout.fillWidth: true
                Layout.preferredHeight: Theme.rowHeight
                keyNavigationWraps: true
                onActiveFocusChanged: if (activeFocus)
                    scrollView.ensureVisible(this)
                previousRow: root._streamingActive ? moonlightRow : null
                model: AppDiscoveryManager.applications

                delegate: AppCard {
                    required property int index
                    required property var modelData
                    height: Theme.cardHeight
                    width: Theme.cardWidth
                    app: modelData
                    focus: index === appsRow.currentIndex
                    onActivated: root.launchApp(modelData)
                }

                onEscaped: {
                    root.userActivity();
                    root.closed();
                }
            }
        }
    }

    PopoverMenu {
        id: popoverMenu
        onClosed: {
            popoverMenu.opened = false;
            root._focusFirstVisibleRow();
        }
    }

    // Catch B/Escape when focus rests on the surface itself (no row focused).
    Keys.onEscapePressed: {
        root.userActivity();
        root.closed();
    }
}
