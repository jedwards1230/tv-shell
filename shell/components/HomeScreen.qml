import QtQuick
import QtQuick.Layouts
import "lib"
import "../widgets/lib"

// Home screen — the "glance + jump back in" overview (#249), composed of
// standardized, individually-toggleable, individually-sized home widgets. The
// full browse catalog lives in the secondary LibraryScreen (the "All Apps"
// entry). Vertical layout, top→bottom:
//   hero → Moonlight → Now-Playing → Plex → Recent (apps) → All Apps
//
// Standardized widget model (#249 Phase 2): each widget extends the `Widget` base
// (the duck-typed home-tile focus contract: visible / regionFocused / canFocus /
// firstRow / lastRow / focusFirstChild + previousRow/nextRow). A WidgetHost
// instantiates the set from the WidgetRegistry and builds the vertical focus
// chain generically, so HomeScreen no longer hand-wires per-widget neighbours; it
// only owns the two terminal anchors (the QuickActions row above, the All Apps
// entry below) and attaches each widget's behaviour via `widgetById`. Now-Playing
// is ONE widget whose `size` selects its renderer (small = strip, medium = card).
FocusScope {
    id: root

    property var targets: []
    property string shellState: "idle"

    property var runningWindows: []
    property var pads: []

    // Standardized home widgets are instantiated by WidgetHost from the
    // WidgetRegistry; these reactive aliases reach the live instances by id so the
    // context-menu helpers, recents suppression, and hint bar can read each
    // widget's state. Any may be null until its Loader resolves.
    readonly property Item moonlightWidget: widgetHost.widgetById("moonlight")
    readonly property Item nowPlayingWidget: widgetHost.widgetById("nowplaying")
    readonly property Item plexWidget: widgetHost.widgetById("plex")
    readonly property Item recentWidget: widgetHost.widgetById("recent")

    // Recent (apps) size: small = icon-only square tiles (label dropped),
    // medium = full icon + label cards. A reformat, not a scale.
    readonly property bool _recentSmall: SettingsStore.widget("recent").size === "small"

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

    signal streamRequested(var target)
    signal streamQuitRequested(var target)
    signal appLaunchRequested(var app)
    signal appFocusRequested(string address)
    signal appCloseRequested(string address)
    signal settingsRequested
    signal widgetsRequested
    signal notificationCenterRequested
    signal powerRequested
    signal networkRequested(var anchorRect)
    signal volumeRequested(var anchorRect)
    signal libraryRequested
    signal userActivity

    function _reanchorFocusIfNeeded() {
        if (!root.activeFocus)
            return;
        if (statusIcons.activeFocus || popoverMenu.activeFocus)
            return;
        // Focus is still on the home content if any hosted widget region or the
        // All Apps entry holds it; otherwise re-seat on the first focusable row.
        if (widgetHost.regionFocused || allAppsEntry.regionFocused)
            return;
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

    // Pre-discover Moonlight apps so the home X-menu profile picker is populated
    // on the first press (the provider runs `moonlight list` per host). Re-run
    // ONLY when the host SET changes — not when a target's `app` (default profile)
    // changes via the X-menu "set default". discoverApps() clears hostApps while
    // it re-queries, so re-running it on our own setHostApp write would blank the
    // picker mid-rebuild and collapse it to the empty-state fallback.
    property string _discoveredHosts: ""
    function _maybeDiscoverApps() {
        let ts = StreamProviders.active.targets || [];
        if (ts.length === 0)
            return;
        let hosts = ts.map(t => t.host).sort().join(",");
        if (hosts === root._discoveredHosts)
            return;
        root._discoveredHosts = hosts;
        StreamProviders.active.discoverApps();
    }

    Component.onCompleted: root._maybeDiscoverApps()

    Connections {
        target: StreamProviders.active
        function onTargetsChanged() {
            root._maybeDiscoverApps();
        }
    }

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

    function _entryIsPlex(entry) {
        var hay = ((entry.name || "") + " " + (entry.windowClass || "") + " " + (entry.exec || "")).toLowerCase();
        return hay.indexOf("plex") !== -1;
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

    // === Steam launch choreography ===
    // Select a Steam card →
    //   1. `steam-launch <appid>` → the host NAVIGATES Big Picture to that game's
    //      page (it no longer launches the game directly — just moves BPM).
    //   2. If THIS client is NOT already viewing a stream (`shellState !==
    //      "streaming"`) → start one stream to the primary target (targets[0]).
    //      Moonlight RESUMES a host that already has a session, so this both opens
    //      a fresh stream and reconnects to a resumable one — selecting a game must
    //      ALWAYS get you onto the screen.
    //   3. If this client IS already streaming → the nav alone moved the live BPM;
    //      don't launch a 2nd local Moonlight process.
    // The gate is THIS client's own `shellState`, NOT the host's session flag
    // (`moonlightWidget.streaming`, which is true whenever *any* client — e.g. the
    // laptop — holds a resumable session). Gating on the host flag wrongly blocked
    // launching/resuming whenever a session existed elsewhere; that flag drives the
    // session INDICATOR only. Never trigger a `rungameid`-style direct launch.
    function launchSteamGame(appid) {
        root.userActivity();
        // Fire the host-side navigate (fire-and-forget; the reply is just ok/error).
        steamLaunchReq.request("steam-launch", appid);
        if (root.shellState === "streaming") {
            // This client is already in the stream — the nav moved the live BPM.
            // Don't start a 2nd local Moonlight process.
            return;
        }
        // Not viewing a stream here → start/resume one to the primary target. With
        // no target configured there's nothing to stream into; the nav still fired.
        let ts = root.targets || [];
        if (ts.length > 0)
            root.streamRequested(ts[0]);
        else
            console.log("HomeScreen: steam-launch " + appid + " sent, but no stream target configured");
    }

    // One-shot socket client for `steam-launch <appid>`. The reply (ok/error) is
    // logged on failure; the stream start above doesn't gate on it (the navigate
    // and the stream race, exactly like picking a game in old GameStream).
    SocketClient {
        id: steamLaunchReq
        onResponseReceived: line => {
            if (line !== "ok")
                console.log("HomeScreen: steam-launch reply: " + line);
        }
        onRequestFailed: console.log("HomeScreen: steam-launch request failed (daemon down?)")
    }

    // === Open Steam Big Picture (home) choreography ===
    // The "Open Steam" action chip in the library view →
    //   1. `steam-bigpicture` (no appid) → the host RESETS Big Picture to its HOME
    //      screen (fires `steam://open/bigpicture`; no game pre-selected).
    //   2. If THIS client is NOT already viewing a stream (`shellState !==
    //      "streaming"`) → start/resume one stream to the primary target
    //      (targets[0]) — Moonlight resumes a resumable host, so this both opens a
    //      fresh stream and reconnects to one already live elsewhere.
    //   3. If this client IS already streaming → the host-side BPM-home reset alone
    //      moved the live session; don't start a 2nd local Moonlight process.
    // Mirrors launchSteamGame() exactly (same one-session guard / stream path) but
    // sends the bare `steam-bigpicture` instead of `steam-launch <appid>`. The gate
    // is THIS client's own shellState, NOT the host's session flag.
    function launchSteamBigPicture() {
        root.userActivity();
        // Fire the host-side BPM-home reset (fire-and-forget; reply is just ok/error).
        steamBigPictureReq.request("steam-bigpicture");
        if (root.shellState === "streaming") {
            // This client is already in the stream — the reset moved the live BPM.
            // Don't start a 2nd local Moonlight process.
            return;
        }
        // Not viewing a stream here → start/resume one to the primary target. With
        // no target configured there's nothing to stream into; the reset still fired.
        let ts = root.targets || [];
        if (ts.length > 0)
            root.streamRequested(ts[0]);
        else
            console.log("HomeScreen: steam-bigpicture sent, but no stream target configured");
    }

    // One-shot socket client for `steam-bigpicture`. The reply (a status JSON) is
    // logged on failure; the stream start above doesn't gate on it (the reset and
    // the stream race, exactly like launchSteamGame).
    SocketClient {
        id: steamBigPictureReq
        onResponseReceived: line => {
            if (line.indexOf("\"status\":\"ok\"") === -1)
                console.log("HomeScreen: steam-bigpicture reply: " + line);
        }
        onRequestFailed: console.log("HomeScreen: steam-bigpicture request failed (daemon down?)")
    }

    // === Quit running Steam game choreography ===
    // The active-game popover's "Quit" action →
    //   1. `steam-quit <appid>` → the host gracefully terminates the running game
    //      (SIGTERM to its process group — like Steam's Stop button). Fire-and-
    //      forget; the reply is a status JSON (logged only on non-ok).
    //   2. Close THIS client's Moonlight stream via the existing
    //      streamQuitRequested(targets[0]) path (same teardown the Moonlight
    //      "Quit Stream" action uses).
    // The two race exactly like launchSteamGame's navigate+stream — the host kill
    // and the local stream-close are independent.
    function quitSteamGame(appid) {
        root.userActivity();
        // Fire the host-side graceful kill (fire-and-forget; reply is a status JSON).
        steamQuitReq.request("steam-quit", appid);
        // Close the local Moonlight stream to the primary target, if one is configured.
        let ts = root.targets || [];
        if (ts.length > 0)
            root.streamQuitRequested(ts[0]);
        else
            console.log("HomeScreen: steam-quit " + appid + " sent, but no stream target configured");
    }

    // One-shot socket client for `steam-quit <appid>`. The reply (a status JSON) is
    // logged on a non-ok status; the stream close above doesn't gate on it (the
    // host kill and the local stream-close race, like launchSteamGame).
    SocketClient {
        id: steamQuitReq
        onResponseReceived: line => {
            if (line.indexOf("\"status\":\"ok\"") === -1)
                console.log("HomeScreen: steam-quit reply: " + line);
        }
        onRequestFailed: console.log("HomeScreen: steam-quit request failed (daemon down?)")
    }

    function _focusFirstVisibleRow() {
        // Try the hosted widgets in order; the All Apps entry is the never-strand
        // fallback (always focusable) so focus is never left in limbo.
        if (!widgetHost.focusFirstVisible())
            allAppsEntry.focusFirstChild();
    }

    function focusDefaultPosition() {
        Qt.callLater(function () {
            scrollView.contentY = 0;
            if (!widgetHost.focusFirstVisible())
                allAppsEntry.focusFirstChild();
        });
    }

    // === Hide-from-Recent descriptors (#249 follow-up) ===
    // One descriptor per home widget that may SHADOW a running window, pairing its
    // id with a generalized `hideFromRecent` capability (matcher + user toggle).
    // HomeScreen drives Recent-row suppression from this list (see _recentModel);
    // adding a shadowing widget is just another entry. The user toggle now reads
    // the namespaced widgets.<id>.prefs.hideFromRecent. The focus chain is built
    // generically by WidgetHost; this is additive metadata only, reading the live
    // widget instances via the widgetById aliases. moonlight/recent don't shadow a
    // window (hideFromRecent: null).
    readonly property var _widgets: [
        {
            "id": "moonlight",
            "hideFromRecent": null
        },
        {
            "id": "nowplaying",
            "hideFromRecent": {
                "capable": true,
                "enabled": SettingsStore.widget("nowplaying").prefs.hideFromRecent,
                // true ⇒ this Recent entry is the player the widget represents.
                "matches": function (e) {
                    var np = root.nowPlayingWidget;
                    return np && np.visible && (np.playerDesktopEntry !== "" || np.playerIdentity !== "") && root._entryIsActivePlayer(e, np.playerDesktopEntry, np.playerIdentity);
                }
            }
        },
        {
            "id": "plex",
            "hideFromRecent": {
                "capable": true,
                "enabled": SettingsStore.widget("plex").prefs.hideFromRecent,
                "matches": function (e) {
                    return root.plexWidget && root.plexWidget.visible && root._entryIsPlex(e);
                }
            }
        },
        {
            "id": "recent",
            "hideFromRecent": null
        }
    ]

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

        // Hide apps that an on-screen widget already represents — generalized
        // hide-from-Recent, driven by the widget descriptors. A descriptor opts
        // in via hideFromRecent.capable and the user keeps it on via .enabled;
        // .matches(entry) returns true for the entry to suppress.
        let widgets = root._widgets;
        for (let w = 0; w < widgets.length; w++) {
            let h = widgets[w].hideFromRecent;
            if (h && h.capable && h.enabled) {
                let matches = h.matches;
                result = result.filter(function (e) {
                    return !matches(e);
                });
            }
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

                    ClockText {
                        kind: "time"
                        font.pixelSize: Theme.fontHero
                        font.bold: true
                        color: Theme.textPrimary
                    }

                    ClockText {
                        kind: "date"
                        font.pixelSize: Theme.fontTitle
                        color: Theme.textSecondary
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
                    onWidgetsRequested: root.widgetsRequested()
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

            // === Standardized home widgets (Moonlight, Now Playing, Plex,
            // Recent) — instantiated + focus-chained generically by WidgetHost
            // from the WidgetRegistry. The QuickActions row above and the All Apps
            // entry below remain HomeScreen-owned and are wired in as the host's
            // top/bottom focus anchors; each widget's behaviour is attached below
            // via widgetById (see the "Widget wiring" block). ===
            WidgetHost {
                id: widgetHost
                Layout.fillWidth: true
                topAnchor: statusIcons
                bottomAnchor: allAppsEntry
            }

            // === All Apps entry (→ Library) ===
            NavigableRow {
                id: allAppsEntry
                Layout.fillWidth: true
                Layout.preferredHeight: Theme.cardHeight
                model: 1
                // previousRow is bound by WidgetHost (bottomAnchor) to the last
                // focusable widget's exit row, so Up/B from here lands correctly
                // whichever widgets are enabled.
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
                    readonly property bool isFocused: (index === allAppsEntry.currentIndex && allAppsEntry.activeFocus && !InputMode.mouseMode) || (allAppsMouse.containsMouse && InputMode.mouseMode)
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
                                InputMode.pointerMoved(p.x, p.y);
                            }
                            onClicked: {
                                InputMode.enterMouseMode();
                                allAppsEntry.currentIndex = 0;
                                allAppsEntry.forceActiveFocus();
                                root.libraryRequested();
                            }
                        }
                    }
                }
            }

            // === Hint Bar ===
            HintBar {
                muted: true
                text: {
                    if (root.recentWidget && root.recentWidget.regionFocused) {
                        let idx = root.recentWidget.currentIndex;
                        let model = root._recentModel;
                        let running = (idx >= 0 && idx < model.length && model[idx].running === true);
                        return (running ? "A: Resume" : "A: Launch") + "  |  X: Actions  |  B: Home  |  ←→: Scroll  |  ↑↓: Switch Row";
                    }
                    if (allAppsEntry.activeFocus)
                        return "A: Browse all  |  B: Home  |  ↑↓: Switch Row";
                    return "A: Select  |  B: Home  |  ←→: Scroll  |  ↑↓: Switch Row";
                }
                Layout.bottomMargin: 16
            }
        }
    }

    // Moonlight server context menu (Resume / Quit a live session). Mirrors the
    // Library's stream-card context behavior, positioned over the focused card.
    function _moonlightContext() {
        if (!root.moonlightWidget)
            return;
        let target = root.moonlightWidget.currentTarget;
        let card = root.moonlightWidget.currentCard;
        if (!target || !card)
            return;
        let pos = card.mapToItem(root, card.width / 2, 0);
        popoverMenu.targetX = pos.x;
        popoverMenu.targetY = pos.y;

        let actions = [];
        // Group 1 — stream controls (app function). Offered whenever the host is
        // reachable (not just when a session is detected) — a stream left running
        // suspends in the background and the Sunshine session probe can't always
        // see it, so this is the reliable way to resume or quit it. Resume
        // re-streams (Moonlight resumes an existing session); Quit Stream ends it.
        // Per-item `hint` keeps the footer correct (no "X: Set default" here).
        let hasControls = false;
        if (root.moonlightWidget.currentOnline) {
            actions.push({
                label: "Resume",
                hint: "A: Resume",
                action: function () {
                    root.streamRequested(target);
                }
            });
            actions.push({
                label: "Quit Stream",
                hint: "A: Quit Stream",
                action: function () {
                    root.streamQuitRequested(target);
                }
            });
            hasControls = true;
        }
        // Group 2 — profile picker (active profile). Each entry clones the host
        // target and overrides `.app` (same pattern as the Library apps view);
        // `hostApps` is filled by the provider's `moonlight list`. `dividerBefore`
        // on the first profile draws the line between the two groups.
        let host = target.host || "";
        let apps = (StreamProviders.active.hostApps && StreamProviders.active.hostApps[host]) ? StreamProviders.active.hostApps[host] : [];
        for (let i = 0; i < apps.length; i++) {
            let appName = apps[i];
            actions.push({
                label: (appName === target.app ? "● " : "") + appName,
                hint: "A: Stream   X: Set default",
                dividerBefore: i === 0 && hasControls,
                // A: stream this profile now (one-off).
                action: function () {
                    let t = JSON.parse(JSON.stringify(target));
                    t.app = appName;
                    root.streamRequested(t);
                },
                // X: make this the host's default (what A on the card launches).
                secondaryAction: function () {
                    StreamProviders.active.setHostApp(target.host, appName);
                    NotificationManager.info("moonlight", "Default profile set", appName + " — A on the card now launches it");
                    // Rebuild the (still-open) menu so the ● marker moves to the new
                    // default live. Deferred so the targets binding chain (provider →
                    // ShellLayout → widget → currentTarget) settles before the rebuild.
                    Qt.callLater(root._moonlightContext);
                }
            });
        }
        if (apps.length === 0) {
            // Not discovered yet (or host offline): offer the default launch and
            // kick discovery so the next open lists the profiles.
            actions.push({
                label: "Stream " + (target.app || "Desktop"),
                hint: "A: Stream",
                dividerBefore: hasControls,
                action: function () {
                    root.streamRequested(target);
                }
            });
            StreamProviders.active.discoverApps();
        }
        popoverMenu.actions = actions;
        popoverMenu.opened = true;
        popoverMenu.forceActiveFocus();
    }

    // Active-game context menu (Resume / Quit) for the RUNNING Steam game's poster
    // card. Opened by the X face over that card (only the running card emits — see
    // SteamCard's guard), positioned over it. Mirrors _moonlightContext's
    // mapToItem positioning, anchored on moonlightWidget.runningCard.
    function _steamGameContext(appid) {
        if (!root.moonlightWidget)
            return;
        let card = root.moonlightWidget.runningCard;
        if (!card)
            return;
        let pos = card.mapToItem(root, card.width / 2, 0);
        popoverMenu.targetX = pos.x;
        popoverMenu.targetY = pos.y;
        popoverMenu.actions = [
            {
                label: "Resume",
                hint: "A: Resume",
                // Reconnect/stream the running session. Reuses the Big-Picture
                // stream path (navigate BPM to this game, then stream targets[0])
                // and respects the one-session guard: if THIS client is already
                // streaming, launchSteamGame just moves the live BPM (no 2nd stream).
                action: function () {
                    root.launchSteamGame(appid);
                }
            },
            {
                label: "Quit",
                hint: "A: Quit",
                // Gracefully kill the running game on the host (`steam-quit`), then
                // close THIS client's Moonlight stream (streamQuitRequested).
                action: function () {
                    root.quitSteamGame(appid);
                }
            }
        ];
        popoverMenu.opened = true;
        popoverMenu.forceActiveFocus();
    }

    // Shared Now-Playing context-menu opener (quit the player).
    function _mediaContext(widget) {
        if (!widget)
            return;
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

    // Recent-row activation: focus a running window, or launch a non-running app.
    function _recentActivate(entry) {
        if (entry.running === true)
            root.appFocusRequested(entry.address);
        else
            root.launchApp(entry);
    }

    // Recent-row context popover (Resume/Quit for a running app, Launch otherwise),
    // positioned over the focused card.
    function _recentContext(entry, card) {
        if (!entry || !card)
            return;
        let pos = card.mapToItem(root, card.width / 2, 0);
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

    PopoverMenu {
        id: popoverMenu
        onClosed: {
            popoverMenu.opened = false;
            root._focusFirstVisibleRow();
        }
    }

    // === Widget wiring ===
    // WidgetHost instantiates the standardized widgets from the registry; these
    // Bindings push HomeScreen-owned inputs into the live instances and the
    // Connections attach each widget's behaviour. Targets resolve via the
    // widgetById aliases (null until a Loader resolves — each Binding/Connections
    // sits idle until then).

    // --- Moonlight ---
    Binding {
        target: root.moonlightWidget
        property: "targets"
        value: root.targets
        when: root.moonlightWidget !== null
    }
    Binding {
        target: root.moonlightWidget
        property: "shellState"
        value: root.shellState
        when: root.moonlightWidget !== null
    }
    Connections {
        target: root.moonlightWidget
        ignoreUnknownSignals: true
        function onEscaped() {
            root.userActivity();
            root.focusDefaultPosition();
        }
        function onStreamRequested(target) {
            root.streamRequested(target);
        }
        function onStreamQuitRequested(target) {
            root.streamQuitRequested(target);
        }
        function onEnsureVisibleRequested(item) {
            scrollView.ensureVisible(item);
        }
        function onContextRequested() {
            root._moonlightContext();
        }
        function onGameSelected(appid) {
            root.launchSteamGame(appid);
        }
        function onGameContextRequested(appid) {
            root._steamGameContext(appid);
        }
        function onOpenBigPictureRequested() {
            root.launchSteamBigPicture();
        }
    }

    // --- Now Playing ---
    Connections {
        target: root.nowPlayingWidget
        ignoreUnknownSignals: true
        function onEscaped() {
            root.userActivity();
            root.focusDefaultPosition();
        }
        function onOpenAppRequested(desktopEntry, identity) {
            root.openMediaApp(desktopEntry, identity);
        }
        function onContextRequested() {
            root._mediaContext(root.nowPlayingWidget);
        }
        function onActiveFocusChanged() {
            if (root.nowPlayingWidget && root.nowPlayingWidget.activeFocus)
                scrollView.ensureVisible(root.nowPlayingWidget);
        }
    }

    // --- Plex ---
    Connections {
        target: root.plexWidget
        ignoreUnknownSignals: true
        function onEscaped() {
            root.userActivity();
            root.focusDefaultPosition();
        }
        function onOpenPlexRequested() {
            root.openPlexApp();
        }
        function onEnsureVisibleRequested(item) {
            scrollView.ensureVisible(item);
        }
    }

    // --- Recent ---
    Binding {
        target: root.recentWidget
        property: "model"
        value: root._recentModel
        when: root.recentWidget !== null
    }
    Binding {
        target: root.recentWidget
        property: "recentSmall"
        value: root._recentSmall
        when: root.recentWidget !== null
    }
    Connections {
        target: root.recentWidget
        ignoreUnknownSignals: true
        function onEscaped() {
            root.userActivity();
            root.focusDefaultPosition();
        }
        function onEntryActivated(entry) {
            root._recentActivate(entry);
        }
        function onEntryContextRequested(entry, card) {
            root._recentContext(entry, card);
        }
        function onEnsureVisibleRequested(item) {
            scrollView.ensureVisible(item);
        }
    }
}
