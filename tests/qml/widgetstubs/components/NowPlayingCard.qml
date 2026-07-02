import QtQuick

// Test stub for components/NowPlayingCard.qml (the real root is a FocusFrame →
// QtQuick.Effects, with Canvas-drawn transport art). NowPlayingWidget's Loader
// instantiates it by default (medium size), so it must load; it only needs the
// `base` handle back to the MprisPlayerBase host.
Item {
    id: card

    property Item base: null
    implicitHeight: 1
}
