import QtQuick
import QtQuick.Layouts

Item {
    id: root
    width: Theme.cardWidth
    height: Theme.cardHeight

    property string label: ""
    property bool isFocused: (activeFocus && !Theme.mouseMode) || (mouseArea.containsMouse && Theme.mouseMode)
    default property alias iconContent: iconArea.data

    signal activated

    Connections {
        target: Theme
        function onMouseModeChanged() {
            if (!Theme.mouseMode && mouseArea.containsMouse) {
                if (root.ListView.view)
                    root.ListView.view.currentIndex = root.ListView.view.indexAt(root.x, root.y);
                root.forceActiveFocus();
            }
        }
    }

    z: root.isFocused ? 10 : 0

    FocusFrame {
        id: focusFrame
        anchors.fill: parent
        focused: root.isFocused

        MouseArea {
            id: mouseArea
            anchors.fill: parent
            hoverEnabled: true
            cursorShape: Qt.PointingHandCursor
            // Mouse-mode engages on a real click only. We deliberately do NOT
            // flip on onEntered/onPositionChanged: when a row scrolls to center
            // the controller-focused card, cards slide under a *stationary*
            // cursor and fire those handlers, hijacking focus mid-nav (#45).
            // Proper hover-to-mouse-mode needs a real global-cursor-delta check;
            // deferred — controller nav reliability wins.
            onClicked: {
                Theme.enterMouseMode();
                root.forceActiveFocus();
                root.activated();
            }
        }

        ColumnLayout {
            anchors.fill: parent
            anchors.margins: Theme.padding / 2
            spacing: 8

            Item {
                Layout.fillHeight: true
            }

            Item {
                id: iconArea
                Layout.preferredWidth: Units.iconSizeXL
                Layout.preferredHeight: Units.iconSizeXL
                Layout.alignment: Qt.AlignHCenter
            }

            Item {
                Layout.fillHeight: true
            }

            MarqueeText {
                Layout.fillWidth: true
                Layout.preferredHeight: Theme.fontSmall * 1.3
                animate: root.isFocused
                text: root.label
                font.pixelSize: Theme.fontSmall
                font.bold: true
                color: Theme.textPrimary
            }
        }
    }

    Keys.onReturnPressed: root.activated()
    Keys.onEnterPressed: root.activated()
}
