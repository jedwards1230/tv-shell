import QtQuick
import "lib"

// Non-visual provider for the Plex `plex-hubs` data + health (#249). Extracted
// from PlexWidget so the redesigned home screen can read ONE source from two
// rails — On Deck feeds the unified Continue rail, Recently Added feeds the New
// rail — instead of rendering the old standalone two-row widget. Mirrors
// PlexWidget's ServiceMonitor wiring exactly (same healthKey/dataCommand), so
// the daemon still owns the URL+token and bakes tokenized art URLs into the
// reply; this provider never sees a credential.
//
// Item (not QtObject) because it hosts a ServiceMonitor (Timer/socket children)
// — same constraint as Theme/SettingsStore. Zero-sized: it renders nothing.
Item {
    id: root
    width: 0
    height: 0
    visible: false

    property var onDeckItems: []
    property var recentItems: []

    readonly property bool ok: plexMon.ok
    readonly property bool degraded: plexMon.degraded
    readonly property string status: plexMon.status

    readonly property bool hasOnDeck: onDeckItems.length > 0
    readonly property bool hasRecent: recentItems.length > 0

    function refresh() {
        plexMon.refresh();
    }

    // Items are cleared whenever Plex isn't `ok` so a transient outage hides the
    // rails (and surfaces the degraded notice) rather than showing stale posters.
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
}
