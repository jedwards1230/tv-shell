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
            if (statusIcons.activeFocus || mergedRow.activeFocus || moonlightRow.activeFocus || appsRow.activeFocus || popoverMenu.activeFocus)
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
        var row = mergedRow;
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
    // cannot steal focus back after this function sets it.
    function focusDefaultPosition() {
        Qt.callLater(function () {
            var firstRow = null;
            // Priority order: mergedRow (recents+running) > app-view rows
            // (apps mode) > moonlightRow (servers mode) > appsRow.
            if (mergedRow.visible) {
                firstRow = mergedRow;
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
            // is visible again. The Flickable's Behavior on contentY animates this smoothly.
            scrollView.contentY = 0;
            if (Window.activeFocusItem === firstRow && firstRow.currentIndex === 0)
                return;
            firstRow.currentIndex = 0;
            firstRow.forceActiveFocus();
        });
    }

    // === Merged row model (running-pinned recents) ===
    //
    // Produces a single sorted list:
    //   1. Running apps, sorted most-recently-focused first, then by launch recency.
    //      Externally-started windows (not in recents) are included here too.
    //   2. Non-running recents, in their existing recency order.
    //
    // Deduplication: an app that is both running and recent appears once only
    // (as the running card at the front). Identity is keyed on windowClass for
    // running windows matched to recents via exec-basename / name heuristic.
    //
    // Reactivity: this binding re-evaluates whenever either root.runningWindows
    // or RecentsTracker.recentApps changes, so close→reorder is live.
    readonly property var _mergedModel: {
        let running = root.runningWindows || [];
        let recents = RecentsTracker.recentApps || [];

        // Build a set of exec basenames / names for running windows so we can
        // match them against recent app entries (which carry exec/name, not
        // windowClass). This is intentionally lenient — the same heuristic the
        // window poller uses for icon/name resolution.
        function execBasename(exec) {
            if (!exec)
                return "";
            let cmd = exec.split(/\s/)[0];
            return cmd.split("/").pop().toLowerCase();
        }

        function normalize(s) {
            return (s || "").toLowerCase().replace(/[-_.]/g, "");
        }

        function runningMatchesRecent(win, recent) {
            let cls = (win.windowClass || "").toLowerCase();
            let execBase = execBasename(recent.exec || "");
            let appName = (recent.name || "").toLowerCase();

            // Check window title/name match
            let winName = (win.name || "").toLowerCase();
            if (winName !== "" && winName === appName)
                return true;

            // Check exec basename match
            if (execBase !== "") {
                if (cls === execBase || normalize(cls) === normalize(execBase))
                    return true;
                if (cls !== "" && (execBase.indexOf(cls) >= 0 || cls.indexOf(execBase) >= 0))
                    return true;
            }

            // Check app name against class
            if (appName !== "" && (cls === appName || normalize(cls) === normalize(appName)))
                return true;

            return false;
        }

        // Build enriched running entries, pairing each window with its recent entry if any
        let runningEntries = [];
        let matchedRecentIndices = new Set();

        for (let r = 0; r < running.length; r++) {
            let win = running[r];
            let recentMatch = null;
            let recentIdx = recents.length;
            for (let j = 0; j < recents.length; j++) {
                if (runningMatchesRecent(win, recents[j])) {
                    recentMatch = recents[j];
                    recentIdx = j;
                    matchedRecentIndices.add(j);
                    break;
                }
            }
            runningEntries.push({
                // Merge window info over recent info: prefer window's live title/name,
                // but use recent's icon/exec when available for better display.
                windowClass: win.windowClass,
                name: (recentMatch && recentMatch.name) ? recentMatch.name : win.name,
                icon: (recentMatch && recentMatch.icon) ? recentMatch.icon : win.icon,
                exec: (recentMatch && recentMatch.exec) ? recentMatch.exec : win.exec,
                comment: (recentMatch && recentMatch.comment) ? recentMatch.comment : "",
                running: true,
                recentRank: recentIdx
            });
        }

        // Sort running entries: paired-with-recent apps first (by recency rank),
        // then externally-started windows (never in recents) after.
        runningEntries.sort(function (a, b) {
            return a.recentRank - b.recentRank;
        });

        // Collect non-running recents (those not matched to any running window)
        let result = runningEntries.slice();
        for (let k = 0; k < recents.length; k++) {
            if (matchedRecentIndices.has(k))
                continue;
            let rec = recents[k];
            result.push({
                windowClass: "",
                name: rec.name || "",
                icon: rec.icon || "",
                exec: rec.exec || "",
                comment: rec.comment || "",
                running: false,
                recentRank: k
            });
        }

        return result;
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

            // === Merged Recents + Running Row ===
            // Running apps are pinned to the front with an ember dot indicator.
            // Non-running recents follow in recency order. No separate Running row.
            Text {
                visible: root._mergedModel.length > 0
                text: "Recent"
                font.pixelSize: Theme.fontTitle
                font.bold: true
                color: Theme.textPrimary
            }

            NavigableRow {
                id: mergedRow
                visible: root._mergedModel.length > 0
                Layout.fillWidth: true
                Layout.preferredHeight: visible ? Theme.rowHeight : 0
                keyNavigationWraps: true
                focus: visible
                previousRow: statusIcons
                nextRow: {
                    var _ = appViewRepeater.count;
                    if (!root._streamingActive)
                        return appsRow;
                    if (Theme.streamingViewMode === "servers")
                        return moonlightRow;
                    return root._appViewRowItem(0) || appsRow;
                }
                model: root._mergedModel
                onActiveFocusChanged: if (activeFocus)
                    scrollView.ensureVisible(this)

                delegate: AppCard {
                    required property int index
                    required property var modelData
                    height: Theme.cardHeight
                    width: Theme.cardWidth
                    app: modelData
                    running: modelData.running === true
                    focus: index === mergedRow.currentIndex
                    onActivated: {
                        if (modelData.running === true) {
                            root.appFocusRequested(modelData.windowClass);
                        } else {
                            root.launchApp(modelData);
                        }
                    }
                }

                onContextRequested: {
                    if (currentItem && currentIndex >= 0 && currentIndex < root._mergedModel.length) {
                        let entry = root._mergedModel[currentIndex];
                        let pos = currentItem.mapToItem(root, currentItem.width / 2, 0);
                        popoverMenu.targetX = pos.x;
                        popoverMenu.targetY = pos.y;
                        if (entry.running === true) {
                            let wc = entry.windowClass;
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
                        } else {
                            let app = entry;
                            popoverMenu.actions = [
                                {
                                    label: "Launch",
                                    action: function () {
                                        root.launchApp(app);
                                    }
                                }
                            ];
                        }
                        popoverMenu.opened = true;
                        popoverMenu.forceActiveFocus();
                    }
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
                focus: root._streamingActive && Theme.streamingViewMode === "servers" && !mergedRow.visible
                previousRow: mergedRow
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
                            focus: Theme.streamingViewMode === "apps" && appViewRowDelegate.index === 0 && !mergedRow.visible
                            onActiveFocusChanged: if (activeFocus)
                                scrollView.ensureVisible(appViewRowDelegate)
                            model: hostAppList
                            previousRow: {
                                var _ = appViewRepeater.count;
                                return appViewRowDelegate.index === 0 ? mergedRow : root._appViewRowItem(appViewRowDelegate.index - 1);
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
                        return mergedRow;
                    if (Theme.streamingViewMode === "servers")
                        return moonlightRow;
                    return root._appViewRowItem(appViewRepeater.count - 1) || mergedRow;
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
                text: {
                    if (mergedRow.activeFocus) {
                        let idx = mergedRow.currentIndex;
                        let model = root._mergedModel;
                        if (idx >= 0 && idx < model.length && model[idx].running === true)
                            return "A: Resume  |  Y: Actions  |  B: Home  |  ←→: Scroll  |  ↑↓: Switch Row";
                    }
                    if (moonlightRow.activeFocus)
                        return "A: Stream  |  Y: Actions  |  B: Home  |  ←→: Scroll  |  ↑↓: Switch Row";
                    return "A: Launch  |  B: Home  |  ←→: Scroll  |  ↑↓: Switch Row";
                }
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
            mergedRow.forceActiveFocus();
        }
    }
}
