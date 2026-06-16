import QtQuick
import QtQuick.Layouts
import "lib"

// Unified poster tile for the home Continue + New rails (#249). Generalizes
// PlexCard: renders real artwork when present (Plex posters today; #114 box art
// later), else falls back to a centered Freedesktop app icon → letter initial
// inside the SAME poster frame, so a mixed rail (apps + media) visually rhymes.
// Carries the running ember-dot (merged-row apps) and the resume progress bar
// (Plex On Deck). Controller-focusable like PlexCard/AppCard.
Item {
    id: root

    // Normalized entry: { kind, title, subtitle, art, iconSource, progress, running, … }
    // Extra routing fields (address/exec/windowClass/…) are read by the host's
    // delegate onActivated; this card only consumes the display fields below.
    required property var entry

    readonly property string title: entry && entry.title ? entry.title : ""
    readonly property string subtitle: entry && entry.subtitle ? entry.subtitle : ""
    readonly property string art: entry && entry.art ? entry.art : ""
    readonly property real progress: entry && entry.progress ? entry.progress : 0
    readonly property bool running: !!(entry && entry.running)

    // Poster geometry (set by the host row so all cards match PlexCard).
    property int posterWidth: Math.round(Theme.cardWidth * 0.62)
    property int posterHeight: Math.round(posterWidth * 1.5)

    readonly property bool isFocused: (activeFocus && !Theme.mouseMode) || (mouseArea.containsMouse && Theme.mouseMode)

    signal activated

    width: posterWidth
    height: posterHeight + captionCol.implicitHeight + Units.spacingSM

    Accessible.role: Accessible.Button
    Accessible.name: root.title + (root.subtitle !== "" ? ", " + root.subtitle : "")
    Accessible.description: root.running ? "Running" : ""
    Accessible.focusable: true
    Accessible.onPressAction: root.activated()

    // Imperative icon refresh — clear before re-assign so a recycled ListView
    // delegate never keeps the previous entry's texture (#194). Only art-less
    // entries drive the AppIcon; entries with real art leave it cleared so the
    // letter fallback shows only briefly while the poster loads.
    function _refreshIcon() {
        appIcon.iconSource = "";
        if (root.art === "" && root.entry && root.entry.iconSource)
            appIcon.iconSource = root.entry.iconSource;
    }
    onEntryChanged: _refreshIcon()
    Component.onCompleted: _refreshIcon()

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

                // Real artwork (Plex poster / future box art).
                Image {
                    id: poster
                    anchors.fill: parent
                    source: root.art
                    fillMode: Image.PreserveAspectCrop
                    asynchronous: true
                    cache: true
                    visible: root.art !== "" && status === Image.Ready
                }

                // Art-less fallback: centered app icon → letter initial. Hidden
                // once real art is ready.
                AppIcon {
                    id: appIcon
                    anchors.centerIn: parent
                    width: Units.iconSizeXL
                    height: Units.iconSizeXL
                    iconSize: Units.iconSizeXL
                    fallbackText: root.title
                    visible: !poster.visible
                }

                // Running badge — ember dot, top-left. Dual cue (color + shape),
                // distinct from the crimson focus ring; outlined so it reads over
                // any artwork.
                Rectangle {
                    visible: root.running
                    anchors.left: parent.left
                    anchors.top: parent.top
                    anchors.margins: Units.spacingSM
                    width: 14
                    height: 14
                    radius: 7
                    color: Theme.ember
                    border.width: 2
                    border.color: Qt.rgba(0, 0, 0, 0.5)
                    Accessible.ignored: true
                }

                // === Resume progress bar ===
                // Bottom-anchored ember fill; only for partially-watched items.
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
