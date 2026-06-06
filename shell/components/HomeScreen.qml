import QtQuick
import QtQuick.Layouts

FocusScope {
    id: root

    property var targets: []
    property string shellState: "idle"

    // False when the no-streaming provider is active — collapses all streaming
    // rows and removes them from the focus chain (pure app-launcher mode).
    readonly property bool _streamingActive: StreamProviders.active.providerId !== "none"

    property var runningWindows: []
    property var pads: []

    // Lowest-charge pad that actually reports a battery (batteryLevel >= 0).
    // null when no wireless pad is reporting — the glyph hides entirely.
    readonly property var _batteryPad: {
        var best = null;
        for (var i = 0; i < root.pads.length; i++) {
            var p = root.pads[i];
            if (p.batteryLevel < 0)
                continue; // wired / no battery reported
            if (best === null || p.batteryLevel < best.batteryLevel)
                best = p;
        }
        return best;
    }

    signal streamRequested(var target)
    signal streamQuitRequested(var target)
    signal appLaunchRequested(var app)
    signal appFocusRequested(string windowClass)
    signal appCloseRequested(string windowClass)
    signal settingsRequested
    signal notificationCenterRequested
    signal powerRequested
    signal networkRequested(var anchorRect)
    signal volumeRequested(var anchorRect)
    // Emitted on user-initiated navigation (B-press / Escaped) so the shell
    // root can reset the auto-suspend idle timer. Keeps HomeScreen decoupled
    // from shell.qml's timer implementation.
    signal userActivity

    onActiveFocusChanged: {
        if (activeFocus)
            _ensureRowFocusTimer.restart();
    }

    Timer {
        id: _ensureRowFocusTimer
        interval: 10
        onTriggered: {
            if (!root.activeFocus)
                return;
            if (statusIcons.activeFocus || runningRow.activeFocus || recentsRow.activeFocus || moonlightRow.activeFocus || appsRow.activeFocus || popoverMenu.activeFocus)
                return;
            for (let i = 0; i < appViewRepeater.count; i++) {
                let item = appViewRepeater.itemAt(i);
                if (item && item.navigableRow && item.navigableRow.activeFocus)
                    return;
            }
            root._focusFirstVisibleRow();
        }
    }

    function launchApp(app) {
        root.appLaunchRequested(app);
        RecentsTracker.recordLaunch(app);
    }

    // Moonlight app discovery lives in MoonlightProvider now; HomeScreen only
    // decides WHEN to (re)discover based on the active view mode.
    Timer {
        id: appDiscoveryTimer
        interval: 60000
        running: Theme.streamingViewMode === "apps"
        repeat: true
        onTriggered: StreamProviders.active.discoverApps()
    }

    onTargetsChanged: {
        if (Theme.streamingViewMode === "apps" && root.targets.length > 0)
            StreamProviders.active.discoverApps();
    }

    Connections {
        target: Theme
        function onStreamingViewModeChanged() {
            if (Theme.streamingViewMode === "apps" && root.targets.length > 0)
                StreamProviders.active.discoverApps();
        }
    }

    function _appViewRowItem(idx) {
        if (idx < 0 || idx >= appViewRepeater.count)
            return null;
        var item = appViewRepeater.itemAt(idx);
        return item ? item.navigableRow : null;
    }

    function _focusFirstVisibleRow() {
        var row = runningRow;
        while (row) {
            if (row.visible) {
                row.forceActiveFocus();
                return;
            }
            row = (row.nextRow !== undefined) ? row.nextRow : null;
        }
    }

    // === Default focus target (B-button on home screen) ===
    // The canonical landing position: top content row, first card.
    // Exposed so shell.qml / screensaver hook can attach later (issue #156).
    // Qt.callLater defers the focus assignment one event-loop tick so that
    // declarative focus: bindings that fire synchronously during onEscaped
    // cannot steal focus back after this function sets it (fix for the B-button
    // regression where focus landed on moonlightRow instead of recentsRow).
    function focusDefaultPosition() {
        Qt.callLater(function () {
            var firstRow = null;
            // Priority order: runningRow > recentsRow > app-view rows (apps mode)
            // > moonlightRow (servers mode) > appsRow.
            // runningRow and recentsRow always take precedence over streaming rows
            // so that B from any deeper row returns to the highest visible content
            // row — matching AC for issue #156.
            if (runningRow.visible) {
                firstRow = runningRow;
            } else if (recentsRow.visible) {
                firstRow = recentsRow;
            } else if (root._streamingActive && Theme.streamingViewMode === "apps") {
                // No recents/running — prefer the first visible app-view row.
                for (var i = 0; i < appViewRepeater.count; i++) {
                    var item = appViewRepeater.itemAt(i);
                    if (item && item.navigableRow && item.navigableRow.visible) {
                        firstRow = item.navigableRow;
                        break;
                    }
                }
            } else if (root._streamingActive && Theme.streamingViewMode === "servers" && moonlightRow.visible) {
                firstRow = moonlightRow;
            }
            if (!firstRow)
                firstRow = appsRow;
            // Snap the home view back to the top so the hero clock/date header
            // is visible again — the canonical home position is focus AND scroll
            // reset. Done before the short-circuit below so that pressing B while
            // already focused on the first row (but scrolled down) still scrolls
            // up. The Flickable's Behavior on contentY animates this smoothly.
            scrollView.contentY = 0;
            // Already at the default position — short-circuit only when the
            // window's active-focus item IS the target row AND its currentIndex
            // is already 0.  Reading activeFocusItem (not firstRow.activeFocus)
            // is correct here because we are inside Qt.callLater; at this point
            // any synchronous focus steal has already settled.
            if (Window.activeFocusItem === firstRow && firstRow.currentIndex === 0)
                return;
            firstRow.currentIndex = 0;
            firstRow.forceActiveFocus();
        });
    }

    // Computed model for app-view rows, re-evaluated when targets or hostApps change
    property var _appViewRows: {
        // Explicitly reference both properties so QML re-evaluates this binding
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

    Flickable {
        id: scrollView
        anchors.fill: parent
        anchors.topMargin: Theme.padding
        anchors.bottomMargin: Theme.padding
        anchors.leftMargin: Theme.padding
        anchors.rightMargin: Theme.padding
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

            // === Hero Clock Area ===
            RowLayout {
                Layout.fillWidth: true
                Layout.preferredHeight: Units.gridUnit * 9
                spacing: 32

                // Clock + date (left side)
                ColumnLayout {
                    spacing: 24
                    Layout.alignment: Qt.AlignVCenter

                    Text {
                        id: heroClockText
                        font.pixelSize: Theme.fontHero
                        font.bold: true
                        color: Theme.textPrimary

                        Timer {
                            interval: 1000
                            running: true
                            repeat: true
                            triggeredOnStart: true
                            onTriggered: {
                                let now = new Date();
                                heroClockText.text = now.toLocaleTimeString(Qt.locale(), "h:mm AP");
                            }
                        }
                    }

                    Text {
                        id: heroDateText
                        font.pixelSize: Theme.fontTitle
                        color: Theme.textSecondary

                        Timer {
                            interval: 60000
                            running: true
                            repeat: true
                            triggeredOnStart: true
                            onTriggered: {
                                let now = new Date();
                                heroDateText.text = now.toLocaleDateString(Qt.locale(), "dddd, MMMM d");
                            }
                        }
                    }
                }

                Item {
                    Layout.fillWidth: true
                }

                // Controller battery glance (#100). Non-interactive status indicator —
                // NOT part of the QuickActions navigable carousel. Shows only when a
                // wireless pad reports charge; mirrors ControllerSettings glyph/colors.
                RowLayout {
                    id: batteryIndicator
                    Layout.alignment: Qt.AlignTop
                    Layout.preferredWidth: visible ? implicitWidth : 0
                    spacing: Units.spacingSM
                    visible: root._batteryPad !== null

                    Text {
                        text: "⚡" // charging bolt
                        font.pixelSize: Theme.fontTitle
                        color: Theme.warning
                        visible: root._batteryPad !== null && root._batteryPad.batteryCharging === true
                    }
                    Text {
                        text: "\u{1F50B}" // battery glyph
                        font.pixelSize: Theme.fontTitle
                        color: Theme.textSecondary
                    }
                    Text {
                        text: root._batteryPad !== null ? root._batteryPad.batteryLevel + "%" : ""
                        font.pixelSize: Theme.fontTitle
                        font.bold: true
                        color: (root._batteryPad !== null && root._batteryPad.batteryLevel <= 15) ? Theme.offline : Theme.textSecondary
                    }
                }

                // Status icons (right side)
                QuickActions {
                    id: statusIcons
                    Layout.alignment: Qt.AlignTop | Qt.AlignRight
                    // B/Escape from the status-icon row must NOT open Settings
                    // (issue #156 AC1). escapeRequestsSettings: false prevents
                    // the QuickActions Escape handler from emitting settingsRequested;
                    // we handle navigation back to home focus ourselves below.
                    escapeRequestsSettings: false
                    onSettingsRequested: root.settingsRequested()
                    onNotificationCenterRequested: root.notificationCenterRequested()
                    onPowerRequested: root.powerRequested()
                    onNetworkRequested: anchorRect => root.networkRequested(anchorRect)
                    onVolumeRequested: anchorRect => root.volumeRequested(anchorRect)
                    onFocusDownRequested: root._focusFirstVisibleRow()
                    onActiveFocusChanged: if (activeFocus)
                        scrollView.contentY = 0
                    Keys.onEscapePressed: {
                        root.userActivity();
                        root.focusDefaultPosition();
                    }
                }
            }

            // === Running Windows Row ===
            Text {
                visible: root.runningWindows.length > 0
                text: "Running"
                font.pixelSize: Theme.fontTitle
                font.bold: true
                color: Theme.textPrimary
            }

            NavigableRow {
                id: runningRow
                visible: root.runningWindows.length > 0
                Layout.fillWidth: true
                Layout.preferredHeight: visible ? Theme.rowHeight : 0
                keyNavigationWraps: true
                focus: visible
                previousRow: statusIcons
                nextRow: recentsRow
                model: root.runningWindows
                onActiveFocusChanged: if (activeFocus)
                    scrollView.ensureVisible(this)

                delegate: AppCard {
                    required property int index
                    required property var modelData
                    height: Theme.cardHeight
                    width: Theme.cardWidth
                    app: modelData
                    focus: index === runningRow.currentIndex
                    onActivated: root.appFocusRequested(modelData.windowClass)
                }

                onContextRequested: {
                    if (currentItem && currentIndex >= 0 && currentIndex < root.runningWindows.length) {
                        let pos = currentItem.mapToItem(root, currentItem.width / 2, 0);
                        popoverMenu.targetX = pos.x;
                        popoverMenu.targetY = pos.y;
                        let wc = root.runningWindows[currentIndex].windowClass;
                        popoverMenu.actions = [
                            {
                                label: "Resume",
                                action: function () {
                                    root.appFocusRequested(wc);
                                }
                            },
                            {
                                label: "Quit App",
                                action: function () {
                                    root.appCloseRequested(wc);
                                }
                            }
                        ];
                        popoverMenu.opened = true;
                        popoverMenu.forceActiveFocus();
                    }
                }
                onEscaped: {
                    root.userActivity();
                    root.focusDefaultPosition();
                }
            }

            // === Recents Row ===
            Text {
                visible: RecentsTracker.recentApps.length > 0
                text: "Recent"
                font.pixelSize: Theme.fontTitle
                font.bold: true
                color: Theme.textPrimary
            }

            NavigableRow {
                id: recentsRow
                visible: RecentsTracker.recentApps.length > 0
                Layout.fillWidth: true
                Layout.preferredHeight: visible ? Theme.rowHeight : 0
                keyNavigationWraps: true
                focus: visible && !runningRow.visible
                previousRow: runningRow
                onActiveFocusChanged: if (activeFocus)
                    scrollView.ensureVisible(this)
                nextRow: {
                    var _ = appViewRepeater.count;
                    if (!root._streamingActive)
                        return appsRow;
                    if (Theme.streamingViewMode === "servers")
                        return moonlightRow;
                    return root._appViewRowItem(0) || appsRow;
                }
                model: RecentsTracker.recentApps

                delegate: AppCard {
                    required property int index
                    required property var modelData
                    height: Theme.cardHeight
                    width: Theme.cardWidth
                    app: modelData
                    focus: index === recentsRow.currentIndex
                    onActivated: root.launchApp(modelData)
                }

                onEscaped: {
                    root.userActivity();
                    root.focusDefaultPosition();
                }
            }

            // === Moonlight Section (server-view or app-view) ===

            // Server view: single "Moonlight" row with one card per server
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
                focus: root._streamingActive && Theme.streamingViewMode === "servers" && !recentsRow.visible && !runningRow.visible
                previousRow: recentsRow
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
                    root.focusDefaultPosition();
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

                        // a11y: shape (filled dot vs hollow ring) + text label — state
                        // is not color-only (colorblind-safe dual cue).
                        Row {
                            spacing: Units.spacingSM
                            Layout.alignment: Qt.AlignVCenter

                            property bool online: hostAppList.length > 0

                            Rectangle {
                                width: 14
                                height: 14
                                radius: 7
                                // ONLINE: filled dot; OFFLINE: hollow ring — distinct shapes
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

                        // Offline state
                        Text {
                            visible: hostAppList.length === 0 && !StreamProviders.active.discovering
                            anchors.centerIn: parent
                            text: "Offline or no apps found"
                            font.pixelSize: Theme.fontSmall
                            color: Theme.textMuted
                        }

                        // Loading state
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
                            focus: Theme.streamingViewMode === "apps" && appViewRowDelegate.index === 0 && !recentsRow.visible && !runningRow.visible
                            onActiveFocusChanged: if (activeFocus)
                                scrollView.ensureVisible(appViewRowDelegate)
                            model: hostAppList
                            previousRow: {
                                var _ = appViewRepeater.count;
                                return appViewRowDelegate.index === 0 ? recentsRow : root._appViewRowItem(appViewRowDelegate.index - 1);
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
                                root.focusDefaultPosition();
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
                        return recentsRow;
                    if (Theme.streamingViewMode === "servers")
                        return moonlightRow;
                    return root._appViewRowItem(appViewRepeater.count - 1) || recentsRow;
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
                    root.focusDefaultPosition();
                }
            }

            // === Hint Bar ===
            Text {
                text: runningRow.activeFocus ? "A: Resume  |  Y: Actions  |  B: Home  |  ←→: Scroll  |  ↑↓: Switch Row" : (moonlightRow.activeFocus ? "A: Stream  |  Y: Actions  |  B: Home  |  ←→: Scroll  |  ↑↓: Switch Row" : "A: Launch  |  B: Home  |  ←→: Scroll  |  ↑↓: Switch Row")
                font.pixelSize: Theme.fontHint
                color: Theme.textMuted
                Layout.alignment: Qt.AlignHCenter
                Layout.bottomMargin: 16
            }
        }
    }

    PopoverMenu {
        id: popoverMenu
        onClosed: {
            popoverMenu.opened = false;
            runningRow.forceActiveFocus();
        }
    }
}
