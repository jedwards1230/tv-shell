import QtQuick

// Test-only MULTI-ROW home widget — exposes firstRow/lastRow so the WidgetHost
// chain resolves a DOWN neighbour to this widget's firstRow and an UP neighbour
// to its lastRow (the entry/exit distinction multi-row widgets like Moonlight /
// Plex rely on). Focusability tracks widgetEnabled.
Widget {
    id: root
    wantVisible: widgetEnabled
    canFocus: visible
    implicitWidth: 100
    implicitHeight: 80

    firstRow: top
    lastRow: bottom

    function focusFirstChild() {
        if (!root.canFocus)
            return false;
        top.forceActiveFocus();
        return true;
    }

    Item {
        id: top
    }
    Item {
        id: bottom
    }
}
