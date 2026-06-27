import QtQuick
import QtQuick.Effects
import QtQuick.Layouts
import "../../components"

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

    // Poster-only mode (small size): hide the title/subtitle caption band so the
    // card collapses to just the poster (+ resume bar). The host row sizes itself
    // to match. Title still feeds Accessible.name.
    property bool showCaption: true

    // Poster geometry (set by the host row so all cards match).
    property int posterWidth: Math.round(Theme.cardWidth * 0.62)
    property int posterHeight: Math.round(posterWidth * 1.5)

    readonly property bool isFocused: (activeFocus && !InputMode.mouseMode) || (mouseArea.containsMouse && InputMode.mouseMode)

    signal activated

    width: posterWidth
    height: posterHeight + (root.showCaption ? captionCol.implicitHeight + Units.spacingSM : 0)

    Accessible.role: Accessible.Button
    Accessible.name: root.title + (root.subtitle !== "" ? ", " + root.subtitle : "")
    Accessible.focusable: true
    Accessible.onPressAction: root.activated()

    // Mirror BaseCard: when leaving mouse-mode with the pointer still over this
    // card, claim controller focus so the crimson ring lands where the cursor was.
    Connections {
        target: Theme
        function onMouseModeChanged() {
            if (!InputMode.mouseMode && mouseArea.containsMouse) {
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

            // Poster surface, corner-rounded via a MultiEffect mask. A Rectangle's
            // `clip` only clips the bounding BOX (its `radius` never reached the
            // child Image), so the art kept square corners. Masking the whole
            // surface to a rounded rect rounds the art, the letter fallback, and the
            // resume bar together — clean, slightly rounded corners.
            Item {
                id: posterSurface
                anchors.fill: parent
                layer.enabled: true
                layer.smooth: true
                layer.effect: MultiEffect {
                    maskEnabled: true
                    maskSource: posterMask
                    // Soft mask edge so the rounded corners stay anti-aliased.
                    maskThresholdMin: 0.5
                    maskSpreadAtMin: 1.0
                }

                Rectangle {
                    anchors.fill: parent
                    color: Theme.surface
                }

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

            // Rounded-rect alpha mask for `posterSurface` (rendered to a layer, not
            // drawn directly); its rounded corners become the poster's corners.
            Item {
                id: posterMask
                anchors.fill: parent
                visible: false
                layer.enabled: true

                Rectangle {
                    anchors.fill: parent
                    radius: Units.radiusMD
                }
            }
        }

        // === Caption ===
        ColumnLayout {
            id: captionCol
            visible: root.showCaption
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
            InputMode.pointerMoved(p.x, p.y);
        }
        onClicked: {
            InputMode.enterMouseMode();
            root.forceActiveFocus();
            root.activated();
        }
    }

    Keys.onReturnPressed: root.activated()
    Keys.onEnterPressed: root.activated()
}
