import QtQuick
import QtQuick.Layouts
import "lib"

// Home-screen Moonlight widget (#249) — the "jump into streaming" surface: one
// row of server cards (one per configured Moonlight target) with online / active-
// session status, fed by `StreamProviders.active.targets`. Selecting a server
// streams it; the context key offers Resume/Quit when a session is live.
//
// This is the GLANCE surface (is my PC up → stream it). The exhaustive per-host
// *app* browse (apps-view) deliberately stays one level deeper in the Library
// ("All Apps") — the home widget is the quick action, not the catalog.
//
// `size` reformats the row (a reflow, not a scale), mirroring the Recent widget:
//   small  = icon-only square tiles (server name dropped) — a compact online rail
//   medium = full cards with the server name (default)
//
// Implements the duck-typed home-tile focus contract (visible / regionFocused /
// canFocus / firstRow / lastRow / focusFirstChild + previousRow/nextRow) so
// HomeScreen drives it from the same ordered region list as the other widgets.
ColumnLayout {
    id: root

    property Item previousRow: null
    property Item nextRow: null
    property bool widgetEnabled: true
    // "small" | "medium".
    property string size: "medium"

    property var targets: []
    property string shellState: "idle"

    signal escaped
    signal streamRequested(var target)
    signal streamQuitRequested(var target)
    signal ensureVisibleRequested(var item)
    // Raised on the context key; HomeScreen owns the PopoverMenu and reads
    // currentTarget / currentCard / currentHasSession to build it.
    signal contextRequested

    spacing: Units.spacingMD

    readonly property bool _hasTargets: root.targets.length > 0
    visible: root.widgetEnabled && _hasTargets

    // === Size reflow ===
    readonly property bool _iconOnly: root.size === "small"
    readonly property int _cardW: _iconOnly ? Theme.cardHeight : Theme.cardWidth

    // === Home-tile focus contract ===
    readonly property var firstRow: serverRow
    readonly property var lastRow: serverRow
    readonly property bool canFocus: visible && _hasTargets
    readonly property bool regionFocused: serverRow.activeFocus

    // Context-menu passthrough for HomeScreen.
    readonly property var currentTarget: (serverRow.currentIndex >= 0 && serverRow.currentIndex < root.targets.length) ? root.targets[serverRow.currentIndex] : null
    readonly property Item currentCard: serverRow.currentItem
    readonly property bool currentHasSession: serverRow.currentItem ? serverRow.currentItem.hasActiveSession === true : false

    function focusFirstChild() {
        if (!root.canFocus)
            return false;
        return serverRow.focusFirstChild();
    }

    Text {
        Layout.fillWidth: true
        text: "Moonlight"
        font.pixelSize: Theme.fontTitle
        font.bold: true
        color: Theme.textPrimary
    }

    NavigableRow {
        id: serverRow
        Layout.fillWidth: true
        Layout.preferredHeight: Theme.cardHeight
        keyNavigationWraps: true
        previousRow: root.previousRow
        nextRow: root.nextRow
        model: root.targets
        onActiveFocusChanged: if (activeFocus)
            root.ensureVisibleRequested(this)
        onActivated: {
            if (root.currentTarget)
                root.streamRequested(root.currentTarget);
        }
        onContextRequested: root.contextRequested()
        onEscaped: root.escaped()

        delegate: StreamCard {
            required property int index
            required property var modelData
            iconOnly: root._iconOnly
            width: root._cardW
            height: Theme.cardHeight
            target: modelData
            shellState: root.shellState
            focus: index === serverRow.currentIndex
            onActivated: root.streamRequested(modelData)
        }
    }
}
