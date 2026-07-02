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

    // --- Inherited chain traversal (shared focusChain.js via Widget base) -----
    // A bare column of single-stop StubWidgets wired previous/next, with the middle
    // one NOT focusable (widgetEnabled=false). The base's inherited _navigateUp/
    // _navigateDown must skip it and land on the far neighbour — with ZERO nav code
    // in StubWidget itself (it only extends Widget).
    Component {
        id: chainComp
        Item {
            property alias w1: sw1
            property alias w2: sw2
            property alias w3: sw3
            StubWidget {
                id: sw1
                nextRow: sw2
            }
            StubWidget {
                id: sw2
                widgetEnabled: false  // → !visible → !canFocus → skipped by the walk
                previousRow: sw1
                nextRow: sw3
            }
            StubWidget {
                id: sw3
                previousRow: sw2
            }
        }
    }

    function test_widget_traversal_skips_non_focusable() {
        var chain = createTemporaryObject(chainComp, testCase);
        verify(chain, "chain instantiated");

        // Up from w3: w2 is not focusable → skip to w1.
        chain.w3.forceActiveFocus();
        verify(chain.w3.activeFocus, "w3 holds focus");
        verify(chain.w3._navigateUp(), "navigateUp found a focusable neighbour");
        verify(chain.w1.regionFocused, "Up skipped the disabled middle, landed on w1");

        // Down from w1: w2 is not focusable → skip to w3.
        verify(chain.w1._navigateDown(), "navigateDown found a focusable neighbour");
        verify(chain.w3.regionFocused, "Down skipped the disabled middle, landed on w3");
    }

    function test_widget_traversal_noop_when_no_neighbour() {
        var chain = createTemporaryObject(chainComp, testCase);
        // w1 has no previousRow; Up is a no-op (returns false, focus unchanged).
        chain.w1.forceActiveFocus();
        compare(chain.w1._navigateUp(), false);
        verify(chain.w1.regionFocused, "focus stays on w1 when Up finds nothing");
    }

    // --- Generic signals forwarded up through the host (wired ONCE) -----------
    // Every widget inherits escaped + ensureVisibleRequested from the Widget base;
    // WidgetHost re-emits them as host-level widgetEscaped / widgetEnsureVisible-
    // Requested so HomeScreen connects them a single time.
    function test_host_forwards_widget_signals() {
        var rig = newRig();
        var b = rig.host.widgetById("b");
        verify(b, "widget b present");

        var escapedCount = 0;
        var ensureItem = null;
        rig.host.widgetEscaped.connect(function () {
            escapedCount++;
        });
        rig.host.widgetEnsureVisibleRequested.connect(function (item) {
            ensureItem = item;
        });

        b.escaped();
        compare(escapedCount, 1, "widget escaped re-emitted as host widgetEscaped");

        b.ensureVisibleRequested(b);
        compare(ensureItem, b, "ensureVisibleRequested forwarded with its item arg");
    }

    // --- Single-stop base auto-emits ensureVisibleRequested on focus entry -----
    // A single-stop widget (firstRow unset) has no internal row to emit on entry,
    // so the base emits it — reaching HomeScreen's scroll via the host forward.
    function test_single_stop_autoemits_ensure_visible_on_focus() {
        var rig = newRig();
        var a = rig.host.widgetById("a"); // StubWidget = single-stop (firstRow null)
        compare(a.firstRow, null, "single-stop widget has no firstRow");

        var ensureItem = null;
        rig.host.widgetEnsureVisibleRequested.connect(function (item) {
            ensureItem = item;
        });
        a.focusFirstChild();
        verify(a.activeFocus, "single-stop widget took focus");
        compare(ensureItem, a, "base auto-emitted ensureVisibleRequested(self) on focus entry");
    }
}
