import QtQuick
import QtQuick.Layouts

// Secondary "Library" surface for the redesigned home screen (#249). The home
// body now leads with glance/resume content (Continue + New rails); the full
// browse catalog — every Moonlight server (servers view) or per-host app row
// (apps view) and the complete local Applications list — lives here, one step
// away behind the home "All Apps" entry. Modeled on SettingsPanel's lifecycle:
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

    // Ordered focusable regions, top→bottom (Moonlight rows then Applications).
    // moonlightRow and the per-host app-view rows are mutually exclusive
    // (servers vs apps mode); each reports itself non-focusable in the other
    // mode, so listing both is safe.
    function _contentRegions() {
        let regions = [moonlightRow];
        for (let i = 0; i < appViewRepeater.count; i++) {
            let item = appViewRepeater.itemAt(i);
            if (item && item.navigableRow)
                regions.push(item.navigableRow);
        }
        regions.push(appsRow);
        return regions;
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

    function _appViewRowItem(idx) {
        if (idx < 0 || idx >= appViewRepeater.count)
            return null;
        var item = appViewRepeater.itemAt(idx);
        return item ? item.navigableRow : null;
    }

    // Discover apps while the apps view is active (same cadence as HomeScreen
    // used to run before this content moved here).
    Timer {
        id: appDiscoveryTimer
        interval: 60000
        running: root.visible && Theme.streamingViewMode === "apps"
        repeat: true
        onTriggered: StreamProviders.active.discoverApps()
    }

    onVisibleChanged: {
        if (visible && Theme.streamingViewMode === "apps" && root.targets.length > 0)
            StreamProviders.active.discoverApps();
    }
    onTargetsChanged: {
        if (root.visible && Theme.streamingViewMode === "apps" && root.targets.length > 0)
            StreamProviders.active.discoverApps();
    }

    Connections {
        target: Theme
        function onStreamingViewModeChanged() {
            if (root.visible && Theme.streamingViewMode === "apps" && root.targets.length > 0)
                StreamProviders.active.discoverApps();
        }
    }

    property var _appViewRows: {
        let ha = StreamProviders.active.hostApps;
        let tgts = root.targets;
        let rows = [];
        for (let i = 0; i < tgts.length; i++) {
            let t = tgts[i];
            let apps = ha[t.host] || [];
            rows.push({
                host: t.host,
                name: t.name,
                apps: apps,
                target: t
            });
        }
        return rows;
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

            // === Moonlight Section (server-view or app-view) ===
            Text {
                visible: root._streamingActive && Theme.streamingViewMode === "servers"
                text: "Moonlight"
                font.pixelSize: Theme.fontTitle
                font.bold: true
                color: Theme.textPrimary
            }

            NavigableRow {
                id: moonlightRow
                visible: root._streamingActive && Theme.streamingViewMode === "servers"
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

            // App view: one row per host, each card is an available app
            Repeater {
                id: appViewRepeater
                model: root._streamingActive && Theme.streamingViewMode === "apps" ? root._appViewRows : []

                delegate: ColumnLayout {
                    id: appViewRowDelegate
                    required property var modelData
                    required property int index

                    Layout.fillWidth: true
                    spacing: contentColumn.spacing

                    property var hostData: modelData
                    property var hostTarget: modelData.target
                    property var hostAppList: modelData.apps
                    property alias navigableRow: appViewNavRow

                    RowLayout {
                        spacing: 12

                        Text {
                            text: "Moonlight — " + hostData.name
                            font.pixelSize: Theme.fontTitle
                            font.bold: true
                            color: Theme.textPrimary
                        }

                        Row {
                            spacing: Units.spacingSM
                            Layout.alignment: Qt.AlignVCenter

                            property bool online: hostAppList.length > 0

                            Rectangle {
                                width: 14
                                height: 14
                                radius: 7
                                color: parent.online ? Theme.online : "transparent"
                                border.width: parent.online ? 0 : 2
                                border.color: Theme.offline
                                anchors.verticalCenter: parent.verticalCenter
                            }

                            Text {
                                text: parent.online ? "Online" : "Offline"
                                font.pixelSize: Theme.fontSmall
                                color: Theme.textMuted
                                anchors.verticalCenter: parent.verticalCenter
                            }
                        }
                    }

                    Item {
                        Layout.fillWidth: true
                        Layout.preferredHeight: Theme.rowHeight

                        Text {
                            visible: hostAppList.length === 0 && !StreamProviders.active.discovering
                            anchors.centerIn: parent
                            text: "Offline or no apps found"
                            font.pixelSize: Theme.fontSmall
                            color: Theme.textMuted
                        }

                        Text {
                            visible: hostAppList.length === 0 && StreamProviders.active.discovering
                            anchors.centerIn: parent
                            text: "Discovering apps..."
                            font.pixelSize: Theme.fontSmall
                            color: Theme.textMuted
                        }

                        NavigableRow {
                            id: appViewNavRow
                            anchors.fill: parent
                            visible: hostAppList.length > 0
                            keyNavigationWraps: true
                            onActiveFocusChanged: if (activeFocus)
                                scrollView.ensureVisible(appViewRowDelegate)
                            model: hostAppList
                            previousRow: {
                                var _ = appViewRepeater.count;
                                return appViewRowDelegate.index === 0 ? moonlightRow : root._appViewRowItem(appViewRowDelegate.index - 1);
                            }
                            nextRow: appViewRowDelegate.index < appViewRepeater.count - 1 ? root._appViewRowItem(appViewRowDelegate.index + 1) : appsRow

                            delegate: StreamCard {
                                required property int index
                                required property var modelData
                                height: Theme.cardHeight
                                width: Theme.cardWidth
                                target: hostTarget
                                appName: modelData
                                shellState: root.shellState
                                focus: index === appViewNavRow.currentIndex
                                onActivated: {
                                    let t = JSON.parse(JSON.stringify(hostTarget));
                                    t.app = modelData;
                                    root.streamRequested(t);
                                }
                            }

                            onContextRequested: {
                                if (currentItem && currentIndex >= 0 && currentIndex < hostAppList.length) {
                                    let card = currentItem;
                                    if (card.hasActiveSession) {
                                        let pos = card.mapToItem(root, card.width / 2, 0);
                                        popoverMenu.targetX = pos.x;
                                        popoverMenu.targetY = pos.y;
                                        let t = JSON.parse(JSON.stringify(hostTarget));
                                        t.app = hostAppList[currentIndex];
                                        popoverMenu.actions = [
                                            {
                                                label: "Resume",
                                                action: function () {
                                                    root.streamRequested(t);
                                                }
                                            },
                                            {
                                                label: "Quit Stream",
                                                action: function () {
                                                    root.streamQuitRequested(t);
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
                    }
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
                previousRow: {
                    if (!root._streamingActive)
                        return null;
                    if (Theme.streamingViewMode === "servers")
                        return moonlightRow;
                    return root._appViewRowItem(appViewRepeater.count - 1) || moonlightRow;
                }
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
