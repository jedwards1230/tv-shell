pragma Singleton
import QtQuick
import "../../components"
import "../recent"

// Hand-written home-widget registry (#249 Phase 2/3). The single, ordered source
// of truth for which standardized widgets the home screen renders. NOT codegen —
// the repo forbids QML build tooling; adding a widget is a one-line edit to
// `_entries` plus a `Component` below.
//
// Each entry pairs a stable `id` with the `component` to instantiate and its
// persisted config, sourced from the namespaced `widgets.<id>.*` subtree via
// `SettingsStore.widget(id)` (enabled / size / order). WidgetHost reads `widgets`
// to instantiate the set and build the focus chain.
//
// Reactivity notes:
//   - Each entry is a long-lived QtObject whose `enabled` / `size` / `order` track
//     SettingsStore.widget(id), so toggling one widget updates only that entry's
//     properties — the `widgets` array identity stays stable and WidgetHost never
//     tears down and rebuilds every widget on an unrelated config change.
//   - `order` is declared `int` so QML suppresses no-op change notifications: a
//     change to `enabled` reassigns store.widgets → each entry's `order` recomputes
//     to the SAME int → no orderChanged → the sorted `widgets` binding (which reads
//     only `order`) does NOT re-run → WidgetHost's Repeater does not rebuild. The
//     sorted `widgets` rebuilds ONLY when an actual order changes (the reorder UI).
Item {
    id: registry

    // Fixed entry set (registration order); `widgets` exposes them sorted by order.
    readonly property var _entries: [moonlightEntry, nowPlayingEntry, plexEntry, recentEntry]

    // Ordered widget set — a STABLE sort of _entries by each entry's `order`
    // (registration order breaks ties). Re-evaluates only when an `order` changes.
    readonly property var widgets: {
        var indexed = [];
        for (var i = 0; i < registry._entries.length; i++)
            indexed.push({
                "e": registry._entries[i],
                "i": i
            });
        indexed.sort(function (a, b) {
            if (a.e.order !== b.e.order)
                return a.e.order - b.e.order;
            return a.i - b.i;
        });
        var out = [];
        for (var k = 0; k < indexed.length; k++)
            out.push(indexed[k].e);
        return out;
    }

    // Order-independent lookup over the fixed entry set.
    function entryById(widgetId) {
        for (var i = 0; i < registry._entries.length; i++) {
            if (registry._entries[i].widgetId === widgetId)
                return registry._entries[i];
        }
        return null;
    }

    QtObject {
        id: moonlightEntry
        readonly property string widgetId: "moonlight"
        readonly property Component component: Component {
            MoonlightWidget {}
        }
        readonly property bool enabled: SettingsStore.widget("moonlight").enabled
        readonly property string size: SettingsStore.widget("moonlight").size
        readonly property int order: SettingsStore.widget("moonlight").order
    }

    QtObject {
        id: nowPlayingEntry
        readonly property string widgetId: "nowplaying"
        readonly property Component component: Component {
            NowPlayingWidget {}
        }
        readonly property bool enabled: SettingsStore.widget("nowplaying").enabled
        readonly property string size: SettingsStore.widget("nowplaying").size
        readonly property int order: SettingsStore.widget("nowplaying").order
    }

    QtObject {
        id: plexEntry
        readonly property string widgetId: "plex"
        readonly property Component component: Component {
            PlexWidget {}
        }
        readonly property bool enabled: SettingsStore.widget("plex").enabled
        readonly property string size: SettingsStore.widget("plex").size
        readonly property int order: SettingsStore.widget("plex").order
    }

    QtObject {
        id: recentEntry
        readonly property string widgetId: "recent"
        readonly property Component component: Component {
            RecentWidget {}
        }
        readonly property bool enabled: SettingsStore.widget("recent").enabled
        readonly property string size: SettingsStore.widget("recent").size
        readonly property int order: SettingsStore.widget("recent").order
    }
}
