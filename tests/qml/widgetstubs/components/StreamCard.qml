import QtQuick

// Test stub for components/StreamCard.qml (the real one imports Quickshell.Io for
// its Sunshine session probe). Declares only the delegate surface MoonlightWidget's
// server row assigns plus the two status reads HomeScreen makes off currentItem, so
// the delegate compiles and instantiates when targets are injected. No visuals.
Item {
    id: root

    property var target: null
    property bool showProfile: false
    property string shellState: "idle"
    property bool isOnline: false
    property bool hasActiveSession: false

    signal activated
}
