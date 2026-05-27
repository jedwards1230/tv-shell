import QtQuick

Item {
    id: root
    clip: true

    property alias text: label.text
    property alias font: label.font
    property alias color: label.color
    property bool animate: true
    property int scrollSpeed: 50  // pixels per second

    implicitHeight: label.implicitHeight

    Text {
        id: label
        y: 0

        // Only scroll if text is wider than container
        property bool needsScroll: implicitWidth > root.width && root.animate

        SequentialAnimation on x {
            running: label.needsScroll
            loops: Animation.Infinite

            // Pause at start
            PauseAnimation {
                duration: 2000
            }

            // Scroll left to reveal full text
            NumberAnimation {
                from: 0
                to: -(label.implicitWidth - root.width + 40)
                duration: Math.max(1000, (label.implicitWidth - root.width + 40) / root.scrollSpeed * 1000)
                easing.type: Easing.Linear
            }

            // Pause at end
            PauseAnimation {
                duration: 1500
            }

            // Snap back
            NumberAnimation {
                from: -(label.implicitWidth - root.width + 40)
                to: 0
                duration: 400
                easing.type: Easing.OutCubic
            }
        }

        // Reset position when not scrolling
        onNeedsScrollChanged: {
            if (!needsScroll)
                x = 0;
        }
    }
}
