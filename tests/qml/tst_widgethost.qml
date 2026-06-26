import QtQuick
import QtTest
import components.lib

// Headless tests for WidgetHost's generic vertical focus chain (#249 Phase 2).
// The real WidgetHost is exercised against a test-only WidgetRegistry stub
// (3 widgets: single-stop A, multi-row B, single-stop C) with writable `enabled`
// flags. These lock the invariant the refactor replaced hand-wired neighbours
// with: each widget's UP/DOWN neighbour resolves to the nearest preceding/
// following FOCUSABLE widget's exit/entry (lastRow/firstRow, or the widget itself
// when single-stop), falling back to the host's top/bottom anchors — and it
// reroutes when a widget is disabled.
TestCase {
    id: testCase
    name: "WidgetHost"
    when: windowShown
    visible: true
    width: 400
    height: 400

    // A self-contained rig: the two terminal anchors + a WidgetHost wired to them.
    Component {
        id: rigComp
        Item {
            property alias host: h
            property alias topA: ta
            property alias bottomA: ba
            Item {
                id: ta
            }
            // The bottom anchor needs a previousRow slot (the real one is a
            // NavigableRow); WidgetHost binds it to the last focusable exit.
            Item {
                id: ba
                property Item previousRow: null
            }
            WidgetHost {
                id: h
                topAnchor: ta
                bottomAnchor: ba
            }
        }
    }

    // Reset the shared singleton registry before each test.
    function init() {
        for (var i = 0; i < WidgetRegistry.widgets.length; i++)
            WidgetRegistry.widgets[i].enabled = true;
    }

    function newRig() {
        var rig = createTemporaryObject(rigComp, testCase);
        verify(rig, "rig instantiated");
        verify(rig.host, "host instantiated");
        return rig;
    }

    // --- Order + id map ----------------------------------------------------
    function test_regions_order_and_ids() {
        var rig = newRig();
        compare(rig.host.regions.length, 3);
        verify(rig.host.widgetById("a"), "widget a present");
        verify(rig.host.widgetById("b"), "widget b present");
        verify(rig.host.widgetById("c"), "widget c present");
        compare(rig.host.widgetById("nope"), null);
    }

    // --- Full chain, all widgets enabled -----------------------------------
    function test_chain_all_enabled() {
        var rig = newRig();
        var a = rig.host.widgetById("a");
        var b = rig.host.widgetById("b"); // multi-row (firstRow/lastRow)
        var c = rig.host.widgetById("c");

        // First widget's UP neighbour is the top anchor.
        compare(a.previousRow, rig.topA);
        // DOWN into a multi-row widget lands on its firstRow (entry).
        compare(a.nextRow, b.firstRow);

        // Single-stop A is targeted directly going UP from B.
        compare(b.previousRow, a);
        compare(b.nextRow, c);

        // UP from C lands on B's lastRow (exit).
        compare(c.previousRow, b.lastRow);
        // Last widget's DOWN neighbour is the bottom anchor.
        compare(c.nextRow, rig.bottomA);

        // Bottom anchor's UP neighbour is the last focusable widget (C, single-stop).
        compare(rig.bottomA.previousRow, c);
    }

    // --- Disabling the middle widget reroutes its neighbours ---------------
    function test_disable_middle_reroutes() {
        var rig = newRig();
        var a = rig.host.widgetById("a");
        var c = rig.host.widgetById("c");

        WidgetRegistry.entryById("b").enabled = false;

        // B is skipped: A now points straight at C and vice-versa.
        compare(a.nextRow, c);
        compare(c.previousRow, a);
        // Bottom anchor still resolves to C.
        compare(rig.bottomA.previousRow, c);
    }

    // --- Disabling the last widget moves the bottom-anchor exit up ---------
    function test_disable_last_moves_bottom_exit() {
        var rig = newRig();
        var a = rig.host.widgetById("a");
        var b = rig.host.widgetById("b");

        WidgetRegistry.entryById("c").enabled = false;

        // A still enters B going down; B now exits to the bottom anchor.
        compare(a.nextRow, b.firstRow);
        compare(b.nextRow, rig.bottomA);
        // Bottom anchor's UP neighbour is now B's exit row.
        compare(rig.bottomA.previousRow, b.lastRow);
    }

    // --- No focusable widgets: everything collapses to the anchors ---------
    function test_disable_all_falls_to_anchors() {
        var rig = newRig();
        var a = rig.host.widgetById("a");
        var c = rig.host.widgetById("c");

        WidgetRegistry.entryById("a").enabled = false;
        WidgetRegistry.entryById("b").enabled = false;
        WidgetRegistry.entryById("c").enabled = false;

        // With nothing focusable, the first widget's UP is the top anchor and
        // the last widget's DOWN is the bottom anchor.
        compare(a.previousRow, rig.topA);
        compare(c.nextRow, rig.bottomA);
        // Bottom anchor falls back to the top anchor.
        compare(rig.bottomA.previousRow, rig.topA);
    }

    // --- Host focus helpers ------------------------------------------------
    function test_focus_first_visible_skips_disabled() {
        var rig = newRig();
        WidgetRegistry.entryById("a").enabled = false;
        // A disabled → focus walk should land on B (the first focusable).
        verify(rig.host.focusFirstVisible(), "focused a widget");
        verify(rig.host.widgetById("b").regionFocused, "B holds focus");
        verify(rig.host.regionFocused, "host reports a focused region");
    }

    function test_focus_first_visible_none() {
        var rig = newRig();
        WidgetRegistry.entryById("a").enabled = false;
        WidgetRegistry.entryById("b").enabled = false;
        WidgetRegistry.entryById("c").enabled = false;
        compare(rig.host.focusFirstVisible(), false);
        compare(rig.host.regionFocused, false);
    }
}
