import QtQuick
import "lib/focusChain.js" as FocusChain

// Vertical wrapping grid that implements the same duck-typed home-tile focus
// contract as NavigableRow (regionFocused / canFocus / focusFirstChild +
// previousRow/nextRow), but lays out its cells in a wrapping Grid instead of one
// horizontal rail. It GROWS to its content height (no internal scrolling — the
// home screen's outer Flickable scrolls it; an internal GridView would create a
// nested-scroll focus trap), so its implicit height is the laid-out Grid's height.
//
// Key model (mirrors NavigableRow, extended to two dimensions):
//   • Left/Right step currentIndex by ±1 (clamped at the ends — no wrap, so you
//     can't jump rows by over-running a row edge).
//   • Up/Down step by ±columns. Up off the top row, or Down off the bottom row,
//     hands focus to previousRow/nextRow by walking that chain exactly like
//     NavigableRow (skipping hidden/!canFocus neighbours). A Down from a SHORT
//     last row first clamps to the final cell before it will hand off.
//   • Return/Enter → activated (the focused AppCard delegate also self-activates
//     on Return/click, so callers usually wire the card's onActivated); X →
//     contextRequested; Escape/B → escaped.
//
// `columns` is derived from the available width and the cell footprint, so the
// grid reflows as the home column resizes. The delegate renders its own focus by
// reading `focus: index === <grid>.currentIndex` (same as NavigableRow), and the
// FocusScope routes activeFocus to that delegate; unhandled keys bubble up here.
FocusScope {
    id: root

    property Item previousRow: null
    property Item nextRow: null
    property alias model: rep.model
    property alias delegate: rep.delegate
    property alias count: rep.count

    // Cell footprint — set by the host so `columns` can be computed from width.
    // The delegate is expected to size itself to (cellWidth × cellHeight).
    property real cellWidth: Theme.cardWidth
    property real cellHeight: Theme.cardHeight
    property real spacing: Theme.cardSpacing

    // The focused cell. Clamped to a valid index whenever the model shrinks.
    property int currentIndex: 0
    readonly property Item currentItem: (currentIndex >= 0 && currentIndex < rep.count) ? rep.itemAt(currentIndex) : null

    signal activated
    signal escaped
    signal contextRequested
    // Emitted (deferred) when the focused cell changes / the grid gains focus,
    // passing the focused delegate so the outer Flickable scrolls IT into view.
    signal ensureVisibleRequested(var item)

    // Columns that fit the available width (≥1). The last column needs no trailing
    // spacing, hence the +spacing in the numerator (an exact fit, not conservative).
    readonly property int columns: Math.max(1, Math.floor((root.width + root.spacing) / (root.cellWidth + root.spacing)))
    readonly property int rows: Math.ceil(rep.count / Math.max(1, root.columns))

    // === Home-tile focus contract ===
    readonly property bool regionFocused: activeFocus
    readonly property bool canFocus: visible && rep.count > 0

    function focusFirstChild() {
        if (!visible || rep.count === 0)
            return false;
        root.currentIndex = 0;
        root.forceActiveFocus();
        return true;
    }

    implicitWidth: grid.implicitWidth
    implicitHeight: grid.implicitHeight

    // Keep currentIndex valid as the model grows/shrinks (a stale index would
    // leave no delegate carrying focus:true, stranding the highlight).
    onCountChanged: {
        if (root.currentIndex >= rep.count)
            root.currentIndex = Math.max(0, rep.count - 1);
    }

    onCurrentIndexChanged: if (activeFocus && root.currentItem)
        Qt.callLater(() => root.ensureVisibleRequested(root.currentItem))

    onActiveFocusChanged: if (activeFocus && root.currentItem)
        Qt.callLater(() => root.ensureVisibleRequested(root.currentItem))

    Keys.onPressed: event => {
        var n = rep.count;
        if (n === 0)
            return;
        switch (event.key) {
        case Qt.Key_Left:
            InputMode.exitMouseMode();
            if (root.currentIndex > 0)
                root.currentIndex--;
            event.accepted = true;
            break;
        case Qt.Key_Right:
            InputMode.exitMouseMode();
            if (root.currentIndex < n - 1)
                root.currentIndex++;
            event.accepted = true;
            break;
        case Qt.Key_Up:
            InputMode.exitMouseMode();
            {
                var up = root.currentIndex - root.columns;
                if (up >= 0) {
                    root.currentIndex = up;
                    event.accepted = true;
                } else {
                    // top row → hand off UP the chain; accept only if it moved focus
                    // (a failed hand-off bubbles, matching NavigableRow / WakeCard).
                    event.accepted = FocusChain.navigateUp(root);
                }
            }
            break;
        case Qt.Key_Down:
            InputMode.exitMouseMode();
            {
                var down = root.currentIndex + root.columns;
                if (down < n) {
                    root.currentIndex = down;
                    event.accepted = true;
                } else {
                    // Past the last full step. If we're NOT already on the bottom
                    // row, clamp to the final cell (a short last row); only when
                    // already on the bottom row do we hand off DOWN the chain.
                    var onBottomRow = root.currentIndex >= (root.rows - 1) * root.columns;
                    if (!onBottomRow && root.currentIndex !== n - 1) {
                        root.currentIndex = n - 1;
                        event.accepted = true;
                    } else {
                        // hand off DOWN the chain; accept only if it moved focus
                        // (a failed hand-off bubbles, matching NavigableRow / WakeCard).
                        event.accepted = FocusChain.navigateDown(root);
                    }
                }
            }
            break;
        case Qt.Key_Return:
        case Qt.Key_Enter:
            // The focused AppCard delegate self-activates on Return (BaseCard.Keys),
            // so this only fires for delegates that don't — kept for parity.
            InputMode.exitMouseMode();
            if (root.currentItem)
                root.activated();
            event.accepted = true;
            break;
        case Qt.Key_X:
            InputMode.exitMouseMode();
            if (root.currentItem) {
                root.contextRequested();
                event.accepted = true;
            }
            break;
        case Qt.Key_Escape:
        case Qt.Key_B:
            if (event.key === Qt.Key_B && event.modifiers)
                break;
            InputMode.exitMouseMode();
            root.escaped();
            event.accepted = true;
            break;
        }
    }

    Grid {
        id: grid
        columns: root.columns
        columnSpacing: root.spacing
        rowSpacing: root.spacing

        Repeater {
            id: rep
        }
    }
}
