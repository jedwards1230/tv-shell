// The one binding that opens the drawer from a real key.
//
// gamescope does not deliver KEY_MENU to its clients (measured on htpc-1,
// 2026-09-10 — jedwards1230/tv-shell#489), so the Menu-only binding this
// replaced could not be reached by any input device on the couch. Tab can, and
// this lane is where that stays true.
import QtQuick
import QtTest
import TvShell

Item {
    id: harness

    Component {
        id: shellComponent

        Main {}
    }

    TestCase {
        name: "MainDrawerKeys"
        when: windowShown

        function test_tab_opens_and_closes_the_drawer() {
            const shell = shellComponent.createObject(harness);
            verify(shell);
            verify(!shell.drawerOpen);

            shell.base.requestActivate();
            tryCompare(shell.base, "active", true);
            keyClick(Qt.Key_Tab);
            tryCompare(shell, "drawerOpen", true);

            shell.drawer.requestActivate();
            tryCompare(shell.drawer, "active", true);
            keyClick(Qt.Key_Tab);
            tryCompare(shell, "drawerOpen", false);

            shell.destroy();
        }
    }
}
