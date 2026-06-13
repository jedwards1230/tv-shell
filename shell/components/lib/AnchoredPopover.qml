import QtQuick
import QtQuick.Layouts
import "../"

// Anchored quick-popover frame (#118). A light click-to-dismiss scrim plus a
// panel that positions itself relative to `anchorRect` (scene-root coords),
// flipping above/below the anchor and clamping fully on-screen. Hosts arbitrary
// content via the default property alias; callers own the FocusScope/key handling
// around it (this is a frame, not a focus container).
//
// Extracted verbatim from VolumeOverlay/NetworkOverlay — positioning behavior
// must match those exactly.
Item {
    id: root

    // Scene-root rect {x, y, w, h} of the glyph that opened this popover.
    property var anchorRect: null

    // Panel width (callers differ: Volume = gridUnit*22, Network = gridUnit*24).
    property real panelWidth: Units.gridUnit * 22

    // Scrim opacity — popover, not a full modal.
    property real scrimOpacity: 0.35

    // Emitted when the scrim (click-outside) is tapped.
    signal dismissed

    // Inner panel content goes here (a ColumnLayout, typically).
    default property alias content: overlayColumn.data

    anchors.fill: parent

    // Light scrim — click-outside dismisses.
    Rectangle {
        anchors.fill: parent
        color: Qt.rgba(0, 0, 0, root.scrimOpacity)
        MouseArea {
            anchors.fill: parent
            onClicked: root.dismissed()
        }
    }

    // === Anchored popover panel ===
    Rectangle {
        id: panel
        width: Math.min(root.panelWidth, root.width - Units.gridUnit * 2)
        height: overlayColumn.implicitHeight + Units.gridUnit * 1.5
        radius: Units.radiusLG
        color: Theme.surface
        border.width: Units.borderMedium
        border.color: Theme.surfaceBorder

        readonly property real _gap: Units.spacingMD
        readonly property real _ax: root.anchorRect ? root.anchorRect.x : 0
        readonly property real _ay: root.anchorRect ? root.anchorRect.y : 0
        readonly property real _aw: root.anchorRect ? root.anchorRect.w : 0
        readonly property real _ah: root.anchorRect ? root.anchorRect.h : 0
        readonly property bool _below: (_ay + _ah / 2) < root.height / 2

        x: {
            if (!root.anchorRect)
                return (root.width - width) / 2;
            var desired = _below ? (_ax + _aw - width) : _ax;
            var maxX = root.width - width - Units.spacingLG;
            return Math.max(Units.spacingLG, Math.min(desired, maxX));
        }
        y: {
            if (!root.anchorRect)
                return (root.height - height) / 2;
            var desired = _below ? (_ay + _ah + _gap) : (_ay - height - _gap);
            var maxY = root.height - height - Units.spacingLG;
            return Math.max(Units.spacingLG, Math.min(desired, maxY));
        }

        ColumnLayout {
            id: overlayColumn
            anchors {
                left: parent.left
                right: parent.right
                top: parent.top
                margins: Units.gridUnit * 0.75
            }
            spacing: Units.spacingMD
        }
    }
}
