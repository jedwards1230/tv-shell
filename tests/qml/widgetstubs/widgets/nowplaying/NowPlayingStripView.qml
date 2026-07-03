import QtQuick

// Test stub for widgets/nowplaying/NowPlayingStripView.qml (the real root is a
// FocusFrame). Only instantiated at size "small", but its type must resolve for
// NowPlayingWidget's stripComp Component; it needs only the `base` handle.
Item {
    id: card

    property Item base: null
    implicitHeight: 1
}
