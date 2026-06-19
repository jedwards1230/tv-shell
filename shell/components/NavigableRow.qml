import QtQuick

FocusScope {
    id: root

    property Item previousRow: null
    property Item nextRow: null
    property alias model: listView.model
    property alias delegate: listView.delegate
    property alias currentItem: listView.currentItem
    property alias currentIndex: listView.currentIndex
    property alias count: listView.count
    property bool keyNavigationWraps: false

    // Optional per-INDEX focusability predicate. `null` (the default) means every
    // index is focusable — left/right navigation, focus-entry, and the highlight
    // behave EXACTLY as before (no index is ever skipped), so every existing
    // NavigableRow (server row, app rows, plex rows) is byte-for-byte unchanged.
    // When set to a function `(i) => bool`, left/right skip indices that return
    // false (wrapping correctly), focus-entry lands on the nearest focusable
    // index, and it's a safe no-op when NO index is focusable. The host supplies
    // this to lock the row down to a single allowed card (e.g. a running game).
    property var focusableIndex: null

    signal activated
    signal escaped
    signal contextRequested

    readonly property alias listView: listView

    // === Home-tile focus contract ===
    // Shared duck-typed interface (also implemented by MprisPlayerBase /
    // MoonlightWidget / PlexWidget) so HomeScreen's focus helpers can drive every
    // home region from one ordered list instead of per-widget branches:
    //   regionFocused : bool   — does this region currently hold focus?
    //   canFocus      : bool   — does this region have something to focus right now?
    //   focusFirstChild(): bool — focus the first selectable child; false if it
    //                             could not (hidden/empty) so callers skip it.
    readonly property bool regionFocused: activeFocus

    // Part of the contract — true only when this row is both shown and non-empty.
    // The vertical-chain walk (_navigateUp/_navigateDown) skips a neighbour whose
    // canFocus is false so focus never lands on a visible-but-empty row.
    readonly property bool canFocus: visible && listView.count > 0

    function focusFirstChild() {
        if (!visible || listView.count === 0)
            return false;
        var first = root._firstFocusableIndex();
        if (first < 0)
            return false;
        listView.currentIndex = first;
        forceActiveFocus();
        return true;
    }

    // Is index `i` focusable? `focusableIndex == null` → every index is focusable
    // (the unchanged default path). Otherwise consult the host predicate.
    function _indexFocusable(i) {
        if (root.focusableIndex === null)
            return true;
        return root.focusableIndex(i) === true;
    }

    // First focusable index scanning forward from 0, or -1 if NONE are focusable.
    function _firstFocusableIndex() {
        for (var i = 0; i < listView.count; i++) {
            if (root._indexFocusable(i))
                return i;
        }
        return -1;
    }

    // Step `currentIndex` by `dir` (-1 left / +1 right) onto the next focusable
    // index, honoring keyNavigationWraps. No-op if nothing (or only the current
    // index) is focusable, so a locked row never strands on a disabled card.
    function _stepFocusable(dir) {
        var n = listView.count;
        if (n <= 0)
            return;
        var i = listView.currentIndex;
        for (var step = 0; step < n; step++) {
            var next = i + dir;
            if (next < 0 || next >= n) {
                if (!root.keyNavigationWraps)
                    return;
                next = (next + n) % n;
            }
            i = next;
            if (root._indexFocusable(i)) {
                listView.currentIndex = i;
                return;
            }
        }
    }

    ListView {
        id: listView
        anchors.fill: parent
        anchors.topMargin: -16
        anchors.bottomMargin: -16
        leftMargin: 16
        orientation: ListView.Horizontal
        spacing: Theme.cardSpacing
        clip: false
        highlightMoveDuration: 150
        highlightMoveVelocity: -1
        keyNavigationEnabled: true
        keyNavigationWraps: root.keyNavigationWraps
        focus: true

        // Any nav/activation key means the user is driving with the controller
        // or keyboard, not the mouse — flip out of mouse-mode (no daemon hop).
        // Left/Right are consumed by keyNavigationEnabled, so intercept them
        // explicitly: exit mouse-mode, then advance honoring keyNavigationWraps
        // (the always-wrapping ListView increment/decrement methods would break
        // the default no-wrap behavior).
        Keys.onLeftPressed: {
            Theme.exitMouseMode();
            // Default (no predicate): the exact original index move. Predicate set:
            // skip non-focusable indices via _stepFocusable.
            if (root.focusableIndex !== null) {
                root._stepFocusable(-1);
            } else if (listView.currentIndex > 0) {
                listView.currentIndex--;
            } else if (root.keyNavigationWraps && listView.count > 0) {
                listView.currentIndex = listView.count - 1;
            }
        }
        Keys.onRightPressed: {
            Theme.exitMouseMode();
            if (root.focusableIndex !== null) {
                root._stepFocusable(1);
            } else if (listView.currentIndex < listView.count - 1) {
                listView.currentIndex++;
            } else if (root.keyNavigationWraps && listView.count > 0) {
                listView.currentIndex = 0;
            }
        }
        Keys.onReturnPressed: {
            Theme.exitMouseMode();
            if (listView.currentItem)
                root.activated();
        }
        Keys.onEnterPressed: {
            Theme.exitMouseMode();
            if (listView.currentItem)
                root.activated();
        }
        Keys.onEscapePressed: {
            Theme.exitMouseMode();
            root.escaped();
        }
        Keys.onTabPressed: event => {
            Theme.exitMouseMode();
            if (listView.currentItem) {
                root.contextRequested();
                event.accepted = true;
            }
        }
        Keys.onUpPressed: {
            Theme.exitMouseMode();
            root._navigateUp();
        }
        Keys.onDownPressed: {
            Theme.exitMouseMode();
            root._navigateDown();
        }
    }

    // A neighbour is focusable when its contract `canFocus` is true. Targets that
    // predate the contract (or non-widget rows like the QuickActions strip)
    // expose no canFocus, so fall back to plain `visible`.
    function _focusable(item) {
        return item.canFocus !== undefined ? item.canFocus : item.visible;
    }

    function _navigateUp() {
        var target = previousRow;
        while (target) {
            if (root._focusable(target)) {
                target.forceActiveFocus();
                return;
            }
            target = (target.previousRow !== undefined) ? target.previousRow : null;
        }
    }

    function _navigateDown() {
        var target = nextRow;
        while (target) {
            if (root._focusable(target)) {
                target.forceActiveFocus();
                return;
            }
            target = (target.nextRow !== undefined) ? target.nextRow : null;
        }
    }
}
