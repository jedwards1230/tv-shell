import QtQuick
import QtQuick.Layouts
import "../"

// Shared collapsible dropdown for settings pages.
//
// Usage:
//   SettingsDropdown {
//       model: myArray              // var[] — items to list
//       displayText: "My Value"     // text shown in collapsed header
//       isCurrentItem: function(item) { return item === currentVal }
//       itemLabel:     function(item) { return item.label }
//       onItemSelected: function(item) { ... }
//       KeyNavigation.up: siblingAbove
//       KeyNavigation.down: siblingBelow
//   }
//
// The component is a FocusScope so SettingsPanel's Flickable scroll-follow
// (which tracks activeFocusItem) continues to work.  The internal ListView
// does NOT fill the height — it clips to the expanding FocusScope so there
// is no inner scroll viewport breaking the outer one.
//
// All five original dropdown sites had different isCurrent logic and commit
// behaviour, so those are delegated back to the parent via:
//   isCurrentItem(modelData) -> bool
//   itemLabel(modelData)     -> string
//   onItemSelected(modelData) signal / function
//
// KeyNavigation.up/.down are normal alias'd properties on the root FocusScope
// (QML attaches them to whatever object you set them on directly).

FocusScope {
    id: root

    // --- Public API ---

    // The list model passed to the internal ListView.
    property var model: []

    // Text shown in the collapsed header (caller computes it).
    property string displayText: ""

    // Called with each modelData item: return true if it is the "current" selection.
    property var isCurrentItem: function (item) {
        return false;
    }

    // Called with each modelData item: return the display string for that row.
    property var itemLabel: function (item) {
        return String(item);
    }

    // Emitted when the user confirms a selection (Return on a list row, or
    // double-click). Parent connects: SettingsDropdown { onItemSelected: ... }
    signal itemSelected(var item)

    // Row / header geometry — match the originals.
    property int headerHeight: 80
    property int rowHeight: 68
    property int maxHeight: 400

    // --- Internal state ---
    property bool _open: false

    // Height animates between headerHeight (collapsed) and
    // min(count*rowHeight + headerHeight + 8, maxHeight + headerHeight + 8) (expanded).
    Layout.fillWidth: true
    Layout.preferredHeight: _open ? Math.min(root.model.length * root.rowHeight + root.headerHeight + 8, root.maxHeight + root.headerHeight + 8) : root.headerHeight

    Behavior on Layout.preferredHeight {
        NumberAnimation {
            duration: Theme.reduceMotion ? 0 : 200
            easing.type: Easing.OutCubic
        }
    }

    // Collapsed header
    Rectangle {
        id: header
        width: parent.width
        height: root.headerHeight
        radius: 16
        color: root.activeFocus && !root._open ? Theme.surfaceHover : Theme.surface
        border.width: 2
        border.color: root.activeFocus ? Theme.focusBorder : Theme.surfaceBorder

        Behavior on color {
            ColorAnimation {
                duration: Theme.reduceMotion ? 0 : 150
            }
        }
        Behavior on border.color {
            ColorAnimation {
                duration: Theme.reduceMotion ? 0 : 150
            }
        }

        RowLayout {
            anchors.fill: parent
            anchors.leftMargin: 24
            anchors.rightMargin: 24
            spacing: 16

            Text {
                text: root.displayText
                font.pixelSize: Theme.fontSmall
                color: Theme.textPrimary
                elide: Text.ElideRight
                Layout.fillWidth: true
            }

            Text {
                text: "(current)"
                font.pixelSize: Theme.fontHint
                color: Theme.textMuted
            }

            Text {
                text: root._open ? "▲" : "▼"
                font.pixelSize: Theme.fontSmall
                color: Theme.textSecondary
            }
        }

        MouseArea {
            anchors.fill: parent
            cursorShape: Qt.PointingHandCursor
            onClicked: {
                root.forceActiveFocus();
                root._open = !root._open;
                if (root._open)
                    dropList.forceActiveFocus();
            }
        }
    }

    // Expanding list
    ListView {
        id: dropList
        anchors.top: header.bottom
        anchors.topMargin: 8
        width: parent.width
        height: parent.height - root.headerHeight - 8
        spacing: 4
        clip: true
        visible: root._open
        // Disabled while collapsed so the FocusScope cannot re-delegate active
        // focus back into this (invisible) list after an open/close cycle — that
        // left the Up/Down clamp handlers below eating nav keys, trapping focus on
        // a closed dropdown. Disabled => not focusable => root scope holds focus
        // and its KeyNavigation.up/down work normally.
        enabled: root._open
        model: root.model
        keyNavigationEnabled: true
        highlightFollowsCurrentItem: true
        highlightMoveDuration: Theme.reduceMotion ? 0 : 100

        delegate: Rectangle {
            required property int index
            required property var modelData

            width: dropList.width
            height: root.rowHeight
            radius: 12

            property bool isCurrent: root.isCurrentItem(modelData)

            color: {
                if (isCurrent)
                    return Theme.sidebarActive;
                if (dropList.currentIndex === index && dropList.activeFocus)
                    return Theme.surfaceHover;
                return Theme.cardBackground;
            }
            border.width: isCurrent ? 2 : 1
            border.color: isCurrent ? Theme.focusBorder : Theme.surfaceBorder

            Behavior on color {
                ColorAnimation {
                    duration: Theme.reduceMotion ? 0 : 150
                }
            }

            Text {
                anchors.centerIn: parent
                text: root.itemLabel(modelData) + (isCurrent ? "  (current)" : "")
                font.pixelSize: Theme.fontSmall
                color: isCurrent ? Theme.textOnDark : Theme.textPrimary
                elide: Text.ElideRight
                width: parent.width - 48
                horizontalAlignment: Text.AlignHCenter
            }

            MouseArea {
                anchors.fill: parent
                cursorShape: Qt.PointingHandCursor
                onClicked: {
                    dropList.currentIndex = index;
                    dropList.forceActiveFocus();
                }
                onDoubleClicked: {
                    root.itemSelected(modelData);
                    root._open = false;
                    root.forceActiveFocus();
                }
            }
        }

        Keys.onReturnPressed: {
            if (currentIndex >= 0 && currentIndex < root.model.length) {
                root.itemSelected(root.model[currentIndex]);
                root._open = false;
                root.forceActiveFocus();
            }
        }

        Keys.onEscapePressed: {
            root._open = false;
            root.forceActiveFocus();
        }

        // Clamp Up/Down at the ends so focus cannot escape the open list.
        Keys.onUpPressed: event => {
            if (currentIndex > 0)
                currentIndex--;
            event.accepted = true;
        }

        Keys.onDownPressed: event => {
            if (currentIndex < root.model.length - 1)
                currentIndex++;
            event.accepted = true;
        }
    }

    // Header key handling — open on A, close on Escape
    Keys.onReturnPressed: {
        if (!_open) {
            _open = true;
            // Pre-select the current item
            for (var i = 0; i < root.model.length; i++) {
                if (root.isCurrentItem(root.model[i])) {
                    dropList.currentIndex = i;
                    break;
                }
            }
            dropList.forceActiveFocus();
        }
    }

    Keys.onEscapePressed: {
        if (_open) {
            _open = false;
        } else {
            event.accepted = false;
        }
    }
}
