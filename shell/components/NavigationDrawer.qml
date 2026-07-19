import QtQuick
import QtQuick.Layouts
import "lib"
import "resumeModel.js" as ResumeModel

// Left navigation drawer (#216, epic #133 Phase 1). Top→bottom:
//   1. Clock/date header
//   2. Resume HERO zone — large app tiles merging running windows + recent apps
//      (deduped, running-first). Collapses entirely when nothing is running/recent.
//   3. Nav cluster — the Home row (navList) + the QuickActions row (drawerActions)
//   4. Now-Playing mini-strip — a MediaWidget, hidden when idle (no MPRIS player)
//   5. Status glyph line — a non-focusable readout pinned to the bottom
//      (online · volume · controller battery · notifications)
//
// Focus chain (down-flow): Resume row → navList → drawerActions → MediaWidget.
// Up reverses. The status line is NOT in the focus chain.
Drawer {
    id: root
    edge: "left"
    drawerWidth: Units.gridUnit * 18

    property bool overlayMode: false

    // Running windows (Hyprland clients) — wired from ShellLayout
    // (AppLifecycleManager.runningWindows). Feeds the resume merge.
    property var runningWindows: []
    // Gamepad fleet model (InputManager.pads) — wired from ShellLayout. Drives the
    // controller-battery status glyph; empty ⇒ that glyph is hidden.
    property var pads: []

    signal settingsRequested
    signal widgetsSelected
    signal notificationCenterRequested
    signal powerRequested
    signal networkRequested(var anchorRect)
    signal volumeRequested(var anchorRect)
    signal homeSelected

    // Resume-tile activation contract (mirrors HomeScreen; ShellLayout forwards
    // each to shell.qml → AppLifecycle).
    signal appLaunchRequested(var app)
    signal appResumeRequested(var app, string address)
    signal appFocusRequested(string address)
    signal appCloseRequested(string address)

    // === Resume model — running windows + non-running recents, deduped ===
    // Pure merge lives in resumeModel.js (headless-testable). WindowMatcher is a
    // components singleton exposing execBasename/normalize (the matcher contract).
    readonly property var resumeModel: ResumeModel.build(root.runningWindows, RecentsTracker.recentApps, AppDiscoveryManager.applications, WindowMatcher)
    readonly property bool hasResume: root.resumeModel.length > 0

    // Best wireless pad reporting a battery level (mirrors HomeScreen._batteryPad).
    // null when only wired pads / none are connected — the glyph then hides.
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

    // Two large tiles across the drawer, inset a gridUnit each side. The tiles are
    // half-width (not full-width) and sit inside a gridUnit gutter, so BaseCard's
    // center-origin focus scale (6%) stays within the drawer edge — the FocusFrame
    // full-width overflow hazard doesn't bite here (BaseCard exposes no
    // availableWidth to clamp, so the geometry is the guard).
    readonly property int _resumeTileWidth: Math.round((root.drawerWidth - Units.gridUnit * 2 - Theme.cardSpacing) / 2)
    readonly property int _resumeTileHeight: Math.round(root._resumeTileWidth * 0.82)

    onOpenedChanged: {
        if (opened) {
            navList.currentIndex = 0;
            navFocusTimer.restart();
        }
    }

    Timer {
        id: navFocusTimer
        interval: 50
        // Land on the resume row's first tile when present, else the Home row.
        onTriggered: {
            if (root.hasResume)
                resumeRow.focusFirstChild();
            else
                navList.forceActiveFocus();
        }
    }

    // Return controller focus to the QuickActions row — called when an anchored
    // popover (Volume/Network) closes while the drawer is still open, so focus
    // lands back on the glyph the user activated rather than jumping to home.
    function focusQuickActions() {
        drawerActions.forceActiveFocus();
    }

    // Resume-tile activation (mirrors HomeScreen._recentActivate): focus a running
    // window, else launch a non-running app + record it. Closes the drawer either
    // way (a successful jump-back-in, like the nav actions do).
    function _resumeActivate(entry) {
        if (entry.running === true) {
            root.appFocusRequested(entry.address);
        } else {
            root.appLaunchRequested(entry);
            RecentsTracker.recordLaunch(entry);
        }
        root.closed();
    }

    // Resume-tile context popover (mirrors HomeScreen._recentContext): Resume/Quit
    // for a running app, Launch otherwise. Positioned over the focused tile.
    function _resumeContext(entry, card) {
        if (!entry || !card)
            return;
        var pos = card.mapToItem(root, card.width / 2, 0);
        popoverMenu.targetX = pos.x;
        popoverMenu.targetY = pos.y;
        if (entry.running === true) {
            var addr = entry.address;
            popoverMenu.actions = [
                {
                    label: "Resume",
                    action: function () {
                        root.appFocusRequested(addr);
                        root.closed();
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
            var app = entry;
            popoverMenu.actions = [
                {
                    label: "Launch",
                    action: function () {
                        root.appLaunchRequested(app);
                        RecentsTracker.recordLaunch(app);
                        root.closed();
                    }
                }
            ];
        }
        popoverMenu.opened = true;
        popoverMenu.forceActiveFocus();
    }

    ColumnLayout {
        anchors.fill: parent
        spacing: 0

        // === Clock + Date Header ===
        Item {
            Layout.fillWidth: true
            Layout.preferredHeight: Units.gridUnit * 5

            ColumnLayout {
                anchors.centerIn: parent
                spacing: 12

                ClockText {
                    kind: "time"
                    running: root.opened
                    font.pixelSize: Theme.fontHero * 0.7
                    font.bold: true
                    color: Theme.textPrimary
                    Layout.alignment: Qt.AlignHCenter
                }

                ClockText {
                    kind: "date"
                    running: root.opened
                    font.pixelSize: Theme.fontBody
                    color: Theme.textSecondary
                    Layout.alignment: Qt.AlignHCenter
                }
            }
        }

        Rectangle {
            Layout.fillWidth: true
            height: 2
            color: Theme.surfaceBorder
        }

        // === Resume HERO zone (collapses entirely when empty) ===
        ColumnLayout {
            id: resumeZone
            Layout.fillWidth: true
            visible: root.hasResume
            Layout.preferredHeight: visible ? implicitHeight : 0
            Layout.topMargin: visible ? Units.spacingMD : 0
            spacing: Units.spacingSM

            Text {
                text: "Resume"
                Layout.leftMargin: Units.gridUnit
                font.pixelSize: Theme.fontBody
                font.bold: true
                color: Theme.textSecondary
            }

            // Horizontal rail of large tiles — a NavigableRow so D-pad Left/Right +
            // focusHistoryId ordering work (matches how AppsWidget rails the same
            // merged model). Reuses AppCard for the icon + letter-initial fallback
            // (#114 box-art graceful fallback) and the ember running dot.
            NavigableRow {
                id: resumeRow
                Layout.fillWidth: true
                Layout.leftMargin: Math.max(0, Units.gridUnit - 16)
                Layout.rightMargin: Units.gridUnit
                Layout.preferredHeight: root._resumeTileHeight + Units.spacingMD
                keyNavigationWraps: false
                model: root.resumeModel
                // Top of the focus chain (Up = no-op); Down → the Home row.
                previousRow: null
                nextRow: navList

                delegate: AppCard {
                    required property int index
                    required property var modelData
                    width: root._resumeTileWidth
                    height: root._resumeTileHeight
                    app: modelData
                    running: modelData.running === true
                    focus: index === resumeRow.currentIndex
                    onActivated: {
                        resumeRow.currentIndex = index;
                        root._resumeActivate(modelData);
                    }
                }

                onContextRequested: {
                    if (currentItem && currentIndex >= 0 && currentIndex < root.resumeModel.length)
                        root._resumeContext(root.resumeModel[currentIndex], currentItem);
                }
                onEscaped: root.closed()
            }
        }

        Rectangle {
            Layout.fillWidth: true
            Layout.topMargin: root.hasResume ? Units.spacingMD : 0
            height: 2
            color: Theme.surfaceBorder
            visible: root.hasResume
        }

        // === Nav cluster: Home row ===
        ListView {
            id: navList
            Layout.fillWidth: true
            Layout.preferredHeight: contentHeight
            // Widgets is reached via the QuickActions row below (idx 2) / the
            // `intent settings:widgets` path — no redundant drawer row for it.
            model: [
                {
                    label: "Home",
                    icon: "\u{1F3E0}",
                    action: "home"
                }
            ]
            // Hold default focus the instant the drawer opens so there is never a
            // handler-less window before navFocusTimer runs; the timer then refines
            // focus onto the resume row's first tile when the hero is present.
            focus: true
            interactive: false
            currentIndex: 0

            delegate: Rectangle {
                required property int index
                required property var modelData
                width: navList.width
                height: Math.round(Units.gridUnit * 2.2)
                color: navList.currentIndex === index && navList.activeFocus ? Theme.surfaceHover : "transparent"
                Accessible.role: Accessible.Button
                Accessible.name: modelData.label
                Accessible.focusable: true
                Accessible.onPressAction: root._activateNav(index)
                Behavior on color {
                    ColorAnimation {
                        duration: 150
                    }
                }

                FocusAccentBar {
                    active: navList.currentIndex === index && navList.activeFocus
                }

                RowLayout {
                    anchors.fill: parent
                    anchors.leftMargin: Units.gridUnit
                    anchors.rightMargin: Units.gridUnit
                    spacing: Units.spacingLG

                    Text {
                        text: modelData.icon
                        font.pixelSize: Theme.fontTitle
                        Layout.preferredWidth: Units.gridUnit
                        horizontalAlignment: Text.AlignHCenter
                    }
                    Text {
                        text: modelData.label
                        font.pixelSize: Theme.fontTitle
                        color: Theme.textPrimary
                        Layout.fillWidth: true
                    }
                }

                MouseArea {
                    anchors.fill: parent
                    hoverEnabled: true
                    cursorShape: Qt.PointingHandCursor
                    onClicked: root._activateNav(index)
                }
            }

            Keys.onDownPressed: {
                if (currentIndex < count - 1)
                    currentIndex++;
                else
                    drawerActions.forceActiveFocus();
            }
            Keys.onUpPressed: {
                if (currentIndex > 0)
                    currentIndex--;
                else if (root.hasResume)
                    resumeRow.focusFirstChild();
            }
            Keys.onReturnPressed: root._activateNav(currentIndex)
        }

        // === Nav cluster: Quick Actions row ===
        Item {
            Layout.fillWidth: true
            Layout.topMargin: Units.spacingSM
            // Reserve full label height plus a safe-margin so the floating
            // focus label never bleeds into the row below (#142) — now the
            // Now-Playing strip rather than the viewport bottom.
            Layout.preferredHeight: drawerActions.implicitHeight + Units.spacingLG
            Layout.bottomMargin: Units.spacingLG

            QuickActions {
                id: drawerActions
                anchors.left: parent.left
                anchors.right: parent.right
                anchors.leftMargin: Units.gridUnit
                anchors.rightMargin: Units.gridUnit
                anchors.top: parent.top
                // Bubble Escape/B up to the Drawer so it closes instead of
                // opening Settings.
                escapeRequestsSettings: false
                // Cap width to the drawer so overflowing actions scroll.
                maxContentWidth: width

                onFocusUpRequested: navList.forceActiveFocus()
                // Down → the Now-Playing strip when a player is present; a no-op
                // (focus stays) when idle, since focusFirstChild returns false.
                onFocusDownRequested: mediaWidget.focusFirstChild()
                onSettingsRequested: {
                    root.closed();
                    root.settingsRequested();
                }
                onWidgetsRequested: {
                    root.closed();
                    root.widgetsSelected();
                }
                onNotificationCenterRequested: {
                    root.closed();
                    root.notificationCenterRequested();
                }
                onPowerRequested: {
                    root.closed();
                    root.powerRequested();
                }
                onNetworkRequested: anchorRect => {
                    // Keep the drawer open underneath; the popover paints on
                    // top (higher z) anchored to this glyph (#118).
                    root.networkRequested(anchorRect);
                }
                onVolumeRequested: anchorRect => {
                    root.volumeRequested(anchorRect);
                }
            }
        }

        // === Now-Playing mini-strip (hides when idle) ===
        // Reuses MediaWidget (NowPlayingCard + MPRIS); collapses to zero height and
        // out of the focus chain when there is no player (hasPlayer false). Up →
        // the QuickActions row via the shared focus chain (previousRow). Its
        // Left/Right/Return transport nav is internal.
        MediaWidget {
            id: mediaWidget
            Layout.fillWidth: true
            Layout.leftMargin: Units.gridUnit
            Layout.rightMargin: Units.gridUnit
            Layout.topMargin: hasPlayer ? Units.spacingMD : 0
            Layout.preferredHeight: hasPlayer ? implicitHeight : 0
            // visible is inherited from the Widget base (`visible: wantVisible`,
            // and MprisPlayerBase sets wantVisible = hasPlayer) — no override here.
            previousRow: drawerActions
            onEscaped: root.closed()
        }

        // === Spacer — absorbs slack so the status line pins to the bottom ===
        Item {
            Layout.fillWidth: true
            Layout.fillHeight: true
        }

        // === Status glyph line (non-focusable readout, bottom-pinned) ===
        Rectangle {
            Layout.fillWidth: true
            height: 2
            color: Theme.surfaceBorder
        }

        Flow {
            Layout.fillWidth: true
            Layout.leftMargin: Units.gridUnit
            Layout.rightMargin: Units.gridUnit
            Layout.topMargin: Units.spacingMD
            Layout.bottomMargin: Units.spacingLG
            spacing: Units.spacingSM

            // Online — always available (NetworkManager singleton).
            StatusPill {
                pillState: NetworkManager.connected ? "good" : "bad"
                text: NetworkManager.connected ? "Online" : "Offline"
            }

            // Volume — always available (AudioController singleton).
            StatusPill {
                pillState: "neutral"
                showDot: false
                text: AudioController.muted ? "\u{1F507} Muted" : "\u{1F509} " + AudioController.volume + "%"
            }

            // Controller battery — only when a wireless pad reports a level.
            StatusPill {
                visible: root._batteryPad !== null
                showDot: false
                pillState: (root._batteryPad !== null && root._batteryPad.batteryLevel <= 15) ? "bad" : "good"
                text: root._batteryPad !== null ? "\u{1F50B} " + root._batteryPad.batteryLevel + "%" : ""
            }

            // Notifications — only when there is something unread (explicit empty
            // state: no pill at zero, mirroring the QuickActions CountBadge).
            // "neutral" (not "warn") — a warn pill renders gold text, and the
            // palette rule forbids gold for text; this is always-on chrome.
            StatusPill {
                visible: NotificationManager.unreadCount > 0
                pillState: "neutral"
                showDot: false
                text: "\u{1F514} " + NotificationManager.unreadCount
            }
        }
    }

    // Resume-tile context menu (Resume/Quit or Launch), positioned over the tile.
    PopoverMenu {
        id: popoverMenu
        onClosed: {
            popoverMenu.opened = false;
            if (root.hasResume)
                resumeRow.forceActiveFocus();
            else
                navList.forceActiveFocus();
        }
    }

    function _activateNav(index) {
        let items = navList.model;
        if (index < 0 || index >= items.length)
            return;
        switch (items[index].action) {
        case "home":
            root.homeSelected();
            break;
        }
    }
}
