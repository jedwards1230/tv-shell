import QtQuick

Rectangle {
    id: root
    implicitWidth: label.implicitWidth + 80
    implicitHeight: 96
    width: implicitWidth
    height: implicitHeight
    radius: 16
    color: ((root.activeFocus || root.highlighted) && !Theme.mouseMode) || (mouseArea.containsMouse && Theme.mouseMode) ? Theme.surfaceHover : Theme.surface
    border.width: (root.activeFocus || root.highlighted) && !Theme.mouseMode ? 3 : 2
    border.color: (root.activeFocus || root.highlighted) && !Theme.mouseMode ? Theme.focusBorder : Theme.surfaceBorder

    property alias text: label.text
    // Externally-driven focus styling for call sites that manage selection via a
    // parent index (e.g. MoonlightSettings server-row actions) rather than real
    // QML focus. Renders the focused look without holding activeFocus.
    property bool highlighted: false

    // Single activation signal — emitted by mouse click, Return/Enter, and
    // the AT-SPI press action so all three routes converge (mirrors
    // BaseCard.qml). Call sites handle onActivated.
    signal activated

    Accessible.role: Accessible.Button
    Accessible.name: label.text
    Accessible.focusable: true
    Accessible.onPressAction: root.activated()

    Connections {
        target: Theme
        function onMouseModeChanged() {
            if (!Theme.mouseMode && mouseArea.containsMouse)
                root.forceActiveFocus();
        }
    }

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

    MouseArea {
        id: mouseArea
        anchors.fill: parent
        hoverEnabled: true
        cursorShape: Qt.PointingHandCursor
        // Hover flips to mouse-mode only on a GENUINE pointer move, filtered by
        // Theme.pointerMoved (global-coords delta). No onEntered: containsMouse
        // flips when content scrolls under a stationary cursor and would hijack
        // controller-nav focus (#45). Coords mapped to scene root (null) —
        // mapToItem (used elsewhere here) over mapToGlobal (used nowhere).
        onPositionChanged: mouse => {
            let p = mapToItem(null, mouse.x, mouse.y);
            Theme.pointerMoved(p.x, p.y);
        }
        onClicked: {
            Theme.enterMouseMode();
            root.forceActiveFocus();
            root.activated();
        }
    }

    Text {
        id: label
        anchors.centerIn: parent
        font.pixelSize: Theme.fontBody
        // When a call site fills the button with the active accent (sidebarActive,
        // e.g. an "On" toggle or a selected chip), the label must be light or it
        // is unreadable in light mode (textPrimary is dark there). Derive the
        // label color from the actual fill so every blue-filled SettingsButton
        // gets readable text without each call site repeating the logic.
        color: root.color === Theme.sidebarActive ? Theme.textOnDark : Theme.textPrimary
    }

    Keys.onReturnPressed: root.activated()
    Keys.onEnterPressed: root.activated()
}
