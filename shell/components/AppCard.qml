import QtQuick

BaseCard {
    id: root

    required property var app
    // True when this card represents a currently-running window.
    // Renders a subtle ember status dot in the top-right corner and
    // enables Resume/Close actions instead of Launch.
    property bool running: false

    label: root.app.name || "Unknown"

    Image {
        id: iconImage
        anchors.fill: parent
        source: root.app.icon ? "image://icon/" + root.app.icon : ""
        sourceSize: Qt.size(Units.iconSizeXL, Units.iconSizeXL)
        fillMode: Image.PreserveAspectFit
        visible: status === Image.Ready
    }

    Text {
        visible: iconImage.status !== Image.Ready
        anchors.fill: parent
        text: (root.app.name || "?").charAt(0).toUpperCase()
        font.pixelSize: Units.iconSizeLG
        font.bold: true
        color: Theme.textSecondary
        horizontalAlignment: Text.AlignHCenter
        verticalAlignment: Text.AlignVCenter
    }

    // Running indicator — ember dot in top-right corner.
    // Ember (#e06236) is intentionally distinct from crimson (#c72138, the
    // focus-ring color) so the running state is never confused with focus.
    // Dual cue: color + circular shape (not text-only, colorblind-safe).
    Rectangle {
        id: runningDot
        visible: root.running
        width: 10
        height: 10
        radius: 5
        color: Theme.ember
        // Soft inner glow ring so it reads on both dark and light card backgrounds.
        border.width: 2
        border.color: Theme.cardBackground
        anchors.top: parent.top
        anchors.right: parent.right
        anchors.topMargin: 6
        anchors.rightMargin: 6
        z: 5

        // Accessible label so screen readers report "running" when the dot is visible.
        Accessible.role: Accessible.StaticText
        Accessible.name: "Running"
    }
}
