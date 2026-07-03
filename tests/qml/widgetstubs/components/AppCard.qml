import QtQuick

// Test stub for components/AppCard.qml (the real one extends BaseCard and loads
// image://icon/ Freedesktop icons). Declares only the delegate surface AppsWidget's
// rail assigns, so the delegate Component compiles and instantiates cleanly when a
// model is injected. No visuals.
Item {
    id: root

    property bool iconOnly: false
    property var app: null
    property bool running: false

    signal activated
}
