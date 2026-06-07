import QtQuick
import Quickshell

BaseCard {
    id: root

    required property var app
    // `running` is inherited from BaseCard, which renders the ember status dot
    // beside the name and enables Resume/Close actions instead of Launch.

    label: root.app.name || "Unknown"
    // Expose running state on the card itself (mirrors StreamCard's
    // online/session Accessible.description) rather than via a separate a11y node.
    Accessible.description: root.running ? "Running" : ""

    // Resolve the icon imperatively via Quickshell.iconPath(name, /*check*/ true),
    // which returns "" when the icon genuinely isn't in the theme (#194). The old
    // image://icon/<name> path returned a Ready *placeholder* texture for missing
    // icons (the magenta "broken icon" tile) and could keep a recycled ListView
    // delegate's previous texture (Plex card rendered the Discord icon) — both
    // bypass a status-only fallback. Clearing the source first drops any retained
    // pixmap; an unresolved icon then lands on the letter fallback below, never a
    // placeholder or a neighbour's logo.
    function _refreshIcon() {
        iconImage.source = "";
        if (root.app && root.app.icon) {
            let p = Quickshell.iconPath(root.app.icon, true);
            if (p)
                iconImage.source = p;
        }
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
