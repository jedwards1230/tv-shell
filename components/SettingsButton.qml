import QtQuick

Rectangle {
    id: root
    implicitWidth: label.implicitWidth + 80
    implicitHeight: 96
    width: implicitWidth
    height: implicitHeight
    radius: 16
    color: (root.activeFocus && !Theme.mouseMode) || (mouseArea.containsMouse && Theme.mouseMode) ? Theme.surfaceHover : Theme.surface
    border.width: root.activeFocus && !Theme.mouseMode ? 3 : 2
    border.color: root.activeFocus && !Theme.mouseMode ? Theme.focusBorder : Theme.surfaceBorder

    property alias text: label.text

    Connections {
        target: Theme
        function onMouseModeChanged() {
            if (!Theme.mouseMode && mouseArea.containsMouse)
                root.forceActiveFocus();
        }
    }

    Behavior on color {
        ColorAnimation {
            duration: 150
        }
    }
    Behavior on border.color {
        ColorAnimation {
            duration: 150
        }
    }

    MouseArea {
        id: mouseArea
        anchors.fill: parent
        hoverEnabled: true
        cursorShape: Qt.PointingHandCursor
        // Mouse-mode engages on a real click only — NOT onEntered/onPositionChanged,
        // which fire when content scrolls under a stationary cursor and hijack
        // controller-nav focus (#45 hover-mode deferred; needs a global-cursor delta).
        onClicked: {
            Theme.enterMouseMode();
            root.forceActiveFocus();
            root.Keys.returnPressed(null);
        }
    }

    Text {
        id: label
        anchors.centerIn: parent
        font.pixelSize: Theme.fontBody
        color: Theme.textPrimary
    }
}
