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

    // Resolve the icon imperatively (not via a plain binding) so a recycled
    // ListView delegate can never keep showing the PREVIOUS app's texture — the
    // reported #194 bug (the Plex card rendered the Discord icon). Clearing the
    // source the instant `app` changes drops any retained pixmap before the new
    // icon loads, so a card never shows a neighbour's logo. (Apps whose icon name
    // isn't in the theme still render the provider's placeholder tile rather than
    // the letter — a separate, pre-existing cosmetic gap, tracked for follow-up.)
    function _refreshIcon() {
        iconImage.source = "";
        if (root.app && root.app.icon)
            iconImage.source = "image://icon/" + root.app.icon;
    }
    onAppChanged: _refreshIcon()
    Component.onCompleted: _refreshIcon()

    Image {
        id: iconImage
        anchors.fill: parent
        sourceSize: Qt.size(Units.iconSizeXL, Units.iconSizeXL)
        fillMode: Image.PreserveAspectFit
        // Don't share/keep cached textures across delegates — a cache hit is how
        // a stale neighbour icon leaks in when the new name doesn't resolve.
        cache: false
        visible: status === Image.Ready && source != ""
    }

    Text {
        visible: !iconImage.visible
        anchors.fill: parent
        text: (root.app.name || "?").charAt(0).toUpperCase()
        font.pixelSize: Units.iconSizeLG
        font.bold: true
        color: Theme.textSecondary
        horizontalAlignment: Text.AlignHCenter
        verticalAlignment: Text.AlignVCenter
    }
}
