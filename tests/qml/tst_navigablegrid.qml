import QtQuick
import QtTest
import components

// Headless tests for NavigableGrid's wrapping-grid key navigation + the up/down
// hand-off to previousRow/nextRow (#249 Apps widget). The real NavigableGrid runs
// against the stub Theme/InputMode singletons. With a 400-wide grid of 100-wide
// cells and 10px spacing → 3 columns; a 7-item model lays out as:
//   row0=[0,1,2]  row1=[3,4,5]  row2=[6]   (a short last row)
// which exercises every branch: clamp-at-ends, ±columns row steps, the short-row
// clamp before hand-off, and the top-row UP / bottom-row DOWN chain hand-off.
TestCase {
    id: testCase
    name: "NavigableGrid"
    when: windowShown
    visible: true
    width: 500
    height: 500

    // Rig: a NavigableGrid wired to two focus sentinels (up/down) that record when
    // the chain hands focus off to them. A trivial Rectangle delegate carries
    // `focus: index === grid.currentIndex` (the same idiom AppCard uses).
    Component {
        id: rigComp
        Item {
            width: 500
            height: 500
            property alias grid: g
            property bool upFocused: false
            property bool downFocused: false

            FocusScope {
                id: up
                width: 10
                height: 10
                property bool canFocus: true
                onActiveFocusChanged: if (activeFocus)
                    up.parent.upFocused = true
            }
            FocusScope {
                id: down
                width: 10
                height: 10
                y: 20
                property bool canFocus: true
                onActiveFocusChanged: if (activeFocus)
                    down.parent.downFocused = true
            }
            NavigableGrid {
                id: g
                y: 40
                width: 400
                previousRow: up
                nextRow: down
                cellWidth: 100
                cellHeight: 60
                spacing: 10
                model: 7
                delegate: Rectangle {
                    required property int index
                    width: 100
                    height: 60
                    focus: index === g.currentIndex
                    color: focus ? "#dd3333" : "#333333"
                }
            }
        }
    }

    function newRig() {
        var rig = createTemporaryObject(rigComp, testCase);
        verify(rig, "rig instantiated");
        verify(rig.grid, "grid instantiated");
        return rig;
    }

    // --- Geometry: columns + rows derived from width/cell footprint ---------
    function test_columns_and_rows() {
        var rig = newRig();
        compare(rig.grid.columns, 3, "400w / (100+10) cells → 3 columns");
        compare(rig.grid.rows, 3, "7 items / 3 cols → 3 rows");
        compare(rig.grid.count, 7);
    }

    // --- focusFirstChild lands on cell 0 and takes focus -------------------
    function test_focus_first_child() {
        var rig = newRig();
        verify(rig.grid.focusFirstChild(), "focusFirstChild succeeded");
        compare(rig.grid.currentIndex, 0);
        verify(rig.grid.regionFocused, "grid holds focus");
    }

    // --- Left/Right step ±1 and clamp at the ends (no row wrap) ------------
    function test_left_right_clamp() {
        var rig = newRig();
        rig.grid.focusFirstChild();
        keyClick(Qt.Key_Left);
        compare(rig.grid.currentIndex, 0, "Left clamps at start");
        keyClick(Qt.Key_Right);
        compare(rig.grid.currentIndex, 1);
        keyClick(Qt.Key_Right);
        compare(rig.grid.currentIndex, 2);
        // Stepping right past a row edge advances by one index (does NOT wrap rows).
        keyClick(Qt.Key_Right);
        compare(rig.grid.currentIndex, 3, "Right crosses into the next row by index");
    }

    // --- Down/Up step ±columns within the grid ----------------------------
    function test_down_up_rows() {
        var rig = newRig();
        rig.grid.focusFirstChild();
        keyClick(Qt.Key_Down);
        compare(rig.grid.currentIndex, 3, "Down from 0 → +columns");
        keyClick(Qt.Key_Down);
        compare(rig.grid.currentIndex, 6, "Down from 3 → +columns (last cell)");
        keyClick(Qt.Key_Up);
        compare(rig.grid.currentIndex, 3, "Up → -columns");
        keyClick(Qt.Key_Up);
        compare(rig.grid.currentIndex, 0, "Up → top row");
    }

    // --- Down from a middle cell with a SHORT last row clamps to the end ---
    function test_down_short_last_row_clamps() {
        var rig = newRig();
        rig.grid.currentIndex = 4;     // row1 middle; +columns would overshoot 7
        rig.grid.forceActiveFocus();
        keyClick(Qt.Key_Down);
        compare(rig.grid.currentIndex, 6, "clamps to the final cell, not a hand-off");
        verify(!rig.downFocused, "no DOWN hand-off from a non-bottom row");
    }

    // --- Up off the top row hands focus UP the chain ----------------------
    function test_up_handoff() {
        var rig = newRig();
        rig.grid.focusFirstChild();    // cell 0, top row
        keyClick(Qt.Key_Up);
        verify(rig.upFocused, "Up off the top row focuses previousRow");
    }

    // --- Down off the bottom row hands focus DOWN the chain ----------------
    function test_down_handoff() {
        var rig = newRig();
        rig.grid.currentIndex = 6;     // the only bottom-row cell
        rig.grid.forceActiveFocus();
        keyClick(Qt.Key_Down);
        verify(rig.downFocused, "Down off the bottom row focuses nextRow");
    }

    // --- currentIndex clamps when the model shrinks below it ---------------
    function test_count_change_clamps_index() {
        var rig = newRig();
        rig.grid.currentIndex = 6;
        rig.grid.model = 3;            // indices 0..2 now
        compare(rig.grid.currentIndex, 2, "index clamps to the new last cell");
    }
}
