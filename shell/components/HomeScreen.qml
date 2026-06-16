import QtQuick
import QtQuick.Layouts

// Home screen — the "glance + jump back in" overview (#249), composed of
// standardized, individually-toggleable, individually-sized home widgets. The
// full browse catalog lives in the secondary LibraryScreen (the "All Apps"
// entry). Vertical layout, top→bottom:
//   hero → Now-Playing widget → Plex widget → Recent (apps) widget → All Apps
//
// Standardized widget model (#249): each widget reads its own `enabled` + `size`
// from SettingsStore (via Theme) and implements the duck-typed home-tile focus
// contract (visible / regionFocused / focusFirstChild + previousRow/nextRow), so
// HomeScreen drives focus from one ordered _contentRegions() list and the
// Widgets settings page configures each uniformly. Now-Playing has two size
// renderers — small = NowPlayingStrip, medium = MediaWidget (card + progress);
// only the size-matching one is visible.
FocusScope {
    id: root

    property var targets: []
    property string shellState: "idle"

    property var runningWindows: []
    property var pads: []

    // The active Now-Playing renderer for the current size (used for the
    // recents dedup + focus wiring). Both instances exist; one is visible.
    readonly property var _npActive: Theme.widgetSpotifySize === "small" ? nowPlayingStrip : nowPlayingCard

    // Recent (apps) size: small = icon-only square tiles (label dropped),
    // medium = full icon + label cards. A reformat, not a scale.
    readonly property bool _recentSmall: Theme.widgetRecentSize === "small"

    readonly property var _batteryPad: {
        var best = null;
        for (var i = 0; i < root.pads.length; i++) {
            var p = root.pads[i];
            if (p.batteryLevel < 0)
                continue;
            if (best === null || p.batteryLevel < best.batteryLevel)
                best = p;
        }
        return best;
    }

    signal appLaunchRequested(var app)
    signal appFocusRequested(string address)
    signal appCloseRequested(string address)
    signal settingsRequested
    signal notificationCenterRequested
    signal powerRequested
    signal networkRequested(var anchorRect)
    signal volumeRequested(var anchorRect)
    signal libraryRequested
    signal userActivity

    // Ordered focusable regions, top→bottom. Both Now-Playing renderers are
    // listed; the hidden one reports focusFirstChild()===false and is skipped.
    function _contentRegions() {
        return [nowPlayingStrip, nowPlayingCard, plexWidget, recentRow, allAppsEntry];
    }

    function _reanchorFocusIfNeeded() {
        if (!root.activeFocus)
            return;
        if (statusIcons.activeFocus || popoverMenu.activeFocus)
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

    onRunningWindowsChanged: Qt.callLater(root._reanchorFocusIfNeeded)

    function launchApp(app) {
        root.appLaunchRequested(app);
        RecentsTracker.recordLaunch(app);
    }

    // === Now-Playing "Open app" support ===
    function _mediaNorm(s) {
        return (s || "").toLowerCase().replace(/[-_.]/g, "");
    }
    function _mediaExecBase(exec) {
        if (!exec)
            return "";
        return exec.split(/\s/)[0].split("/").pop().toLowerCase();
    }
    function _entryIsActivePlayer(entry, desktopEntry, identity) {
        var de = (desktopEntry || "").toLowerCase();
        var id = (identity || "").toLowerCase();
        if (de === "" && id === "")
            return false;
        var cls = (entry.windowClass || "").toLowerCase();
        var execBase = root._mediaExecBase(entry.exec || "");
        var name = (entry.name || "").toLowerCase();
        if (de !== "") {
            if (cls === de || root._mediaNorm(cls) === root._mediaNorm(de))
                return true;
            if (execBase === de || root._mediaNorm(execBase) === root._mediaNorm(de))
                return true;
            if (root._mediaNorm(name) === root._mediaNorm(de))
                return true;
        }
        if (id !== "" && (root._mediaNorm(name) === root._mediaNorm(id) || root._mediaNorm(cls) === root._mediaNorm(id)))
            return true;
        return false;
    }
    function _resolveMediaApp(desktopEntry, identity) {
        var apps = AppDiscoveryManager.applications || [];
        var de = (desktopEntry || "").toLowerCase();
        var id = (identity || "").toLowerCase();
        for (var i = 0; i < apps.length; i++) {
            var a = apps[i];
            if (de !== "") {
                if (root._mediaExecBase(a.exec || "") === de || root._mediaNorm(a.wmClass || "") === root._mediaNorm(de) || root._mediaNorm(a.name || "") === root._mediaNorm(de))
                    return a;
            }
            if (id !== "" && root._mediaNorm(a.name || "") === root._mediaNorm(id))
                return a;
        }
        if (de === "" && id === "")
            return null;
        return {
            "name": identity || desktopEntry,
            "exec": desktopEntry || id,
            "wmClass": identity || desktopEntry,
            "comment": "",
            "icon": ""
        };
    }
    function openMediaApp(desktopEntry, identity) {
        var app = root._resolveMediaApp(desktopEntry, identity);
        if (!app)
            return;
        root.userActivity();
        root.launchApp(app);
    }

    function openPlexApp() {
        var apps = AppDiscoveryManager.applications || [];
        for (var i = 0; i < apps.length; i++) {
            var a = apps[i];
            var hay = ((a.name || "") + " " + (a.wmClass || "") + " " + (a.exec || "")).toLowerCase();
            if (hay.indexOf("plex") !== -1) {
                root.userActivity();
                root.launchApp(a);
                return;
            }
        }
        console.log("HomeScreen: no Plex app found to launch");
    }

    function _focusFirstVisibleRow() {
        var regions = root._contentRegions();
        for (var i = 0; i < regions.length; i++) {
            if (regions[i] && regions[i].focusFirstChild())
                return;
        }
    }

    function focusDefaultPosition() {
        Qt.callLater(function () {
            scrollView.contentY = 0;
            var regions = root._contentRegions();
            for (var i = 0; i < regions.length; i++) {
                if (regions[i] && regions[i].focusFirstChild())
                    return;
            }
        });
    }

    // === Recent model (running windows + non-running recents) ===
    readonly property var _recentModel: {
        let running = root.runningWindows || [];
        let recents = RecentsTracker.recentApps || [];
        let allApps = AppDiscoveryManager.applications || [];

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
            let winName = (win.name || "").toLowerCase();
            if (winName !== "" && winName === appName)
                return true;
            if (execBase !== "") {
                if (cls === execBase || normalize(cls) === normalize(execBase))
                    return true;
                if (cls !== "" && (execBase.indexOf(cls) >= 0 || cls.indexOf(execBase) >= 0))
                    return true;
            }
            if (appName !== "" && (cls === appName || normalize(cls) === normalize(appName)))
                return true;
            return false;
        }
        function resolveRecentIcon(rec) {
            let rexec = execBasename(rec.exec || "");
            for (let i = 0; i < allApps.length; i++) {
                let a = allApps[i];
                if (a.name && rec.name && a.name === rec.name)
                    return a.icon || "";
                if (rexec !== "" && execBasename(a.exec || "") === rexec)
                    return a.icon || "";
            }
            return rec.icon || "";
        }

        let runningEntries = [];
        let matchedRecentIndices = new Set();
        for (let r = 0; r < running.length; r++) {
            let win = running[r];
            for (let j = 0; j < recents.length; j++) {
                if (runningMatchesRecent(win, recents[j]))
                    matchedRecentIndices.add(j);
            }
            runningEntries.push({
                windowClass: win.windowClass,
                address: win.address || "",
                name: win.title || win.name || win.windowClass,
                icon: win.icon || "",
                exec: "",
                comment: "",
                running: true,
                focusHistoryId: (win.focusHistoryId !== undefined) ? win.focusHistoryId : 9999
            });
        }
        runningEntries.sort(function (a, b) {
            return a.focusHistoryId - b.focusHistoryId;
        });

        let result = runningEntries.slice();
        for (let k = 0; k < recents.length; k++) {
            if (matchedRecentIndices.has(k))
                continue;
            let rec = recents[k];
            result.push({
                windowClass: "",
                address: "",
                name: rec.name || "",
                icon: resolveRecentIcon(rec),
                exec: rec.exec || "",
                comment: rec.comment || "",
                running: false,
                focusHistoryId: 9999
            });
        }

        // Hide the app the active Now-Playing widget already represents.
        var np = root._npActive;
        if (np && np.visible && (np.playerDesktopEntry !== "" || np.playerIdentity !== "")) {
            let de = np.playerDesktopEntry;
            let id = np.playerIdentity;
            result = result.filter(function (e) {
                return !root._entryIsActivePlayer(e, de, id);
            });
        }
        return result;
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

                RowLayout {
                    id: batteryIndicator
                    Layout.alignment: Qt.AlignTop
                    Layout.preferredWidth: visible ? implicitWidth : 0
                    spacing: Units.spacingSM
                    visible: root._batteryPad !== null

                    Text {
                        text: "⚡"
                        font.pixelSize: Theme.fontTitle
                        color: Theme.warning
                        visible: root._batteryPad !== null && root._batteryPad.batteryCharging === true
                    }
                    Text {
                        text: "\u{1F50B}"
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

                QuickActions {
                    id: statusIcons
                    Layout.alignment: Qt.AlignTop | Qt.AlignRight
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

            // === Now Playing — small (strip) renderer ===
            NowPlayingStrip {
                id: nowPlayingStrip
                Layout.fillWidth: true
                widgetEnabled: Theme.widgetSpotifyEnabled && Theme.widgetSpotifySize === "small"
                previousRow: statusIcons
                nextRow: plexWidget.canFocus ? plexWidget.firstRow : recentRow
                onEscaped: {
                    root.userActivity();
                    root.focusDefaultPosition();
                }
                onOpenAppRequested: (desktopEntry, identity) => root.openMediaApp(desktopEntry, identity)
                onContextRequested: root._mediaContext(nowPlayingStrip)
            }

            // === Now Playing — medium (card + progress) renderer ===
            MediaWidget {
                id: nowPlayingCard
                Layout.fillWidth: true
                widgetEnabled: Theme.widgetSpotifyEnabled && Theme.widgetSpotifySize === "medium"
                previousRow: statusIcons
                nextRow: plexWidget.canFocus ? plexWidget.firstRow : recentRow
                onEscaped: {
                    root.userActivity();
                    root.focusDefaultPosition();
                }
                onOpenAppRequested: (desktopEntry, identity) => root.openMediaApp(desktopEntry, identity)
                onContextRequested: root._mediaContext(nowPlayingCard)
            }

            // === Plex widget (On Deck + Recently Added + dynamic chips) ===
            PlexWidget {
                id: plexWidget
                Layout.fillWidth: true
                widgetEnabled: Theme.widgetPlexEnabled
                size: Theme.widgetPlexSize
                previousRow: root._npActive
                nextRow: recentRow
                onEscaped: {
                    root.userActivity();
                    root.focusDefaultPosition();
                }
                onOpenPlexRequested: root.openPlexApp()
                onEnsureVisibleRequested: item => scrollView.ensureVisible(item)
            }

            // === Recent (apps) widget ===
            Text {
                visible: Theme.widgetRecentEnabled && root._recentModel.length > 0
                text: "Recent"
                font.pixelSize: Theme.fontTitle
                font.bold: true
                color: Theme.textPrimary
            }

            NavigableRow {
                id: recentRow
                visible: Theme.widgetRecentEnabled && root._recentModel.length > 0
                Layout.fillWidth: true
                Layout.preferredHeight: visible ? Theme.rowHeight : 0
                keyNavigationWraps: true
                previousRow: plexWidget.canFocus ? plexWidget.lastRow : root._npActive
                nextRow: allAppsEntry
                model: root._recentModel
                onActiveFocusChanged: if (activeFocus)
                    scrollView.ensureVisible(this)

                delegate: AppCard {
                    required property int index
                    required property var modelData
                    iconOnly: root._recentSmall
                    width: root._recentSmall ? Theme.cardHeight : Theme.cardWidth
                    height: Theme.cardHeight
                    app: modelData
                    running: modelData.running === true
                    focus: index === recentRow.currentIndex
                    onActivated: {
                        if (modelData.running === true)
                            root.appFocusRequested(modelData.address);
                        else
                            root.launchApp(modelData);
                    }
                }

                onContextRequested: {
                    if (currentItem && currentIndex >= 0 && currentIndex < root._recentModel.length) {
                        let entry = root._recentModel[currentIndex];
                        let pos = currentItem.mapToItem(root, currentItem.width / 2, 0);
                        popoverMenu.targetX = pos.x;
                        popoverMenu.targetY = pos.y;
                        if (entry.running === true) {
                            let addr = entry.address;
                            popoverMenu.actions = [
                                {
                                    label: "Resume",
                                    action: function () {
                                        root.appFocusRequested(addr);
                                    }
                                },
                                {
                                    label: "Quit App",
                                    action: function () {
                                        root.appCloseRequested(addr);
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

            // === All Apps entry (→ Library) ===
            NavigableRow {
                id: allAppsEntry
                Layout.fillWidth: true
                Layout.preferredHeight: Theme.cardHeight
                model: 1
                previousRow: recentRow
                onActivated: root.libraryRequested()
                onActiveFocusChanged: if (activeFocus)
                    scrollView.ensureVisible(this)
                onEscaped: {
                    root.userActivity();
                    root.focusDefaultPosition();
                }

                delegate: Item {
                    required property int index
                    width: Math.round(Theme.cardWidth * 1.8)
                    height: Theme.cardHeight
                    readonly property bool isFocused: (index === allAppsEntry.currentIndex && allAppsEntry.activeFocus && !Theme.mouseMode) || (allAppsMouse.containsMouse && Theme.mouseMode)
                    z: isFocused ? 10 : 0

                    FocusFrame {
                        anchors.fill: parent
                        focused: parent.isFocused

                        RowLayout {
                            anchors.centerIn: parent
                            spacing: Units.spacingLG

                            Text {
                                text: "▦"
                                font.pixelSize: Units.iconSizeLG
                                color: Theme.textPrimary
                            }
                            ColumnLayout {
                                spacing: 2
                                Text {
                                    text: "All Apps"
                                    font.pixelSize: Theme.fontTitle
                                    font.bold: true
                                    color: Theme.textPrimary
                                }
                                Text {
                                    text: (AppDiscoveryManager.applications ? AppDiscoveryManager.applications.length : 0) + " apps · Moonlight"
                                    font.pixelSize: Theme.fontCaption
                                    color: Theme.textMuted
                                }
                            }
                        }

                        MouseArea {
                            id: allAppsMouse
                            anchors.fill: parent
                            hoverEnabled: true
                            cursorShape: Qt.PointingHandCursor
                            onPositionChanged: mouse => {
                                let p = mapToItem(null, mouse.x, mouse.y);
                                Theme.pointerMoved(p.x, p.y);
                            }
                            onClicked: {
                                Theme.enterMouseMode();
                                allAppsEntry.currentIndex = 0;
                                allAppsEntry.forceActiveFocus();
                                root.libraryRequested();
                            }
                        }
                    }
                }
            }

            // === Hint Bar ===
            Text {
                text: {
                    if (recentRow.activeFocus) {
                        let idx = recentRow.currentIndex;
                        let model = root._recentModel;
                        let running = (idx >= 0 && idx < model.length && model[idx].running === true);
                        return (running ? "A: Resume" : "A: Launch") + "  |  Y: Actions  |  B: Home  |  ←→: Scroll  |  ↑↓: Switch Row";
                    }
                    if (allAppsEntry.activeFocus)
                        return "A: Browse all  |  B: Home  |  ↑↓: Switch Row";
                    return "A: Select  |  B: Home  |  ←→: Scroll  |  ↑↓: Switch Row";
                }
                font.pixelSize: Theme.fontHint
                color: Theme.textMuted
                Layout.alignment: Qt.AlignHCenter
                Layout.bottomMargin: 16
            }
        }
    }

    // Shared Now-Playing context-menu opener (quit the player).
    function _mediaContext(widget) {
        let p = widget.player;
        if (!p || !p.canQuit)
            return;
        let pos = widget.mapToItem(root, widget.width / 2, widget.height);
        popoverMenu.targetX = pos.x;
        popoverMenu.targetY = pos.y;
        popoverMenu.actions = [
            {
                label: "Quit " + (p.identity || "Player"),
                action: function () {
                    if (p.canQuit)
                        p.quit();
                }
            }
        ];
        popoverMenu.opened = true;
        popoverMenu.forceActiveFocus();
    }

    PopoverMenu {
        id: popoverMenu
        onClosed: {
            popoverMenu.opened = false;
            root._focusFirstVisibleRow();
        }
    }
}
