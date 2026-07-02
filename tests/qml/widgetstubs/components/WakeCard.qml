import QtQuick

// Test stub for components/WakeCard.qml (the real one draws a FocusFrame ring via
// QtQuick.Effects). SteamLibraryView always instantiates it (hidden unless the host
// is down), so it must load; a FocusScope with the props/signals the view wires.
FocusScope {
    id: root

    property int cardWidth: 0
    property int cardHeight: 0
    property string host: ""
    property bool waking: false
    property Item previousRow: null
    property Item nextRow: null

    signal activated
    signal escaped
}
