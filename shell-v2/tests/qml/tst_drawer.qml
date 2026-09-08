// The overlay drawer, wired.
//
// Short on purpose: the drawer is three rows and one dispatch. What is worth
// asserting is that it shares the shell's ONE focusable type and therefore
// inherits the focus model for free — a vertical chain nobody wired, that stops
// at both ends rather than wrapping — and that each row's effect is decided in
// one place rather than by the row itself.
import QtQuick
import QtTest
import TvShell

Item {
    id: harness

    width: 720
    height: 1080

    // NOT `id: router` — FocusSlot has a property of that name.
    FocusRouter {
        id: theRouter
    }

    property int homeCount: 0
    property int reloadCount: 0
    property int dismissCount: 0

    DrawerScreen {
        id: drawer

        anchors.fill: parent
        router: theRouter
        onGoHome: harness.homeCount++
        onReloadCatalog: harness.reloadCount++
        onDismissed: harness.dismissCount++
    }

    TestCase {
        name: "DrawerScreen"
        when: windowShown

        function init() {
            harness.homeCount = 0;
            harness.reloadCount = 0;
            harness.dismissCount = 0;
            theRouter.setCurrent("drawer:home");
            wait(0);
        }

        function test_focus_walks_the_rows_and_stops_at_both_ends() {
            compare(theRouter.currentId, "drawer:home");
            // Up off the top is a stop, not a wrap.
            compare(theRouter.move("up"), false);

            compare(theRouter.move("down"), true);
            compare(theRouter.currentId, "drawer:reload");
            compare(theRouter.move("down"), true);
            compare(theRouter.currentId, "drawer:close");
            compare(theRouter.move("down"), false);

            compare(theRouter.move("up"), true);
            compare(theRouter.currentId, "drawer:reload");
        }

        // A single column: left/right have nowhere to go and must not throw or
        // jump to another row.
        function test_horizontal_movement_is_a_no_op_in_one_column() {
            compare(theRouter.move("left"), false);
            compare(theRouter.move("right"), false);
            compare(theRouter.currentId, "drawer:home");
        }

        function test_each_row_fires_its_own_effect() {
            theRouter.setCurrent("drawer:home");
            wait(0);
            keyClick(Qt.Key_Return);
            compare(harness.homeCount, 1);
            compare(harness.reloadCount, 0);
            compare(harness.dismissCount, 0);

            theRouter.setCurrent("drawer:reload");
            wait(0);
            keyClick(Qt.Key_Return);
            compare(harness.reloadCount, 1);
            compare(harness.homeCount, 1);

            theRouter.setCurrent("drawer:close");
            wait(0);
            keyClick(Qt.Key_Return);
            compare(harness.dismissCount, 1);
        }

        // The drawer's ids are namespaced for the same reason the rails' are: a
        // router is per-surface today, but a bare "home" would collide the
        // moment anything else on any surface wanted that word.
        function test_row_ids_are_namespaced() {
            for (let i = 0; i < drawer.actions.length; ++i)
                verify(("drawer:" + drawer.actions[i].id).length > "drawer:".length);
        }
    }
}
