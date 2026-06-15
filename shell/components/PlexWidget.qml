import QtQuick
import QtQuick.Layouts
import "lib"

// Home-screen Plex widget (#plex-widget): two controller-navigable poster rows —
// "On Deck" (continue-watching / up-next) and "Recently Added" — fed by the
// daemon's `plex-hubs` IPC. The daemon owns the Plex server URL + token (from
// its env) and bakes ready-to-load tokenized art URLs into the reply, so this
// widget never sees a credential. It mirrors MediaWidget's role: it sits in the
// home-screen vertical focus chain via previousRow/nextRow.
//
// Health-aware (service-health bus): a `ServiceMonitor` keyed on "plex" tracks
// reachability from the daemon's health events, so the widget can tell three
// states apart instead of one: unconfigured ⇒ collapse to zero height; reachable
// with items ⇒ show the poster rows; reachable-but-empty ⇒ collapse; configured
// but the server is down (`unreachable`/`error`) ⇒ show a graceful
// `ServiceStatusNotice` rather than silently vanishing.
//
// Focus wiring contract (the host uses these): the two sub-rows are the real
// focus targets. `firstRow`/`lastRow` resolve to the first/last *visible* row so
// the host can point its neighbours at the right edge of the widget; the
// internal chain (onDeckRow <-> recentRow) lets NavigableRow skip a hidden row.
ColumnLayout {
    id: root

    // Outer vertical-chain neighbours (set by the host).
    property Item previousRow: null
    property Item nextRow: null

    // Home-screen widget toggle (Settings ▸ Widgets). When false the widget is
    // hidden and collapses to zero height.
    property bool widgetEnabled: true

    // True while either poster row holds focus. The host's focus safety-net
    // (_reanchorFocusIfNeeded) checks this so it doesn't yank focus back out of
    // the Plex rows (they aren't NavigableRows it knows about directly).
    readonly property bool rowFocused: onDeckRow.activeFocus || recentRow.activeFocus

    signal escaped
    // A card was activated — the host opens the Plex app.
    signal openPlexRequested
    // A sub-row took focus — the host scrolls it into view (the home Flickable
    // doesn't auto-follow focus).
    signal ensureVisibleRequested(var item)

    spacing: Units.spacingMD

    // === Data (populated from `plex-hubs`) ===
    property var onDeckItems: []
    property var recentItems: []

    readonly property bool _hasOnDeck: onDeckItems.length > 0
    readonly property bool _hasRecent: recentItems.length > 0

    // Show when configured: poster rows while healthy + non-empty, or the
    // degraded notice while the server is down. Collapse to zero height when the
    // widget is toggled off, Plex is unconfigured (`disabled`), or it's healthy
    // but has nothing to show — keeping the home layout unchanged.
    visible: root.widgetEnabled && (plexMon.degraded || (plexMon.ok && (root._hasOnDeck || root._hasRecent)))

    // First/last *visible* sub-row, for the host's neighbour wiring.
    readonly property var firstRow: _hasOnDeck ? onDeckRow : recentRow
    readonly property var lastRow: _hasRecent ? recentRow : onDeckRow

    // === Home-tile focus contract (mirrors NavigableRow) ===
    // `canFocus` is the load-bearing distinction the host needs: the widget can
    // be `visible` yet hold NO focusable row — the degraded "server down" state
    // shows only the (non-focusable) ServiceStatusNotice. Neighbours/helpers must
    // treat that as "skip me", or pressing B would strand focus on an invisible
    // poster row and the stick goes dead.
    readonly property bool canFocus: visible && (root._hasOnDeck || root._hasRecent)
    readonly property bool regionFocused: rowFocused

    function focusFirstChild() {
        if (!root.canFocus)
            return false;
        root.firstRow.forceActiveFocus();
        return true;
    }

    // === Poster geometry (shared by every card so rows align) ===
    readonly property int posterW: Math.round(Theme.cardWidth * 0.62)
    readonly property int posterH: Math.round(posterW * 1.5)
    readonly property int plexRowHeight: posterH + Math.round(Theme.fontSmall * 1.4 + Theme.fontCaption * 1.4 + Units.spacingSM * 2)

    function refresh() {
        plexMon.refresh();
    }

    // Health + data source. `healthKey: "plex"` adopts the daemon's broadcast
    // health status; `dataCommand: "plex-hubs"` fetches the hubs (primed on
    // start, refreshed every 60s while healthy, re-fetched on recovery). Items
    // are cleared whenever Plex isn't `ok` so a transient outage hides the rows
    // and surfaces the notice rather than showing stale posters.
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

    // Graceful "server down" placeholder — visible only for unreachable/error.
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
        nextRow: recentRow
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

    // === Recently Added ===
    Text {
        visible: root._hasRecent
        text: "Recently Added"
        font.pixelSize: Theme.fontTitle
        font.bold: true
        color: Theme.textPrimary
    }

    NavigableRow {
        id: recentRow
        visible: root._hasRecent
        Layout.fillWidth: true
        Layout.preferredHeight: root.plexRowHeight
        keyNavigationWraps: true
        previousRow: onDeckRow
        nextRow: root.nextRow
        model: root.recentItems
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
