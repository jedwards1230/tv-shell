import QtQuick
import QtQuick.Layouts
import "../../components"

// Horizontal chip strip for the home Plex widget (#249). Implements the home-tile
// focus contract (visible / regionFocused / focusFirstChild) and walks
// previousRow/nextRow via forceActiveFocus like NavigableRow, so it slots into
// HomeScreen's _contentRegions() chain and skips hidden neighbours.
//
// Two chip kinds coexist in one strip:
//   • FILTER chips (segments) — Left/Right moves the cursor onto one (highlight
//     only, no side effect); A/Return (or click) COMMITS it (emits
//     filterChanged). The committed filter is `currentIndex` and keeps the
//     crimson "selected" fill while the cursor roams, so you preview before
//     applying instead of the segment flipping live under the d-pad.
//   • ACTION chips (`action: true`) — Left/Right merely focuses them (the active
//     filter is untouched); A/Return fires `actionTriggered(value)`. They never
//     become the "selected" filter and take a distinct EMBER focus fill so they
//     read as a button, not a segment.
// Focus position (`_focusIndex`) is tracked separately from the selected filter
// (`currentIndex`) precisely so an action chip can be focused without dropping
// the active segment.
FocusScope {
    id: root

    property var previousRow: null
    property var nextRow: null

    // [{ label, value, action?: bool }] — value is opaque, handed back via
    // filterChanged (filter chips) or actionTriggered (action chips).
    property var options: []
    // The selected FILTER index (drives the crimson fill). Action chips are
    // skipped by this; callers bind it to the active segment.
    property int currentIndex: 0

    signal filterChanged(var value)
    signal actionTriggered(var value)
    signal escaped

    // Which chip the D-pad is on. Initialised to the selected filter so entering
    // the strip lands on the active segment, not a stale action chip.
    property int _focusIndex: 0

    // True while the D-pad cursor sits on an ACTION chip. Suppresses the
    // selected-segment crimson fill so a focused action chip and the active
    // segment are never both filled at once — only one chip fills at a time.
    readonly property bool _actionFocused: activeFocus && !InputMode.mouseMode && _isAction(_focusIndex)

    // === Home-tile focus contract ===
    readonly property bool regionFocused: activeFocus

    function focusFirstChild() {
        if (!visible)
            return false;
        root._focusIndex = root.currentIndex;
        forceActiveFocus();
        return true;
    }

    implicitWidth: chipRow.implicitWidth
    implicitHeight: chipRow.implicitHeight
    Layout.preferredWidth: chipRow.implicitWidth
    Layout.preferredHeight: chipRow.implicitHeight

    function _isAction(i) {
        return i >= 0 && i < options.length && options[i].action === true;
    }

    // Move the cursor onto chip i — highlight only. Neither a filter nor an
    // action fires here; the committed segment (currentIndex, parent-bound) is
    // untouched until the user presses A (or clicks).
    function _moveTo(i) {
        if (i < 0 || i >= options.length)
            return;
        root._focusIndex = i;
    }

    // A / Return commits the focused chip: an action fires actionTriggered, a
    // filter emits filterChanged. currentIndex stays parent-bound — the caller's
    // filterChanged handler updates the source segment, which re-drives the fill.
    function _activate() {
        if (root._isAction(root._focusIndex))
            root.actionTriggered(options[root._focusIndex].value);
        else
            root.filterChanged(options[root._focusIndex].value);
    }

    Keys.onPressed: event => {
        switch (event.key) {
        case Qt.Key_Left:
            InputMode.exitMouseMode();
            if (root._focusIndex > 0)
                root._moveTo(root._focusIndex - 1);
            event.accepted = true;
            break;
        case Qt.Key_Right:
            InputMode.exitMouseMode();
            if (root._focusIndex < root.options.length - 1)
                root._moveTo(root._focusIndex + 1);
            event.accepted = true;
            break;
        case Qt.Key_Return:
        case Qt.Key_Enter:
            InputMode.exitMouseMode();
            root._activate();
            event.accepted = true;
            break;
        case Qt.Key_Up:
            InputMode.exitMouseMode();
            {
                var up = root.previousRow;
                while (up) {
                    if (up.visible) {
                        up.forceActiveFocus();
                        event.accepted = true;
                        break;
                    }
                    up = (up.previousRow !== undefined) ? up.previousRow : null;
                }
            }
            break;
        case Qt.Key_Down:
            InputMode.exitMouseMode();
            {
                var dn = root.nextRow;
                while (dn) {
                    if (dn.visible) {
                        dn.forceActiveFocus();
                        event.accepted = true;
                        break;
                    }
                    dn = (dn.nextRow !== undefined) ? dn.nextRow : null;
                }
            }
            break;
        case Qt.Key_Escape:
        case Qt.Key_B:
            if (event.key === Qt.Key_B && event.modifiers)
                break;
            root.escaped();
            event.accepted = true;
            break;
        }
    }

    RowLayout {
        id: chipRow
        spacing: Units.spacingMD

        Repeater {
            model: root.options

            delegate: Rectangle {
                id: chip
                required property var modelData
                required property int index
                readonly property bool isAction: modelData.action === true
                // Only filter chips can be the "selected" segment.
                readonly property bool isCurrent: !isAction && index === root.currentIndex
                readonly property bool isFocused: root.activeFocus && !InputMode.mouseMode && index === root._focusIndex
                // Crimson selected fill — hidden while the cursor is on an action
                // chip so it and the focused action chip are never both filled.
                readonly property bool showSelectedFill: isCurrent && !root._actionFocused

                implicitWidth: chipLabel.implicitWidth + Units.spacingLG * 2
                implicitHeight: chipLabel.implicitHeight + Units.spacingSM * 2
                radius: height / 2
                // Action chips read as buttons: ember focus fill (palette's
                // secondary-interactive colour), not the segment crimson.
                color: isAction ? (isFocused ? Theme.ember : Theme.surface) : (showSelectedFill ? Theme.sidebarActive : isFocused ? Theme.surfaceHover : Theme.surface)
                border.width: isFocused ? Units.borderMedium : Units.borderThin
                border.color: isFocused ? Theme.focusBorder : Theme.surfaceBorder

                Behavior on color {
                    ColorAnimation {
                        duration: 150
                    }
                }

                Text {
                    id: chipLabel
                    anchors.centerIn: parent
                    text: chip.modelData.label
                    font.pixelSize: Theme.fontBody
                    font.bold: true
                    color: (chip.showSelectedFill || (chip.isAction && chip.isFocused)) ? Theme.textOnDark : Theme.textPrimary
                }

                MouseArea {
                    anchors.fill: parent
                    hoverEnabled: true
                    cursorShape: Qt.PointingHandCursor
                    onPositionChanged: mouse => {
                        let p = mapToItem(null, mouse.x, mouse.y);
                        InputMode.pointerMoved(p.x, p.y);
                    }
                    onClicked: {
                        InputMode.enterMouseMode();
                        root.forceActiveFocus();
                        root._focusIndex = chip.index;
                        // Click commits (mouse users expect select-on-click) —
                        // both kinds fire their signal, mirroring _activate().
                        if (chip.isAction)
                            root.actionTriggered(chip.modelData.value);
                        else
                            root.filterChanged(chip.modelData.value);
                    }
                }
            }
        }
    }
}
