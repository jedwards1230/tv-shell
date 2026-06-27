import QtQuick
import QtTest
import components.lib
import "../../shell/widgets/lib/widgetConfig.js" as WidgetConfig

// Headless tests for the namespaced widget-config migrator (#249 Phase 3).
// widgetConfig.js is a pure `.pragma library` module imported by its real source
// path (zero drift); WidgetManifests is the real pure-data manifest singleton
// (copied into the assembled module by run.sh). These lock the three migration
// invariants SettingsStore depends on: fresh-install defaults, legacy flat-key
// preservation (esp. the widgetSpotify* → "nowplaying" remap), and idempotency.
TestCase {
    id: testCase
    name: "WidgetMigrate"

    readonly property var manifests: WidgetManifests.manifests

    // --- Fresh install: no flat keys, no widgets subtree -------------------
    function test_fresh_install_defaults() {
        var r = WidgetConfig.migrate({}, testCase.manifests);
        compare(r.changed, true, "absent subtree → changed");

        var w = r.widgets;
        // All four widgets present, enabled, ordered 0..3, sizes = manifest default.
        compare(w.moonlight.enabled, true);
        compare(w.nowplaying.enabled, true);
        compare(w.plex.enabled, true);
        compare(w.recent.enabled, true);

        compare(w.moonlight.order, 0);
        compare(w.nowplaying.order, 1);
        compare(w.plex.order, 2);
        compare(w.recent.order, 3);

        compare(w.moonlight.size, "medium");
        compare(w.nowplaying.size, "medium");
        compare(w.plex.size, "medium");
        compare(w.recent.size, "medium");

        // hideFromRecent pref defaults true for the shadowing widgets.
        compare(w.nowplaying.prefs.hideFromRecent, true);
        compare(w.plex.prefs.hideFromRecent, true);
        // moonlight/recent have no prefs.
        compare(Object.keys(w.moonlight.prefs).length, 0);
        compare(Object.keys(w.recent.prefs).length, 0);
    }

    // --- Legacy flat keys preserved (incl. widgetSpotify* → nowplaying) ----
    function test_legacy_preservation() {
        var legacy = {
            "widgetMoonlightEnabled": true,
            "widgetMoonlightSize": "large",
            "widgetSpotifyEnabled": false,
            "widgetSpotifyHideFromRecent": false,
            "widgetPlexSize": "small"
        };
        var r = WidgetConfig.migrate(legacy, testCase.manifests);
        compare(r.changed, true, "absent subtree → changed");
        var w = r.widgets;

        // Moonlight large size carried over.
        compare(w.moonlight.size, "large");
        compare(w.moonlight.enabled, true);

        // The KEY assertion: widgetSpotify* maps to id "nowplaying".
        compare(w.nowplaying.enabled, false);
        compare(w.nowplaying.prefs.hideFromRecent, false);

        // Plex size overlaid; its enabled + hideFromRecent fall back to defaults.
        compare(w.plex.size, "small");
        compare(w.plex.enabled, true);
        compare(w.plex.prefs.hideFromRecent, true);

        // Recent untouched → manifest defaults.
        compare(w.recent.enabled, true);
        compare(w.recent.size, "medium");
        compare(w.recent.order, 3);
    }

    // --- Idempotency: an existing subtree is preserved, not re-defaulted ---
    function test_idempotent_existing_subtree() {
        var existing = {
            "widgets": {
                "moonlight": {
                    "enabled": false,
                    "order": 2,
                    "size": "large",
                    "prefs": {}
                },
                "nowplaying": {
                    "enabled": true,
                    "order": 0,
                    "size": "small",
                    "prefs": {
                        "hideFromRecent": false
                    }
                },
                "plex": {
                    "enabled": false,
                    "order": 1,
                    "size": "small",
                    "prefs": {
                        "hideFromRecent": true
                    }
                },
                "recent": {
                    "enabled": true,
                    "order": 3,
                    "size": "medium",
                    "prefs": {}
                }
            }
        };
        var r = WidgetConfig.migrate(existing, testCase.manifests);
        compare(r.changed, false, "present subtree → not changed");
        var w = r.widgets;

        // Existing values preserved, not overwritten by defaults.
        compare(w.moonlight.enabled, false);
        compare(w.moonlight.order, 2);
        compare(w.moonlight.size, "large");
        compare(w.nowplaying.size, "small");
        compare(w.nowplaying.prefs.hideFromRecent, false);
        compare(w.plex.enabled, false);
        compare(w.plex.order, 1);
    }

    // --- Partial subtree: missing per-widget keys filled from defaults -----
    function test_partial_subtree_filled() {
        var partial = {
            "widgets": {
                "moonlight": {
                    "enabled": false
                }
            }
        };
        var r = WidgetConfig.migrate(partial, testCase.manifests);
        compare(r.changed, false, "present subtree → not changed");
        var w = r.widgets;

        // Preserved the one value, filled the rest from defaults.
        compare(w.moonlight.enabled, false);
        compare(w.moonlight.order, 0);
        compare(w.moonlight.size, "medium");
        // The other widgets are materialised from defaults so accessors never miss.
        compare(w.nowplaying.enabled, true);
        compare(w.plex.prefs.hideFromRecent, true);
        compare(w.recent.order, 3);
    }
}
