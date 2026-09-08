// The home screen, wired. A real HomeScreen over a real FocusRouter, with real
// Cards in real Rails — the pure lanes prove the arithmetic, and this proves the
// glue actually calls it.
//
// The case that matters most here is the one v1 kept getting wrong: focus must
// never be stranded. In v1 a widget going away took focus with it, because the
// neighbour that pointed at it was hand-wired and nobody updated it. Here rails
// appear and vanish as apps start and stop, and every assertion below about
// `currentId` after such a change is an assertion that nothing had to be
// rewired.
//
// docs/V2_SHELL.md records which mutation each assertion caught.
import QtQuick
import QtTest
import TvShell
import "qrc:/qt/qml/TvShell/homeModel.js" as HomeModel

Item {
    id: harness

    width: 1920
    height: 1080

    readonly property int shellId: 9001

    // Drives the model: which apps the core says are running right now.
    property var running: []

    readonly property var catalog: [
        {
            "appId": 9003,
            "title": "Moonlight",
            "subtitle": "",
            "accent": ""
        },
        {
            "appId": 9004,
            "title": "Steam",
            "subtitle": "",
            "accent": ""
        },
        {
            "appId": 9005,
            "title": "Plex",
            "subtitle": "",
            "accent": ""
        }
    ]
    // Set to [] to exercise the empty screen.
    property var catalogInUse: harness.catalog

    readonly property var model: HomeModel.build(harness.catalogInUse, {
        "focusable_apps": harness.running
    }, harness.shellId)

    // NOT `id: router` — FocusSlot has a property of that name and it shadows an
    // outer id of the same name inside every delegate.
    FocusRouter {
        id: theRouter
    }

    property int unplaceableCount: 0

    Connections {
        target: theRouter
        function onFocusUnplaceable() {
            harness.unplaceableCount++;
        }
    }

    property var lastActivated: null

    HomeScreen {
        id: home

        anchors.fill: parent
        model: harness.model
        router: theRouter
        onActivated: entry => harness.lastActivated = entry
    }

    TestCase {
        name: "HomeScreen"
        when: windowShown

        function init() {
            harness.catalogInUse = harness.catalog;
            harness.running = [];
            harness.unplaceableCount = 0;
            harness.lastActivated = null;
            // Re-place focus deterministically: each test starts from the model
            // it just set, not from wherever the previous one left the router.
            theRouter.focusInitial();
            wait(0);
        }

        // --- the graph the screen builds -------------------------------------

        function test_focus_starts_on_the_first_card_of_the_first_rail() {
            compare(theRouter.currentId, "apps:9003");
        }

        function test_right_and_left_walk_a_rail_and_stop_at_its_ends() {
            compare(theRouter.move("right"), true);
            compare(theRouter.currentId, "apps:9004");
            compare(theRouter.move("right"), true);
            compare(theRouter.currentId, "apps:9005");
            // The end of the rail is a stop, not a wrap: B is the way out of a
            // corner, and a wrap would make the last card feel like the first.
            compare(theRouter.move("right"), false);
            compare(theRouter.currentId, "apps:9005");

            compare(theRouter.move("left"), true);
            compare(theRouter.move("left"), true);
            compare(theRouter.currentId, "apps:9003");
            compare(theRouter.move("left"), false);
        }

        function test_up_and_down_cross_rails() {
            harness.running = [9004];
            wait(0);
            // Continue is now rail 0 and Apps rail 1; focus was on Apps.
            theRouter.setCurrent("apps:9003");
            compare(theRouter.move("up"), true);
            compare(theRouter.currentId, "continue:9004");
            compare(theRouter.move("up"), false); // nothing above the top rail
            compare(theRouter.move("down"), true);
            compare(theRouter.currentId, "apps:9003");
        }

        // Down from a long rail into a short one lands on the nearest column
        // rather than nowhere. Rails have different lengths all the time —
        // Continue usually has one card and Apps has ten.
        function test_moving_into_a_shorter_rail_lands_on_the_nearest_column() {
            harness.running = [9005];
            wait(0);
            theRouter.setCurrent("apps:9005"); // column 2 of the Apps rail
            compare(theRouter.move("up"), true);
            // The Continue rail has a single card, at column 0.
            compare(theRouter.currentId, "continue:9005");
        }

        // --- the stranding guarantee ------------------------------------------

        // A rail vanishing under the focused card must re-home, not strand. This
        // is exactly the v1 bug class: the card is destroyed, its slot
        // unregisters, and NOTHING was wired to it that needs updating.
        function test_a_rail_vanishing_under_focus_rehomes_instead_of_stranding() {
            harness.running = [9004];
            wait(0);
            theRouter.setCurrent("continue:9004");
            compare(theRouter.currentId, "continue:9004");

            // The app exits: the core stops listing it, so the Continue rail is
            // no longer in the model at all.
            harness.running = [];
            // R6: judged once the rebuild has settled. Asserting before the turn
            // ends would be asserting about a state the screen never renders.
            wait(0);

            verify(theRouter.currentId !== "");
            verify(theRouter.currentId.indexOf("apps:") === 0);
            compare(harness.unplaceableCount, 0);
        }

        // And the focused card is a REAL focus holder afterwards, not just a
        // string in the router: a re-home that does not move Qt's activeFocus
        // leaves the pad talking to nothing.
        function test_the_rehomed_card_actually_holds_active_focus() {
            harness.running = [9004];
            wait(0);
            theRouter.setCurrent("continue:9004");
            harness.running = [];
            wait(0);
            verify(home.activeFocus);
            const focused = harness.findSlot(theRouter.currentId);
            verify(focused !== null);
            verify(focused.activeFocus);
        }

        // The whole screen emptying is the only case that may leave focus
        // unplaced — and even then the screen puts a focusable empty-state cell
        // up, so in practice it does not happen. This asserts the practice.
        function test_an_empty_screen_still_has_somewhere_to_focus() {
            harness.catalogInUse = [];
            harness.running = [];
            compare(harness.model.empty, true);
            wait(0);
            compare(theRouter.currentId, "empty");
            compare(harness.unplaceableCount, 0);
            // And it is a genuine stop, not a cell you can walk off.
            compare(theRouter.move("left"), false);
            compare(theRouter.move("down"), false);
            compare(theRouter.currentId, "empty");
        }

        // Coming back from empty must return focus to real content rather than
        // leaving it on a hidden cell.
        function test_leaving_the_empty_state_restores_focus_to_a_card() {
            harness.catalogInUse = [];
            wait(0);
            compare(theRouter.currentId, "empty");
            harness.catalogInUse = harness.catalog;
            wait(0);
            verify(theRouter.currentId.indexOf("apps:") === 0);
            const empty = harness.findSlot("empty");
            // The empty cell is still instantiated, but hidden — and a hidden
            // slot is not focusable, which is what keeps it out of the graph
            // without anyone remembering to disable it.
            verify(empty === null || !empty.visible);
        }

        // --- activation --------------------------------------------------------

        function test_activating_a_card_reports_the_entry_the_model_built() {
            harness.running = [9004];
            wait(0);
            theRouter.setCurrent("continue:9004");
            wait(0);
            const card = harness.findSlot("continue:9004");
            verify(card !== null);
            keyClick(Qt.Key_Return);
            verify(harness.lastActivated !== null);
            compare(harness.lastActivated.appId, 9004);
            // A running app is shown, not launched — and the command is built in
            // exactly one place.
            compare(harness.lastActivated.action, "show");
            compare(HomeModel.commandFor(harness.lastActivated), "show 9004");
        }

        function test_activating_a_stopped_app_launches_it() {
            theRouter.setCurrent("apps:9005");
            wait(0);
            keyClick(Qt.Key_Return);
            verify(harness.lastActivated !== null);
            compare(HomeModel.commandFor(harness.lastActivated), "launch 9005");
        }

        // --- ids ----------------------------------------------------------------

        // The same app appears in Continue and in Apps at once, routinely. If a
        // card keyed on the app id alone, those two would collide and the
        // router's neighbour lookup would be ambiguous.
        function test_the_same_app_in_two_rails_has_two_distinct_cells() {
            harness.running = [9004];
            wait(0);
            verify(harness.findSlot("continue:9004") !== null);
            verify(harness.findSlot("apps:9004") !== null);
            verify(harness.findSlot("continue:9004") !== harness.findSlot("apps:9004"));
        }
    }

    // Find a registered slot by id, or null. Reaches through the router rather
    // than through the item tree, so it sees exactly the set the model sees.
    function findSlot(id) {
        const slots = theRouter.slots;
        for (let i = 0; i < slots.length; ++i) {
            if (slots[i] && slots[i].slotId === id)
                return slots[i];
        }
        return null;
    }
}
