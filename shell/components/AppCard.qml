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
    // ListView delegate can never keep showing the PREVIOUS app's texture when
    // the new icon fails to resolve. On some targets there is no icon theme, so
    // image://icon/<name> frequently can't resolve (#194 — Plex card rendered
    // the Discord icon). Clearing the source first drops any retained pixmap;
    // the failed reload then lands on the letter fallback below, never a
    // neighbouring delegate's logo.
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
