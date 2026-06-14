import QtQuick
import "../"

// App icon resolver: Freedesktop `image://icon/…` with a letter-initial
// fallback when the icon isn't in the theme. Used in AppCard and LaunchOverlay.
//
// NOTE: AppCard uses an imperative _refreshIcon() to clear the source on
// `app` change and avoid ListView stale-texture bugs — that logic stays in
// AppCard.qml on top of this component. AppIcon itself is purely declarative.
//
// Usage:
//   AppIcon {
//       iconSource: root.app.icon    // may be "" or undefined
//       fallbackText: root.app.name  // first char is uppercased
//       iconSize: Units.iconSizeXL   // optional, defaults to Units.iconSizeXL
//   }
Item {
    id: root

    property string iconSource: ""
    property string fallbackText: "?"
    property int iconSize: Units.iconSizeXL

    implicitWidth: iconSize
    implicitHeight: iconSize

    Image {
        id: iconImage
        anchors.fill: parent
        source: root.iconSource ? "image://icon/" + root.iconSource : ""
        sourceSize: Qt.size(root.iconSize, root.iconSize)
        fillMode: Image.PreserveAspectFit
        cache: false
        visible: status === Image.Ready && source != ""
    }

    Text {
        visible: !iconImage.visible
        anchors.fill: parent
        text: (root.fallbackText || "?").charAt(0).toUpperCase()
        font.pixelSize: Math.round(root.iconSize * 0.75)
        font.bold: true
        color: Theme.textSecondary
        horizontalAlignment: Text.AlignHCenter
        verticalAlignment: Text.AlignVCenter
    }
}
