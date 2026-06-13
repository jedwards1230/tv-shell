import QtQuick
import QtQuick.Layouts

// Portrait poster card for the Plex widget (On Deck / Recently Added). Renders
// the artwork the daemon baked a tokenized URL for, a resume-progress bar when
// the item is partially watched, and a title/subtitle caption. Controller-
// focusable like AppCard/StreamCard: it raises a crimson FocusFrame ring on
// focus and emits `activated` on A/Return (the host opens Plex).
Item {
    id: root

    property string title: ""
    property string subtitle: ""
    // Ready-to-load artwork URL (built daemon-side, token already embedded).
    property string art: ""
    // 0..1 resume position; >0 paints the bottom progress bar.
    property real progress: 0

    // Poster geometry (set by the host row so all cards match).
    property int posterWidth: Math.round(Theme.cardWidth * 0.62)
    property int posterHeight: Math.round(posterWidth * 1.5)

    readonly property bool isFocused: (activeFocus && !Theme.mouseMode) || (mouseArea.containsMouse && Theme.mouseMode)

    signal activated

    width: posterWidth
    height: posterHeight + captionCol.implicitHeight + Units.spacingSM

    Accessible.role: Accessible.Button
    Accessible.name: root.title + (root.subtitle !== "" ? ", " + root.subtitle : "")
    Accessible.focusable: true
    Accessible.onPressAction: root.activated()

    // Mirror BaseCard: when leaving mouse-mode with the pointer still over this
    // card, claim controller focus so the crimson ring lands where the cursor was.
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

    ColumnLayout {
        anchors.fill: parent
        spacing: Units.spacingSM

        // === Poster ===
        FocusFrame {
            id: frame
            focused: root.isFocused
            radius: Units.radiusMD
            Layout.preferredWidth: root.posterWidth
            Layout.preferredHeight: root.posterHeight
            Layout.alignment: Qt.AlignHCenter

            Rectangle {
                anchors.fill: parent
                radius: Units.radiusMD
                color: Theme.surface
                clip: true

                Image {
                    id: poster
                    anchors.fill: parent
                    source: root.art
                    fillMode: Image.PreserveAspectCrop
                    asynchronous: true
                    cache: true
                    visible: status === Image.Ready
                }

                // Fallback while loading / when art is missing: the title initial
                // (or a clapperboard glyph when there's no title yet).
                Text {
                    anchors.centerIn: parent
                    visible: poster.status !== Image.Ready
                    text: root.title !== "" ? root.title.charAt(0).toUpperCase() : "\u{1F3AC}"
                    font.pixelSize: Units.iconSizeLG
                    font.bold: true
                    color: Theme.textMuted
                }

                // === Resume progress bar ===
                // Bottom-anchored ember fill over a dark track; only shown for
                // partially-watched On Deck items.
                Rectangle {
                    visible: root.progress > 0
                    anchors.left: parent.left
                    anchors.right: parent.right
                    anchors.bottom: parent.bottom
                    height: Math.max(4, Math.round(Units.gridUnit * 0.16))
                    color: Qt.rgba(0, 0, 0, 0.55)

                    Rectangle {
                        anchors.left: parent.left
                        anchors.top: parent.top
                        anchors.bottom: parent.bottom
                        width: parent.width * Math.max(0, Math.min(1, root.progress))
                        color: Theme.ember
                    }
                }
            }
        }

        // === Caption ===
        ColumnLayout {
            id: captionCol
            Layout.preferredWidth: root.posterWidth
            Layout.alignment: Qt.AlignHCenter
            spacing: 2

            MarqueeText {
                Layout.fillWidth: true
                Layout.preferredHeight: Theme.fontSmall * 1.3
                animate: root.isFocused
                text: root.title
                font.pixelSize: Theme.fontSmall
                font.bold: true
                color: Theme.textPrimary
            }

            Text {
                Layout.fillWidth: true
                visible: root.subtitle !== ""
                text: root.subtitle
                elide: Text.ElideRight
                maximumLineCount: 1
                font.pixelSize: Theme.fontCaption
                color: Theme.textMuted
            }
        }
    }

    MouseArea {
        id: mouseArea
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
            root.activated();
        }
    }

    Keys.onReturnPressed: root.activated()
    Keys.onEnterPressed: root.activated()
}
