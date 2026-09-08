// homeModel.js, headless. Pins H1..H8 — the rules that make the home screen a
// pure function of the core's snapshot.
//
// docs/V2_SHELL.md records which mutation of homeModel.js each assertion caught.
import QtQuick
import QtTest
import "qrc:/qt/qml/TvShell/homeModel.js" as HomeModel

Item {
    id: harness

    readonly property int shellId: 9001

    // A three-app catalog. No id collides with the shell's own.
    function catalog() {
        return [
            {
                "appId": 9003,
                "title": "Moonlight",
                "subtitle": "Stream",
                "accent": ""
            },
            {
                "appId": 9004,
                "title": "Steam",
                "subtitle": "",
                "accent": "#e06236"
            },
            {
                "appId": 9005,
                "title": "Plex",
                "subtitle": "",
                "accent": ""
            }
        ];
    }

    // A `screen-state` reply shaped the way core/src/screen.rs serializes one.
    function snapshot(focusable, onScreen) {
        const s = {
            "focused_window": 1,
            "base_layer": [],
            "base_layer_windows": [],
            "focusable_windows": [],
            "focusable_apps": focusable,
            "display": {},
            "xwayland_server_id": 0,
            "focused_app_atom_diagnostic": null
        };
        if (onScreen !== null)
            s.on_screen = {
                "window": 1,
                "app_id": onScreen,
                "source": "focusable"
            };
        return s;
    }

    function railById(model, id) {
        for (let i = 0; i < model.rails.length; ++i) {
            if (model.rails[i].id === id)
                return model.rails[i];
        }
        return null;
    }

    TestCase {
        name: "HomeModel"

        // --- H1 / H5: running comes from the core, and decides the action ----

        function test_running_apps_come_from_focusable_apps() {
            const m = HomeModel.build(harness.catalog(), harness.snapshot([9003], 9003), harness.shellId);
            const cont = harness.railById(m, "continue");
            verify(cont !== null);
            compare(cont.entries.length, 1);
            compare(cont.entries[0].appId, 9003);
            compare(cont.entries[0].running, true);
            compare(cont.entries[0].action, "show");
        }

        function test_a_catalog_app_that_is_not_running_launches() {
            const m = HomeModel.build(harness.catalog(), harness.snapshot([], null), harness.shellId);
            const apps = harness.railById(m, "apps");
            compare(apps.entries.length, 3);
            for (let i = 0; i < apps.entries.length; ++i) {
                compare(apps.entries[i].running, false);
                compare(apps.entries[i].action, "launch");
            }
        }

        // A launch we just made does NOT make an app running — only the core's
        // list does. This is the rule that stops the shell holding an opinion
        // the compositor disagrees with.
        function test_the_action_follows_the_snapshot_and_nothing_else() {
            const before = HomeModel.build(harness.catalog(), harness.snapshot([], null), harness.shellId);
            compare(harness.railById(before, "apps").entries[0].action, "launch");
            const after = HomeModel.build(harness.catalog(), harness.snapshot([9003], 9003), harness.shellId);
            compare(harness.railById(after, "apps").entries[0].action, "show");
        }

        function test_command_for_is_the_only_place_a_verb_is_built() {
            compare(HomeModel.commandFor({
                "action": "show",
                "appId": 9003
            }), "show 9003");
            compare(HomeModel.commandFor({
                "action": "launch",
                "appId": 9004
            }), "launch 9004");
            // A malformed entry yields "", which the caller treats as "send
            // nothing" — never a half-built command line.
            compare(HomeModel.commandFor(null), "");
            compare(HomeModel.commandFor({
                "action": "show"
            }), "");
        }

        // --- H2: a running app with no catalog row still appears -------------

        function test_an_unknown_running_app_is_shown_not_hidden() {
            const m = HomeModel.build(harness.catalog(), harness.snapshot([424242], 424242), harness.shellId);
            const cont = harness.railById(m, "continue");
            compare(cont.entries.length, 1);
            compare(cont.entries[0].appId, 424242);
            compare(cont.entries[0].title, "App 424242");
            compare(cont.entries[0].action, "show");
        }

        // --- H3: the shell never offers itself -------------------------------

        function test_the_shell_app_id_never_appears() {
            // In the core's running list...
            const m = HomeModel.build(harness.catalog(), harness.snapshot([harness.shellId, 9003], harness.shellId), harness.shellId);
            const cont = harness.railById(m, "continue");
            compare(cont.entries.length, 1);
            compare(cont.entries[0].appId, 9003);

            // ...and in the catalog, where a misconfiguration could also put it.
            const withShell = harness.catalog();
            withShell.push({
                "appId": harness.shellId,
                "title": "tv-shell",
                "subtitle": "",
                "accent": ""
            });
            const m2 = HomeModel.build(withShell, harness.snapshot([], null), harness.shellId);
            compare(harness.railById(m2, "apps").entries.length, 3);
        }

        // --- H4: the Apps rail does not reshuffle ----------------------------

        function test_apps_rail_holds_the_whole_catalog_in_order_running_or_not() {
            const m = HomeModel.build(harness.catalog(), harness.snapshot([9004], 9004), harness.shellId);
            const apps = harness.railById(m, "apps");
            compare(apps.entries.length, 3);
            compare(apps.entries[0].appId, 9003);
            compare(apps.entries[1].appId, 9004);
            compare(apps.entries[2].appId, 9005);
            // The running one is marked, not moved.
            compare(apps.entries[1].running, true);
        }

        // --- H6: no snapshot is a usable screen ------------------------------

        function test_a_missing_snapshot_leaves_apps_intact() {
            const snapshots = [null, undefined, "not an object",
                {}
            ];
            for (let i = 0; i < snapshots.length; ++i) {
                const m = HomeModel.build(harness.catalog(), snapshots[i], harness.shellId);
                compare(harness.railById(m, "continue"), null, "continue for case " + i);
                compare(harness.railById(m, "apps").entries.length, 3, "apps for case " + i);
                compare(m.empty, false);
            }
        }

        function test_a_snapshot_with_a_junk_focusable_list_is_survived() {
            const s = harness.snapshot([9003, "nine", null, 9004], null);
            const m = HomeModel.build(harness.catalog(), s, harness.shellId);
            const cont = harness.railById(m, "continue");
            compare(cont.entries.length, 2);
            compare(cont.entries[0].appId, 9003);
            compare(cont.entries[1].appId, 9004);
        }

        // The on-screen id comes from `on_screen`, NEVER from the atom the core
        // labels diagnostic — that one reads empty under an input-focus overlay,
        // so trusting it would blank the header every time the drawer opens.
        function test_on_screen_id_ignores_the_diagnostic_atom() {
            const s = harness.snapshot([9003], 9003);
            s.focused_app_atom_diagnostic = 424242;
            compare(HomeModel.onScreenId(s), 9003);

            const noOnScreen = harness.snapshot([9003], null);
            noOnScreen.focused_app_atom_diagnostic = 9003;
            compare(HomeModel.onScreenId(noOnScreen), null);
            compare(HomeModel.onScreenId(null), null);
        }

        // --- H7 / H8: empty rails, and the never-strand guarantee ------------

        function test_an_empty_rail_is_omitted() {
            const m = HomeModel.build(harness.catalog(), harness.snapshot([], null), harness.shellId);
            compare(m.rails.length, 1);
            compare(m.rails[0].id, "apps");
        }

        function test_everything_empty_reports_empty() {
            const m = HomeModel.build([], harness.snapshot([], null), harness.shellId);
            compare(m.rails.length, 0);
            compare(m.empty, true);
            // Still an array, never null: the screen iterates it unconditionally.
            compare(typeof m.rails, "object");
        }

        function test_a_shell_only_snapshot_is_still_empty() {
            // The shell is always focusable, so without H3 this case would look
            // non-empty and the empty-state cell — the only focusable thing on
            // the screen — would never render.
            const m = HomeModel.build([], harness.snapshot([harness.shellId], harness.shellId), harness.shellId);
            compare(m.empty, true);
        }
    }
}
