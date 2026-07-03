import QtQuick

// Inert stub of components/FocusFrame for the widget-contract harness. The real
// FocusFrame is a QtQuick.Effects visual bound to many Theme colors the stub Theme
// doesn't carry; the contract test only needs it to host content, hold focus, and
// forward Keys. So this is a plain focusable Item exposing the same `content`
// default alias plus the handful of properties SteamRpWidget sets. SteamRpWidget is
// the only harness widget that instantiates FocusFrame — the card leaves are stubbed
// and never draw it.
Item {
    id: root

    property bool focused: false
    property real radius: 0
    property bool scaleEnabled: true
    property real focusScale: 1.06
    property real availableWidth: 0

    default property alias content: contentArea.data

    Item {
        id: contentArea
        anchors.fill: parent
    }
}
