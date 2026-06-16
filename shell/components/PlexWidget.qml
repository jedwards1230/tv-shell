import QtQuick
import QtQuick.Layouts
import "lib"

// Home-screen Plex widget: two controller-navigable poster rows — "On Deck"
// (continue-watching) and "Recently Added" — fed by the daemon's `plex-hubs`
// IPC. A standardized home widget (#249): honors a `size` (small | medium) that
// scales the poster footprint, and its Recently Added row carries **dynamic
// filter chips** (All / Movies / TV / Music) that only appear for categories
// actually present — a library with no music never shows a Music pill.
//
// Health-aware (service-health bus): a `ServiceMonitor` keyed on "plex" tells
// three states apart — unconfigured/empty ⇒ collapse; reachable with items ⇒
// poster rows; configured but down ⇒ a graceful `ServiceStatusNotice`.
//
// Focus contract (host uses these): `firstRow`/`lastRow` resolve to the first/
// last *visible* internal region (On Deck row, chips, Recently Added row); the
// internal chain lets NavigableRow/FilterChips skip a hidden region.
ColumnLayout {
    id: root

    property Item previousRow: null
    property Item nextRow: null
    property bool widgetEnabled: true
    // Standardized widget size: "small" (compact posters) | "medium".
    property string size: "medium"

    readonly property bool rowFocused: onDeckRow.activeFocus || recentRow.activeFocus || chips.activeFocus

    signal escaped
    signal openPlexRequested
    signal ensureVisibleRequested(var item)

    spacing: Units.spacingMD

    // === Data (populated from `plex-hubs`) ===
    property var onDeckItems: []
    property var recentItems: []

    readonly property bool _hasOnDeck: onDeckItems.length > 0
    readonly property bool _hasRecent: recentItems.length > 0

    // === Recently Added category filter (dynamic chips) ===
    property string _recentFilter: "all"

    readonly property var _categories: {
        let cats = {
            "movie": false,
            "tv": false,
            "music": false
        };
        for (let i = 0; i < recentItems.length; i++) {
            let k = (recentItems[i].kind || "").toLowerCase();
            if (k === "movie")
                cats.movie = true;
            else if (k === "episode" || k === "season" || k === "show")
                cats.tv = true;
            else if (k === "album" || k === "track")
                cats.music = true;
        }
        return cats;
    }
    readonly property var _chipOptions: {
        let o = [
            {
                "label": "All",
                "value": "all"
            }
        ];
        if (_categories.movie)
            o.push({
                "label": "Movies",
                "value": "movie"
            });
        if (_categories.tv)
            o.push({
                "label": "TV",
                "value": "tv"
            });
        if (_categories.music)
            o.push({
                "label": "Music",
                "value": "music"
            });
        return o;
    }
    // Chips only earn their place when there's more than one category to pick
    // between (All + ≥2 real categories) — otherwise filtering is a no-op.
    readonly property bool _chipsVisible: _hasRecent && _chipOptions.length > 2

    readonly property var _filteredRecent: {
        if (_recentFilter === "all")
            return recentItems;
        return recentItems.filter(function (it) {
            let k = (it.kind || "").toLowerCase();
            if (_recentFilter === "movie")
                return k === "movie";
            if (_recentFilter === "tv")
                return k === "episode" || k === "season" || k === "show";
            if (_recentFilter === "music")
                return k === "album" || k === "track";
            return true;
        });
    }

    // Drop a stale filter when its category disappears on a data refresh (so the
    // chip highlight and the rendered list never disagree).
    onRecentItemsChanged: {
        var cats = {
            "all": true,
            "movie": false,
            "tv": false,
            "music": false
        };
        for (var i = 0; i < root.recentItems.length; i++) {
            var k = (root.recentItems[i].kind || "").toLowerCase();
            if (k === "movie")
                cats.movie = true;
            else if (k === "episode" || k === "season" || k === "show")
                cats.tv = true;
            else if (k === "album" || k === "track")
                cats.music = true;
        }
        if (!cats[root._recentFilter])
            root._recentFilter = "all";
    }

    visible: root.widgetEnabled && (plexMon.degraded || (plexMon.ok && (root._hasOnDeck || root._hasRecent)))

    // First/last *visible* internal region, for the host's neighbour wiring.
    readonly property var firstRow: _hasOnDeck ? onDeckRow : (_chipsVisible ? chips : recentRow)
    readonly property var lastRow: (_filteredRecent.length > 0) ? recentRow : (_chipsVisible ? chips : onDeckRow)

    // === Home-tile focus contract ===
    readonly property bool canFocus: visible && (root._hasOnDeck || root._hasRecent)
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

    // === Poster geometry (scaled by size; shared by every card so rows align) ===
    readonly property real _posterScale: root.size === "small" ? 0.46 : 0.62
    readonly property int posterW: Math.round(Theme.cardWidth * _posterScale)
    readonly property int posterH: Math.round(posterW * 1.5)
    readonly property int plexRowHeight: posterH + Math.round(Theme.fontSmall * 1.4 + Theme.fontCaption * 1.4 + Units.spacingSM * 2)

    function refresh() {
        plexMon.refresh();
    }

    ServiceMonitor {
        id: plexMon
        healthKey: "plex"
        dataCommand: "plex-hubs"
        dataIntervalMs: 60000
        onUpdated: {
            if (plexMon.ok && plexMon.data) {
                root.onDeckItems = plexMon.data.onDeck || [];
                root.recentItems = plexMon.data.recentlyAdded || [];
            } else {
                root.onDeckItems = [];
                root.recentItems = [];
            }
        }
    }

    ServiceStatusNotice {
        Layout.fillWidth: true
        serviceName: "Plex"
        status: plexMon.status
    }

    // === On Deck ===
    Text {
        visible: root._hasOnDeck
        text: "On Deck"
        font.pixelSize: Theme.fontTitle
        font.bold: true
        color: Theme.textPrimary
    }

    NavigableRow {
        id: onDeckRow
        visible: root._hasOnDeck
        Layout.fillWidth: true
        Layout.preferredHeight: root.plexRowHeight
        keyNavigationWraps: true
        previousRow: root.previousRow
        nextRow: root._chipsVisible ? chips : recentRow
        model: root.onDeckItems
        onActiveFocusChanged: if (activeFocus)
            root.ensureVisibleRequested(this)
        onActivated: root.openPlexRequested()
        onEscaped: root.escaped()

        delegate: PlexCard {
            required property int index
            required property var modelData
            posterWidth: root.posterW
            posterHeight: root.posterH
            title: modelData.title || ""
            subtitle: modelData.subtitle || ""
            art: modelData.art || ""
            progress: modelData.progress || 0
            focus: index === onDeckRow.currentIndex
            onActivated: root.openPlexRequested()
        }
    }

    // === Recently Added (header + dynamic filter chips) ===
    RowLayout {
        visible: root._hasRecent
        Layout.fillWidth: true
        spacing: Units.spacingXL

        Text {
            text: "Recently Added"
            font.pixelSize: Theme.fontTitle
            font.bold: true
            color: Theme.textPrimary
            Layout.alignment: Qt.AlignVCenter
        }

        FilterChips {
            id: chips
            visible: root._chipsVisible
            Layout.alignment: Qt.AlignVCenter
            options: root._chipOptions
            currentIndex: {
                for (var i = 0; i < root._chipOptions.length; i++) {
                    if (root._chipOptions[i].value === root._recentFilter)
                        return i;
                }
                return 0;
            }
            previousRow: root._hasOnDeck ? onDeckRow : root.previousRow
            nextRow: recentRow
            onFilterChanged: value => root._recentFilter = value
            onEscaped: root.escaped()
        }

        Item {
            Layout.fillWidth: true
        }
    }

    NavigableRow {
        id: recentRow
        visible: root._filteredRecent.length > 0
        Layout.fillWidth: true
        Layout.preferredHeight: root.plexRowHeight
        keyNavigationWraps: true
        previousRow: root._chipsVisible ? chips : (root._hasOnDeck ? onDeckRow : root.previousRow)
        nextRow: root.nextRow
        model: root._filteredRecent
        onActiveFocusChanged: if (activeFocus)
            root.ensureVisibleRequested(this)
        onActivated: root.openPlexRequested()
        onEscaped: root.escaped()

        delegate: PlexCard {
            required property int index
            required property var modelData
            posterWidth: root.posterW
            posterHeight: root.posterH
            title: modelData.title || ""
            subtitle: modelData.subtitle || ""
            art: modelData.art || ""
            progress: modelData.progress || 0
            focus: index === recentRow.currentIndex
            onActivated: root.openPlexRequested()
        }
    }
}
