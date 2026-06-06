import QtQuick

BaseCard {
    id: root

    required property var app
    // `running` is inherited from BaseCard, which renders the ember status dot
    // beside the name and enables Resume/Close actions instead of Launch.

    label: root.app.name || "Unknown"
    // Expose running state on the card itself (mirrors StreamCard's
    // online/session Accessible.description) rather than via a separate a11y node.
    Accessible.description: root.running ? "Running" : ""

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
}
