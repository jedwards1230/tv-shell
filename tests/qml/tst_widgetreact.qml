import QtQuick
import QtTest
import components.lib
import "../../shell/components/lib/widgetConfig.js" as WidgetConfig

// Regression guard for namespaced widget-config reactivity (#249 Phase 3, #281).
//
// The reviewer on #281 flagged that toggling `hideFromRecent` on the Widgets page
// wouldn't reach HomeScreen's Recent row because the catalog reads
// `SettingsStore.widget(id).prefs.hideFromRecent`. That live-update path is in
// fact reactive — but ONLY because `SettingsStore.setWidget*` REASSIGN
// `store.widgets` to a NEW object (firing `widgetsChanged`) instead of mutating
// the nested config in place. This test locks that invariant in two layers:
//
//   1. The REAL `widgetConfig.js` mutators (setWidget/setPref/setOrder, which
//      SettingsStore now delegates to) return a NEW object and never touch the
//      input — the precondition for the reassignment to notify.
//   2. A QML binding that reads a `widget(id)` accessor recomputes after a
//      reassignment driven by those real mutators (hideFromRecent → Recent model,
//      plus registry-style enabled/size/order). A negative control proves an
//      in-place mutation does NOT propagate, so the assertions have teeth.
//
// widgetConfig.js is imported by its real source path (zero drift); the store
// fixture below is a thin stand-in for SettingsStore.qml (which can't load
// headless — it imports Quickshell.Io), whose setters delegate to the SAME real
// mutators, so the mutation path under test is production code.
TestCase {
    id: tc
    name: "WidgetReact"
    when: windowShown

    readonly property var manifests: WidgetManifests.manifests
    function _defaults() {
        return WidgetConfig.defaultSubtree(tc.manifests);
    }
    function _fresh() {
        return WidgetConfig.migrate({}, tc.manifests).widgets;
    }

    // --- SettingsStore stand-in: setters delegate to the REAL mutators ---------
    QtObject {
        id: store
        property var widgets: tc._fresh()

        // Faithful read-shaper mirroring SettingsStore.widget(id) (its resolution
        // logic is the migrator's _fillWidget, covered by tst_widgetmigrate).
        function widget(id) {
            var base = WidgetConfig.defaultSubtree(tc.manifests)[id] || {
                "enabled": true,
                "order": 0,
                "size": "medium",
                "prefs": {}
            };
            var w = (store.widgets && store.widgets[id]) ? store.widgets[id] : null;
            if (!w)
                return base;
            var out = {
                "enabled": (typeof w.enabled === "boolean") ? w.enabled : base.enabled,
                "order": (typeof w.order === "number") ? w.order : base.order,
                "size": (typeof w.size === "string") ? w.size : base.size,
                "prefs": {}
            };
            for (var bk in base.prefs)
                out.prefs[bk] = base.prefs[bk];
            if (w.prefs && typeof w.prefs === "object") {
                for (var pk in w.prefs)
                    out.prefs[pk] = w.prefs[pk];
            }
            return out;
        }
        function setWidget(id, key, value) {
            store.widgets = WidgetConfig.setWidget(store.widgets, id, key, value, tc._defaults());
        }
        function setWidgetPref(id, prefKey, value) {
            store.widgets = WidgetConfig.setPref(store.widgets, id, prefKey, value, tc._defaults());
        }
        function setWidgetOrder(ids) {
            store.widgets = WidgetConfig.setOrder(store.widgets, ids, tc._defaults());
        }
    }

    // --- HomeScreen-style consumers reading store.widget(id) -------------------
    readonly property var _mockRunning: [
        {
            "name": "Spotify",
            "running": true
        }
    ]

    Item {
        id: home

        // Mirror of HomeScreen._widgets (catalog with closures).
        readonly property var _widgets: [
            {
                "id": "nowplaying",
                "hideFromRecent": {
                    "capable": true,
                    "enabled": store.widget("nowplaying").prefs.hideFromRecent,
                    "matches": function (e) {
                        return e.name === "Spotify";
                    }
                }
            }
        ]

        // Mirror of HomeScreen._recentModel's suppression filter.
        readonly property var _recentModel: {
            let result = tc._mockRunning.slice();
            let widgets = home._widgets;
            for (let w = 0; w < widgets.length; w++) {
                let h = widgets[w].hideFromRecent;
                if (h && h.capable && h.enabled) {
                    let matches = h.matches;
                    result = result.filter(function (e) {
                        return !matches(e);
                    });
                }
            }
            return result;
        }

        // Production pushes _recentModel into the Recent widget via a Binding
        // element — replicate that indirection.
        property var recentMirror: []
        Binding {
            target: home
            property: "recentMirror"
            value: home._recentModel
        }

        // Registry-style (WidgetRegistry) enabled/size/order bindings.
        readonly property bool npEnabled: store.widget("nowplaying").enabled
        readonly property string npSize: store.widget("nowplaying").size
        readonly property int npOrder: store.widget("nowplaying").order
    }

    // Reset shared fixture state before each test (QtTest runs tests in order).
    function init() {
        store.widgets = tc._fresh();
    }

    // === Layer 1: real widgetConfig.js mutators are immutable ==================
    function test_setPref_returns_new_object_without_mutating_input() {
        var before = tc._fresh();
        var snapshot = JSON.stringify(before);
        var after = WidgetConfig.setPref(before, "nowplaying", "hideFromRecent", false, tc._defaults());
        verify(after !== before, "setPref must return a NEW object (enables reassignment notify)");
        compare(JSON.stringify(before), snapshot, "input must be left untouched (no in-place mutation)");
        compare(after.nowplaying.prefs.hideFromRecent, false, "new object carries the set value");
    }

    function test_setWidget_returns_new_object_without_mutating_input() {
        var before = tc._fresh();
        var snapshot = JSON.stringify(before);
        var after = WidgetConfig.setWidget(before, "nowplaying", "enabled", false, tc._defaults());
        verify(after !== before, "setWidget must return a NEW object");
        compare(JSON.stringify(before), snapshot, "input untouched");
        compare(after.nowplaying.enabled, false);
        compare(after.nowplaying.size, before.nowplaying.size, "unrelated keys preserved");
    }

    function test_setOrder_returns_new_object_without_mutating_input() {
        var before = tc._fresh();
        var snapshot = JSON.stringify(before);
        var after = WidgetConfig.setOrder(before, ["plex", "nowplaying", "moonlight", "recent"], tc._defaults());
        verify(after !== before, "setOrder must return a NEW object");
        compare(JSON.stringify(before), snapshot, "input untouched");
        compare(after.plex.order, 0);
        compare(after.nowplaying.order, 1);
    }

    // === Layer 2: the QML binding chain recomputes on reassignment =============
    function test_hideFromRecent_live_update() {
        compare(home._recentModel.length, 0, "Spotify suppressed while hideFromRecent on");
        compare(home.recentMirror.length, 0, "Binding mirror reflects suppression");

        store.setWidgetPref("nowplaying", "hideFromRecent", false);

        compare(home._recentModel.length, 1, "Recent model recomputes → Spotify reappears");
        compare(home.recentMirror.length, 1, "Binding-element consumer reflects the change live");
    }

    function test_enabled_live_update() {
        compare(home.npEnabled, true);
        store.setWidget("nowplaying", "enabled", false);
        compare(home.npEnabled, false, "registry-style enabled binding recomputes");
    }

    function test_size_live_update() {
        store.setWidget("nowplaying", "size", "small");
        compare(home.npSize, "small", "registry-style size binding recomputes");
    }

    function test_order_live_update() {
        store.setWidgetOrder(["plex", "nowplaying", "moonlight", "recent"]);
        compare(home.npOrder, 1, "registry-style order binding recomputes");
    }

    // === Negative control: in-place mutation must NOT propagate (teeth) ========
    function test_inplace_mutation_is_not_reactive() {
        compare(home._recentModel.length, 0, "baseline: suppressed");
        // Mutate the nested config WITHOUT reassigning store.widgets — the broken
        // pattern. The downstream model must NOT see it, proving the live-update
        // tests above pass because of the reassignment, not by accident.
        store.widgets["nowplaying"].prefs.hideFromRecent = false;
        compare(home._recentModel.length, 0, "in-place mutation does not fire widgetsChanged → no recompute");
    }
}
