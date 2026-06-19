import QtQuick
import QtQuick.Layouts
import "lib"

// Home-screen Steam widget — ONE poster row with a segmented header that flips
// between "Recently Played" (recently-played rail) and "Library" (all installed
// games), fed by the daemon's `steam-library` IPC. Mirrors `PlexWidget`: a
// single prominent row of poster cards, not two stacked rows. The segment
// control appears only when BOTH segments have content (otherwise the lone
// segment's name is just a header). `size` reformats the row (not a scale):
//   small  = poster-only rail (caption band removed) — glanceable
//   medium = posters + title captions (default)
//
// Selecting a card launches that Steam game on the host (`steam-launch <appid>`)
// and then starts the existing Moonlight stream — the old GameStream "pick a
// game, it streams" flow, rebuilt on Sunshine. The stream START stays in QML
// (HomeScreen owns the existing Moonlight path); this widget only emits
// `gameSelected(appid)`.
//
// Health-aware: a `ServiceMonitor` keyed on "steam" collapses the widget when
// unconfigured/empty and shows a graceful `ServiceStatusNotice` when the host is
// down.
//
// Focus contract (host uses these): `firstRow`/`lastRow` resolve to the first/
// last *visible* internal region (segment chips, poster row).
ColumnLayout {
    id: root

    property Item previousRow: null
    property Item nextRow: null
    property bool widgetEnabled: true
    // "small" | "medium".
    property string size: "medium"

    signal escaped
    // Emitted when a card is activated; HomeScreen launches + streams the appid.
    signal gameSelected(int appid)
    signal ensureVisibleRequested(var item)

    spacing: Units.spacingMD

    // === Data (populated from `steam-library`) ===
    property var recentItems: []
    property var allItems: []

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

    visible: root.widgetEnabled && (steamMon.degraded || (steamMon.ok && (root._hasRecent || root._hasAll)))

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

    // === Poster geometry (reflow by size) ===
    readonly property real _posterScale: root.size === "small" ? 0.50 : 0.62
    readonly property bool _showCaption: root.size !== "small"
    readonly property int posterW: Math.round(Theme.cardWidth * _posterScale)
    readonly property int posterH: Math.round(posterW * 1.5)
    readonly property int _captionBand: Math.round(Theme.fontSmall * 1.4 + Units.spacingSM * 2)
    readonly property int steamRowHeight: posterH + (_showCaption ? _captionBand : 0)

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
            } else {
                root.recentItems = [];
                root.allItems = [];
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

    // === Header: segment chips ===
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
            // Defer so the Flickable geometry is settled (the widget may have been
            // hidden — Steam down — and just re-revealed) before we scroll to it.
            onActiveFocusChanged: if (activeFocus)
                Qt.callLater(() => root.ensureVisibleRequested(segmentChips))
        }

        Item {
            Layout.fillWidth: true
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
            showCaption: root._showCaption
            title: modelData.name || ""
            art: modelData.art || ""
            headerArt: modelData.headerArt || ""
            focus: index === posterRow.currentIndex
            onActivated: root.gameSelected(modelData.appid)
        }
    }
}
