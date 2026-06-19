import QtQuick
import QtQuick.Layouts
import "lib"

// Steam-library poster view — the medium/large rendering of the unified Moonlight
// home widget (MoonlightWidget hosts this; it is NOT an independently-enableable
// widget anymore). ONE poster row with a segmented header that flips between
// "Recently Played" and "Library", fed by the daemon's `steam-library` IPC, plus
// a right-justified, status-only session indicator opposite the tabs.
//
// Selecting a card launches that Steam game on the host (`steam-launch <appid>`)
// and then starts the existing single-target Moonlight stream — there is exactly
// one session; cards do not own a stream. The stream START stays in QML
// (HomeScreen owns the existing Moonlight path); this view only emits
// `gameSelected(appid)`.
//
// `posterScale` is set by the host so the same view renders at two sizes:
//   medium = 0.62 (smaller posters)
//   large  = 0.82 (full posters)
//
// Health-aware: a `ServiceMonitor` keyed on "steam" collapses the view when
// unconfigured/empty and shows a graceful `ServiceStatusNotice` when the host is
// down. The reply's `runningAppid` badges the card whose appid matches as
// "Playing" — source of truth is desktop-1, not which card was tapped.
//
// Focus contract (host uses these): `firstRow`/`lastRow` resolve to the first/
// last *visible* internal region (segment chips, poster row). The session
// indicator is non-focusable (status only).
ColumnLayout {
    id: root

    property Item previousRow: null
    property Item nextRow: null

    // The single configured Moonlight target (used only for the session
    // indicator's `sunshine-status` probe). Null ⇒ no indicator.
    property var target: null

    // Poster scale set by the host (medium vs large). A reflow, not a crop.
    property real posterScale: 0.62

    signal escaped
    // Emitted when a card is activated; HomeScreen launches + streams the appid.
    signal gameSelected(int appid)
    signal ensureVisibleRequested(var item)

    spacing: Units.spacingMD

    // === Data (populated from `steam-library`) ===
    property var recentItems: []
    property var allItems: []
    // The host's currently-running Steam appid (or -1 when nothing is running).
    // Drives the per-card "Playing" badge.
    property int runningAppid: -1

    readonly property bool _hasRecent: recentItems.length > 0
    readonly property bool _hasAll: allItems.length > 0

    // === Segment (Recently Played vs Library) ===
    property string _segment: "recent"
    readonly property var _segmentOptions: {
        let o = [];
        if (_hasRecent)
            o.push({
                "label": "Recently Played",
                "value": "recent"
            });
        if (_hasAll)
            o.push({
                "label": "Library",
                "value": "all"
            });
        return o;
    }
    readonly property var _activeItems: _segment === "recent" ? recentItems : allItems

    readonly property bool rowFocused: posterRow.activeFocus || segmentChips.activeFocus

    visible: steamMon.degraded || (steamMon.ok && (root._hasRecent || root._hasAll))

    // === Home-tile focus contract ===
    readonly property var firstRow: segmentChips
    readonly property var lastRow: posterRow
    readonly property bool canFocus: visible && (root._hasRecent || root._hasAll)
    readonly property bool regionFocused: rowFocused

    function focusFirstChild() {
        if (!root.canFocus)
            return false;
        let r = root.firstRow;
        if (r && r.visible) {
            if (r.focusFirstChild)
                r.focusFirstChild();
            else
                r.forceActiveFocus();
            return true;
        }
        return false;
    }

    // === Poster geometry (reflow by posterScale) ===
    readonly property int posterW: Math.round(Theme.cardWidth * posterScale)
    readonly property int posterH: Math.round(posterW * 1.5)
    readonly property int _captionBand: Math.round(Theme.fontSmall * 1.4 + Units.spacingSM * 2)
    readonly property int steamRowHeight: posterH + _captionBand

    function refresh() {
        steamMon.refresh();
    }

    ServiceMonitor {
        id: steamMon
        healthKey: "steam"
        dataCommand: "steam-library"
        dataIntervalMs: 30000  // matches daemon service_health POLL_INTERVAL
        onUpdated: {
            if (steamMon.ok && steamMon.data) {
                root.recentItems = steamMon.data.recentlyPlayed || [];
                root.allItems = steamMon.data.allGames || [];
                let ra = steamMon.data.runningAppid;
                root.runningAppid = (typeof ra === "number" && ra > 0) ? ra : -1;
            } else {
                root.recentItems = [];
                root.allItems = [];
                root.runningAppid = -1;
            }
            // Keep the active segment on something that has content.
            if (root._segment === "recent" && !root._hasRecent && root._hasAll)
                root._segment = "all";
            else if (root._segment === "all" && !root._hasAll && root._hasRecent)
                root._segment = "recent";
        }
    }

    ServiceStatusNotice {
        Layout.fillWidth: true
        serviceName: "Steam"
        status: steamMon.status
    }

    // === Header: segment chips + right-justified session indicator ===
    RowLayout {
        Layout.fillWidth: true
        visible: root._hasRecent || root._hasAll
        spacing: Units.spacingXL

        FilterChips {
            id: segmentChips
            Layout.alignment: Qt.AlignVCenter
            options: root._segmentOptions
            currentIndex: {
                for (var i = 0; i < root._segmentOptions.length; i++) {
                    if (root._segmentOptions[i].value === root._segment)
                        return i;
                }
                return 0;
            }
            previousRow: root.previousRow
            nextRow: posterRow
            onFilterChanged: value => root._segment = value
            onEscaped: root.escaped()
            // Defer so the Flickable geometry is settled (the view may have been
            // hidden — Steam down — and just re-revealed) before we scroll to it.
            onActiveFocusChanged: if (activeFocus)
                Qt.callLater(() => root.ensureVisibleRequested(segmentChips))
        }

        Item {
            Layout.fillWidth: true
        }

        // Session indicator — status only (NOT focusable, NOT a pill, NOT
        // actionable). Tells the user at a glance whether there's a Moonlight
        // session to resume on the configured target, driven by the same
        // `sunshine-status` probe the server cards use.
        SessionIndicator {
            Layout.alignment: Qt.AlignVCenter
            target: root.target
        }
    }

    // === The one poster row (shows the active segment) ===
    NavigableRow {
        id: posterRow
        visible: root._activeItems.length > 0
        Layout.fillWidth: true
        // Extra breathing room between the chip strip and the posters (on top of
        // the ColumnLayout spacing) so the pills don't crowd the row below.
        Layout.topMargin: Units.spacingMD
        Layout.preferredHeight: root.steamRowHeight
        keyNavigationWraps: true
        previousRow: segmentChips
        nextRow: root.nextRow
        model: root._activeItems
        onActiveFocusChanged: if (activeFocus)
            Qt.callLater(() => root.ensureVisibleRequested(posterRow))
        onActivated: {
            let it = root._activeItems[posterRow.currentIndex];
            if (it)
                root.gameSelected(it.appid);
        }
        onEscaped: root.escaped()

        delegate: SteamCard {
            required property int index
            required property var modelData
            posterWidth: root.posterW
            posterHeight: root.posterH
            showCaption: true
            title: modelData.name || ""
            art: modelData.art || ""
            headerArt: modelData.headerArt || ""
            playing: root.runningAppid > 0 && modelData.appid === root.runningAppid
            focus: index === posterRow.currentIndex
            onActivated: root.gameSelected(modelData.appid)
        }
    }
}
