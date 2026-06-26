import QtQuick
import QtQuick.Layouts
import "lib"

// Home-screen Recent (apps) widget (#249) — the "Recent" header + a horizontal
// rail of recently-used / running apps. Extends Widget (a FocusScope hosting the
// header Text + NavigableRow), exposing the home-tile focus contract by
// delegating to the inner NavigableRow.
//
// The MODEL (running windows + recents merge, plus the widget-shadow
// hide-from-Recent suppression) stays owned by HomeScreen and is passed in via
// `model`; this widget renders it and bubbles activation / context / escape up so
// HomeScreen keeps owning the launch/focus path and the PopoverMenu.
Widget {
    id: root

    // Merged recent/running model (HomeScreen-owned), and the small-size reflow
    // flag (small = icon-only square tiles; medium = full icon + label cards).
    property var model: []
    property bool recentSmall: false

    // Bubbled up so HomeScreen keeps the launch/focus + PopoverMenu logic.
    signal entryActivated(var entry)
    signal entryContextRequested(var entry, var card)
    signal ensureVisibleRequested(var item)

    wantVisible: root.widgetEnabled && root.model.length > 0

    implicitWidth: col.implicitWidth
    implicitHeight: root.wantVisible ? col.implicitHeight : 0

    // Surfaced for HomeScreen's hint bar (current row selection).
    readonly property int currentIndex: recentRow.currentIndex

    // === Home-tile focus contract (delegates to the inner row) ===
    firstRow: recentRow
    lastRow: recentRow
    canFocus: visible && recentRow.count > 0

    function focusFirstChild() {
        if (!root.canFocus)
            return false;
        return recentRow.focusFirstChild();
    }

    ColumnLayout {
        id: col
        width: root.width
        spacing: 24

        Text {
            text: "Recent"
            font.pixelSize: Theme.fontTitle
            font.bold: true
            color: Theme.textPrimary
        }

        NavigableRow {
            id: recentRow
            Layout.fillWidth: true
            Layout.preferredHeight: Theme.rowHeight
            keyNavigationWraps: true
            previousRow: root.previousRow
            nextRow: root.nextRow
            model: root.model
            onActiveFocusChanged: if (activeFocus)
                root.ensureVisibleRequested(recentRow)

            delegate: AppCard {
                required property int index
                required property var modelData
                iconOnly: root.recentSmall
                width: root.recentSmall ? Theme.cardHeight : Theme.cardWidth
                height: Theme.cardHeight
                app: modelData
                running: modelData.running === true
                focus: index === recentRow.currentIndex
                onActivated: root.entryActivated(modelData)
            }

            onContextRequested: {
                if (currentItem && currentIndex >= 0 && currentIndex < root.model.length)
                    root.entryContextRequested(root.model[currentIndex], currentItem);
            }
            onEscaped: root.escaped()
        }
    }
}
