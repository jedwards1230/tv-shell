import QtQuick
import QtQuick.Layouts
import "lib"

// Home-screen Plex widget (#249) — ONE poster row with a segmented header that
// flips between "Up Next" (continue-watching / On Deck) and "Recently Added"
// (new arrivals), fed by the daemon's `plex-hubs` IPC. Apple-TV-style: a single
// prominent row, not two stacked rows. The segment control appears only when
// BOTH segments have content (otherwise the lone segment's name is just a
// header). `size` reformats the row (not a scale):
//   small  = poster-only rail (caption band removed) — glanceable
//   medium = posters + title/subtitle captions + resume bars (default)
// (A "large" featured-backdrop hero is a planned follow-up — it needs 16:9
// backdrop art the daemon doesn't return yet.)
//
// Health-aware: a `ServiceMonitor` keyed on "plex" collapses the widget when
// unconfigured/empty and shows a graceful `ServiceStatusNotice` when the server
// is down.
//
// Focus contract (host uses these): `firstRow`/`lastRow` resolve to the first/
// last *visible* internal region (segment chips, poster row); the internal chain
// lets NavigableRow/FilterChips skip a hidden region.
ColumnLayout {
    id: root

    property Item previousRow: null
    property Item nextRow: null
    property bool widgetEnabled: true
    // "small" | "medium" (large = future hero).
    property string size: "medium"

    signal escaped
    signal openPlexRequested
    signal ensureVisibleRequested(var item)

    spacing: Units.spacingMD

    // === Data (populated from `plex-hubs`) ===
    property var onDeckItems: []
    property var recentItems: []

    readonly property bool _hasOnDeck: onDeckItems.length > 0
    readonly property bool _hasRecent: recentItems.length > 0

    // === Segment (Up Next vs Recently Added) ===
    property string _segment: "ondeck"
    readonly property var _segmentOptions: {
        let o = [];
        if (_hasOnDeck)
            o.push({
                "label": "Up Next",
                "value": "ondeck"
            });
        if (_hasRecent)
            o.push({
                "label": "Recently Added",
                "value": "recent"
            });
        return o;
    }
    // Show the toggle only when there's a genuine choice (both segments present).
    readonly property bool _showSegmentControl: _segmentOptions.length > 1
    readonly property string _segmentName: _segment === "ondeck" ? "Up Next" : "Recently Added"
    readonly property var _activeItems: _segment === "ondeck" ? onDeckItems : recentItems

    readonly property bool rowFocused: posterRow.activeFocus || segmentChips.activeFocus

    visible: root.widgetEnabled && (plexMon.degraded || (plexMon.ok && (root._hasOnDeck || root._hasRecent)))

    // === Home-tile focus contract ===
    readonly property var firstRow: _showSegmentControl ? segmentChips : posterRow
    readonly property var lastRow: posterRow
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

    // === Poster geometry (reflow by size) ===
    readonly property real _posterScale: root.size === "small" ? 0.50 : 0.62
    readonly property bool _showCaption: root.size !== "small"
    readonly property int posterW: Math.round(Theme.cardWidth * _posterScale)
    readonly property int posterH: Math.round(posterW * 1.5)
    readonly property int _captionBand: Math.round(Theme.fontSmall * 1.4 + Theme.fontCaption * 1.4 + Units.spacingSM * 2)
    readonly property int plexRowHeight: posterH + (_showCaption ? _captionBand : 0)

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
            // Keep the active segment on something that has content.
            if (root._segment === "ondeck" && !root._hasOnDeck && root._hasRecent)
                root._segment = "recent";
            else if (root._segment === "recent" && !root._hasRecent && root._hasOnDeck)
                root._segment = "ondeck";
        }
    }

    ServiceStatusNotice {
        Layout.fillWidth: true
        serviceName: "Plex"
        status: plexMon.status
    }

    // === Header: segment control (or single-segment label) ===
    RowLayout {
        Layout.fillWidth: true
        visible: root._hasOnDeck || root._hasRecent
        spacing: Units.spacingXL

        // Single-segment header label (when only one segment has content).
        Text {
            visible: !root._showSegmentControl
            text: root._segmentName
            font.pixelSize: Theme.fontTitle
            font.bold: true
            color: Theme.textPrimary
            Layout.alignment: Qt.AlignVCenter
        }

        // Segment toggle (Up Next | Recently Added) when both have content.
        FilterChips {
            id: segmentChips
            visible: root._showSegmentControl
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
        Layout.preferredHeight: root.plexRowHeight
        keyNavigationWraps: true
        previousRow: root._showSegmentControl ? segmentChips : root.previousRow
        nextRow: root.nextRow
        model: root._activeItems
        onActiveFocusChanged: if (activeFocus)
            root.ensureVisibleRequested(this)
        onActivated: root.openPlexRequested()
        onEscaped: root.escaped()

        delegate: PlexCard {
            required property int index
            required property var modelData
            posterWidth: root.posterW
            posterHeight: root.posterH
            showCaption: root._showCaption
            title: modelData.title || ""
            subtitle: modelData.subtitle || ""
            art: modelData.art || ""
            progress: modelData.progress || 0
            focus: index === posterRow.currentIndex
            onActivated: root.openPlexRequested()
        }
    }
}
