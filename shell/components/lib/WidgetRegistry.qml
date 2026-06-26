pragma Singleton
import QtQuick
import "../"

// Hand-written home-widget registry (#249 Phase 2). The single, ordered source of
// truth for which standardized widgets the home screen renders and in what order.
// NOT codegen — the repo forbids QML build tooling; adding or reordering a widget
// is a one-line edit to `widgets` plus a `Component` entry below.
//
// Each entry pairs a stable `id` with the `component` to instantiate and its
// persisted `enabled` / `size` config, sourced from the flat `Theme.widget*`
// settings (the namespaced `widgets.<id>.*` config is a later phase). WidgetHost
// reads this list to instantiate the set and build the focus chain.
//
// Reactivity note: each entry is a long-lived QtObject whose `enabled` / `size`
// track Theme, so toggling one widget updates only that entry's properties — the
// `widgets` array identity stays stable and WidgetHost never tears down and
// rebuilds every widget on an unrelated config change.
Item {
    id: registry

    // Ordered widget set: moonlight → nowplaying → plex → recent.
    readonly property var widgets: [moonlightEntry, nowPlayingEntry, plexEntry, recentEntry]

    function entryById(widgetId) {
        for (var i = 0; i < registry.widgets.length; i++) {
            if (registry.widgets[i].widgetId === widgetId)
                return registry.widgets[i];
        }
        return null;
    }

    QtObject {
        id: moonlightEntry
        readonly property string widgetId: "moonlight"
        readonly property Component component: Component {
            MoonlightWidget {}
        }
        readonly property bool enabled: Theme.widgetMoonlightEnabled
        readonly property string size: Theme.widgetMoonlightSize
    }

    QtObject {
        id: nowPlayingEntry
        readonly property string widgetId: "nowplaying"
        readonly property Component component: Component {
            NowPlayingWidget {}
        }
        readonly property bool enabled: Theme.widgetSpotifyEnabled
        readonly property string size: Theme.widgetSpotifySize
    }

    QtObject {
        id: plexEntry
        readonly property string widgetId: "plex"
        readonly property Component component: Component {
            PlexWidget {}
        }
        readonly property bool enabled: Theme.widgetPlexEnabled
        readonly property string size: Theme.widgetPlexSize
    }

    QtObject {
        id: recentEntry
        readonly property string widgetId: "recent"
        readonly property Component component: Component {
            RecentWidget {}
        }
        readonly property bool enabled: Theme.widgetRecentEnabled
        readonly property string size: Theme.widgetRecentSize
    }
}
