import QtQuick
import QtQuick.Layouts
import "lib"

// Horizontal filter-chip strip for the home "New on Plex" rail (#249). Unlike
// the settings SettingsButtonGroup (declarative KeyNavigation), this implements
// the home-tile focus contract (visible / regionFocused / focusFirstChild) and
// walks previousRow/nextRow via forceActiveFocus like NavigableRow, so it slots
// into HomeScreen's _contentRegions() chain and skips hidden neighbours.
// Left/Right move the active chip and apply the filter live; Up/Down leave the
// strip; B/Escape bubbles up.
FocusScope {
    id: root

    property var previousRow: null
    property var nextRow: null

    // [{ label, value }] — value is opaque, handed back via filterChanged.
    property var options: []
    property int currentIndex: 0

    signal filterChanged(var value)
    signal escaped

    // === Home-tile focus contract ===
    readonly property bool regionFocused: activeFocus

    function focusFirstChild() {
        if (!visible)
            return false;
        forceActiveFocus();
        return true;
    }

    implicitWidth: chipRow.implicitWidth
    implicitHeight: chipRow.implicitHeight
    Layout.preferredWidth: chipRow.implicitWidth
    Layout.preferredHeight: chipRow.implicitHeight

    function _select(i) {
        if (i < 0 || i >= options.length)
            return;
        root.currentIndex = i;
        root.filterChanged(options[i].value);
    }

    Keys.onPressed: event => {
        switch (event.key) {
        case Qt.Key_Left:
            Theme.exitMouseMode();
            if (root.currentIndex > 0)
                root._select(root.currentIndex - 1);
            event.accepted = true;
            break;
        case Qt.Key_Right:
            Theme.exitMouseMode();
            if (root.currentIndex < root.options.length - 1)
                root._select(root.currentIndex + 1);
            event.accepted = true;
            break;
        case Qt.Key_Up:
            Theme.exitMouseMode();
            {
                var up = root.previousRow;
                while (up) {
                    if (up.visible) {
                        up.forceActiveFocus();
                        event.accepted = true;
                        break;
                    }
                    up = (up.previousRow !== undefined) ? up.previousRow : null;
                }
            }
            break;
        case Qt.Key_Down:
            Theme.exitMouseMode();
            {
                var dn = root.nextRow;
                while (dn) {
                    if (dn.visible) {
                        dn.forceActiveFocus();
                        event.accepted = true;
                        break;
                    }
                    dn = (dn.nextRow !== undefined) ? dn.nextRow : null;
                }
            }
            break;
        case Qt.Key_Escape:
        case Qt.Key_B:
            if (event.key === Qt.Key_B && event.modifiers)
                break;
            root.escaped();
            event.accepted = true;
            break;
        }
    }

    RowLayout {
        id: chipRow
        spacing: Units.spacingMD

        Repeater {
            model: root.options

            delegate: Rectangle {
                id: chip
                required property var modelData
                required property int index
                readonly property bool isCurrent: index === root.currentIndex
                readonly property bool isFocused: root.activeFocus && !Theme.mouseMode && index === root.currentIndex

                implicitWidth: chipLabel.implicitWidth + Units.spacingLG * 2
                implicitHeight: chipLabel.implicitHeight + Units.spacingSM * 2
                radius: height / 2
                color: isCurrent ? Theme.sidebarActive : isFocused ? Theme.surfaceHover : Theme.surface
                border.width: isFocused ? Units.borderMedium : Units.borderThin
                border.color: isFocused ? Theme.focusBorder : Theme.surfaceBorder

                Behavior on color {
                    ColorAnimation {
                        duration: 150
                    }
                }

                Text {
                    id: chipLabel
                    anchors.centerIn: parent
                    text: chip.modelData.label
                    font.pixelSize: Theme.fontBody
                    font.bold: true
                    color: chip.isCurrent ? Theme.textOnDark : Theme.textPrimary
                }

                MouseArea {
                    anchors.fill: parent
                    hoverEnabled: true
                    cursorShape: Qt.PointingHandCursor
                    onPositionChanged: mouse => {
                        let p = mapToItem(null, mouse.x, mouse.y);
                        Theme.pointerMoved(p.x, p.y);
                    }
                    onClicked: {
                        Theme.enterMouseMode();
                        root.forceActiveFocus();
                        root._select(chip.index);
                    }
                }
            }
        }
    }
}
