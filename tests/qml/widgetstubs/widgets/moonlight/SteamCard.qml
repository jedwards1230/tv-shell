import QtQuick

// Test stub for widgets/moonlight/SteamCard.qml (the real one uses QtQuick.Effects
// + Steam CDN art). Declares only SteamLibraryView's poster-row delegate surface,
// so the delegate Component compiles. No visuals.
Item {
    id: root

    property int posterWidth: 0
    property int posterHeight: 0
    property bool showCaption: true
    property string title: ""
    property string art: ""
    property string localArt: ""
    property string headerArt: ""
    property bool playing: false
    property bool locked: false

    signal activated
    signal contextRequested
}
