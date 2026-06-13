import QtQuick
import QtQuick.Layouts

// Home-screen Plex widget (#plex-widget): two controller-navigable poster rows —
// "On Deck" (continue-watching / up-next) and "Recently Added" — fed by the
// daemon's `plex-hubs` IPC. The daemon owns the Plex server URL + token (from
// its env) and bakes ready-to-load tokenized art URLs into the reply, so this
// widget never sees a credential. It mirrors MediaWidget's role: it sits in the
// home-screen vertical focus chain via previousRow/nextRow and collapses to zero
// height when Plex is unconfigured or both hubs are empty.
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

    signal escaped
    // A card was activated — the host opens the Plex app.
    signal openPlexRequested
    // A sub-row took focus — the host scrolls it into view (the home Flickable
    // doesn't auto-follow focus).
    signal ensureVisibleRequested(var item)

    spacing: Units.spacingMD

    // === Data (populated from `plex-hubs`) ===
    property bool _enabled: false
    property var onDeckItems: []
    property var recentItems: []

    readonly property bool _hasOnDeck: onDeckItems.length > 0
    readonly property bool _hasRecent: recentItems.length > 0

    // Collapse entirely when disabled or empty — keeps the home layout unchanged
    // when there's nothing to show (same contract as a missing player / target).
    visible: root._enabled && (root._hasOnDeck || root._hasRecent)

    // First/last *visible* sub-row, for the host's neighbour wiring.
    readonly property var firstRow: _hasOnDeck ? onDeckRow : recentRow
    readonly property var lastRow: _hasRecent ? recentRow : onDeckRow

    // === Poster geometry (shared by every card so rows align) ===
    readonly property int posterW: Math.round(Theme.cardWidth * 0.62)
    readonly property int posterH: Math.round(posterW * 1.5)
    readonly property int plexRowHeight: posterH + Math.round(Theme.fontSmall * 1.4 + Theme.fontCaption * 1.4 + Units.spacingSM * 2)

    function refresh() {
        hubsClient.request("plex-hubs");
    }

    SocketClient {
        id: hubsClient
        onResponseReceived: line => {
            try {
                let data = JSON.parse(line);
                root._enabled = data.enabled === true;
                root.onDeckItems = data.onDeck || [];
                root.recentItems = data.recentlyAdded || [];
            } catch (e) {
                // Malformed reply — leave the last good data in place.
                console.log("PlexWidget: bad plex-hubs reply: " + e);
            }
        }
        // Daemon down / not yet up: keep whatever we have; the timer retries.
        onRequestFailed: {}
    }

    // Poll on start, then refresh periodically so a newly-added title or an
    // advanced watch position shows up without a restart.
    Timer {
        interval: 60000
        running: true
        repeat: true
        triggeredOnStart: true
        onTriggered: root.refresh()
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
