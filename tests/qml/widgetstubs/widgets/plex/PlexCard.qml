import QtQuick

// Test stub for widgets/plex/PlexCard.qml (the real one uses QtQuick.Effects + Plex
// artwork). Declares only PlexWidget's poster-row delegate surface, so the delegate
// Component compiles. No visuals.
Item {
    id: root

    property int posterWidth: 0
    property int posterHeight: 0
    property bool showCaption: true
    property string title: ""
    property string subtitle: ""
    property string art: ""
    property real progress: 0

    signal activated
}
