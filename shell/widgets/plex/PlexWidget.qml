import QtQuick
import QtQuick.Layouts
import "../lib"
import "../../components"
import "../../components/lib"

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
// Extends Widget (the home-screen widget base): a FocusScope hosting the existing
// ColumnLayout. Focus contract (host uses these): `firstRow`/`lastRow` resolve to
// the first/last *visible* internal region (segment chips, poster row); the
// internal chain lets NavigableRow/FilterChips skip a hidden region.
Widget {
    id: root

    // The base defaults size to ""; Plex defaults to the captioned poster row.
    size: "medium"

    signal openPlexRequested
    signal ensureVisibleRequested(var item)

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
    readonly property string _segmentName: _segment === "ondeck" ? "Up Next" : "Recently Added"
    readonly property var _activeItems: _segment === "ondeck" ? onDeckItems : recentItems

    // Chip strip = the present segments + a trailing "Open Plex" ACTION chip that
    // launches the app directly (no row view of its own; distinct ember focus
    // fill via FilterChips). Sentinel value is ignored by the segment handler.
    readonly property string _openValue: "__open_plex__"
    readonly property var _chipOptions: {
        let o = root._segmentOptions.slice();
        o.push({
            "label": "Open Plex",
            "value": root._openValue,
            "action": true
        });
        return o;
    }

    readonly property bool rowFocused: posterRow.activeFocus || segmentChips.activeFocus

    wantVisible: root.widgetEnabled && (plexMon.degraded || (plexMon.ok && (root._hasOnDeck || root._hasRecent)))

    implicitWidth: col.implicitWidth
    implicitHeight: root.wantVisible ? col.implicitHeight : 0

    // === Home-tile focus contract ===
    firstRow: segmentChips
    lastRow: posterRow
    canFocus: visible && (root._hasOnDeck || root._hasRecent)

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
        dataIntervalMs: 30000  // matches daemon service_health POLL_INTERVAL
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

    ColumnLayout {
        id: col
        width: root.width
        spacing: Units.spacingMD

        ServiceStatusNotice {
            Layout.fillWidth: true
            serviceName: "Plex"
            status: plexMon.status
        }

        // === Header: segment chips + trailing "Open Plex" action chip ===
        RowLayout {
            Layout.fillWidth: true
            visible: root._hasOnDeck || root._hasRecent
            spacing: Units.spacingXL

            FilterChips {
                id: segmentChips
                Layout.alignment: Qt.AlignVCenter
                options: root._chipOptions
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
                onActionTriggered: value => root.openPlexRequested()
                onEscaped: root.escaped()
                // Defer so the Flickable geometry is settled (the widget may have been
                // hidden — Plex down — and just re-revealed) before we scroll to it.
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
            Layout.preferredHeight: root.plexRowHeight
            keyNavigationWraps: true
            previousRow: segmentChips
            nextRow: root.nextRow
            model: root._activeItems
            onActiveFocusChanged: if (activeFocus)
                Qt.callLater(() => root.ensureVisibleRequested(posterRow))
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
}
