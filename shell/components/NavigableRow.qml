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
        listView.currentIndex = 0;
        forceActiveFocus();
        return true;
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
            if (listView.currentIndex > 0)
                listView.currentIndex--;
            else if (root.keyNavigationWraps && listView.count > 0)
                listView.currentIndex = listView.count - 1;
        }
        Keys.onRightPressed: {
            Theme.exitMouseMode();
            if (listView.currentIndex < listView.count - 1)
                listView.currentIndex++;
            else if (root.keyNavigationWraps && listView.count > 0)
                listView.currentIndex = 0;
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
