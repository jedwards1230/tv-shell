import QtQuick
import QtQuick.Layouts

// Portrait poster card for the Steam widget (Recently Played / Library). Renders
// the Steam CDN library poster (`library_600x900.jpg`) with an automatic
// fallback to the 16:9 `header.jpg` when the portrait art 404s (older titles
// often lack the portrait asset). Controller-focusable like PlexCard/AppCard: a
// crimson FocusFrame ring on focus, `activated` on A/Return (the host launches
// the game over the stream).
Item {
    id: root

    property string title: ""
    // Portrait library poster URL (built daemon-side off the appid).
    property string art: ""
    // 16:9 header fallback URL — used when the portrait poster fails to load.
    property string headerArt: ""

    // Poster-only mode (small size): hide the title caption band so the card
    // collapses to just the poster. The host row sizes itself to match.
    property bool showCaption: true

    // True when this game is the one currently running on the host (from the
    // daemon's `runningAppid`, NOT a locally-tracked tap). Shows an ember running
    // dot inline with the title (matching the recent-apps row / BaseCard) so the
    // user sees what's live regardless of which client started it.
    property bool playing: false

    // Poster geometry (set by the host row so all cards match).
    property int posterWidth: Math.round(Theme.cardWidth * 0.62)
    property int posterHeight: Math.round(posterWidth * 1.5)

    readonly property bool isFocused: (activeFocus && !Theme.mouseMode) || (mouseArea.containsMouse && Theme.mouseMode)

    // Art source with header.jpg fallback: start on the portrait poster; on a
    // load error swap to the header image; if that also fails, the letter-initial
    // placeholder shows through.
    property string _artSource: root.art
    onArtChanged: root._artSource = root.art

    signal activated

    width: posterWidth
    height: posterHeight + (root.showCaption ? captionCol.implicitHeight + Units.spacingSM : 0)

    Accessible.role: Accessible.Button
    Accessible.name: root.title
    Accessible.description: root.playing ? "Running" : ""
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
                    source: root._artSource
                    fillMode: Image.PreserveAspectCrop
                    asynchronous: true
                    cache: true
                    visible: status === Image.Ready
                    // Fallback to the header image once, then give up to the
                    // letter placeholder. Guard against a loop by only swapping
                    // when we're still on the portrait art.
                    onStatusChanged: {
                        if (status === Image.Error && root._artSource === root.art && root.headerArt !== "")
                            root._artSource = root.headerArt;
                    }
                }

                // Fallback while loading / when art is missing: the title initial
                // (or a controller glyph when there's no title yet).
                Text {
                    anchors.centerIn: parent
                    visible: poster.status !== Image.Ready
                    text: root.title !== "" ? root.title.charAt(0).toUpperCase() : "\u{1F3AE}"
                    font.pixelSize: Units.iconSizeLG
                    font.bold: true
                    color: Theme.textMuted
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

            RowLayout {
                Layout.fillWidth: true
                spacing: 6

                // Per-game running indicator — ember dot beside the title,
                // matching the recent-apps row (BaseCard). Ember (#e06236) is
                // distinct from the crimson focus ring, so "running" is never
                // confused with "focused"; dual cue (color + circular shape) is
                // colourblind-safe. Collapses out of the row when this game isn't
                // the one live on the host. This is the per-game signal, separate
                // from the header's session indicator (any stream active at all).
                Rectangle {
                    visible: root.playing
                    Layout.alignment: Qt.AlignVCenter
                    implicitWidth: 8
                    implicitHeight: 8
                    radius: 4
                    color: Theme.ember
                    Accessible.ignored: true
                }

                MarqueeText {
                    Layout.fillWidth: true
                    Layout.preferredHeight: Theme.fontSmall * 1.3
                    animate: root.isFocused
                    text: root.title
                    font.pixelSize: Theme.fontSmall
                    font.bold: true
                    color: Theme.textPrimary
                }
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
