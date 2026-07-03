import QtQuick
import QtQuick.Layouts
import "../lib"
import "../../components"

// Home-screen Steam Remote Play widget — a single focus-stop launcher TILE,
// shaped and sized like the game poster cards (mirrors WakeCard). Sizing it like
// the other cards is deliberate: FocusFrame pops 1.06x from the CENTER on focus,
// so a card-sized tile grows only ~one margin's worth per side and stays on
// screen, whereas a full-width bar spilled past the screen edges.
//
// LAUNCHER only, not a stream: activating it (A / Return / click) emits
// `launchRequested`; HomeScreen routes that to a LOCAL launch of Steam Big
// Picture (window class `steam`), landing the shell in `appRunning`. Native Steam
// Remote Play is not scriptable per-game (Steam's own UI owns host/game + Stream),
// so this opens Steam RP and hands off. Because the window is class `steam`, the
// class-agnostic kiosk fullscreen and the StreamAudioMuter (mute-on-background,
// streamClasses: ["steam"]) cover it for free — no stream, shell state, or provider.
//
// Ships DISABLED by default (WidgetManifests defaultEnabled: false). Single-stop:
// leaves firstRow/lastRow null, so it inherits the Widget base's contract wholesale
// — Up/Down forward to previousRow/nextRow via the shared focusChain helper, and
// ensureVisibleRequested (declared once on the base, forwarded by WidgetHost) is
// auto-emitted by the base on focus entry so the home Flickable scrolls to it when
// it is below the fold. This widget adds only its own launchRequested signal.
Widget {
    id: root

    // Emitted on A / Return / click → HomeScreen launches Steam Big Picture.
    signal launchRequested

    // Poster-tile dimensions — same proportion as the game cards / WakeCard, so
    // the FocusFrame focus-pop grows ~one margin per side and never runs offscreen.
    readonly property int _tileWidth: Math.round(Theme.cardWidth * (root.size === "small" ? 0.5 : 0.62))
    readonly property int _tileHeight: Math.round(_tileWidth * 1.5)

    wantVisible: root.widgetEnabled
    implicitWidth: tile.implicitWidth
    implicitHeight: root.wantVisible ? tile.implicitHeight : 0

    // === Home-tile focus contract (single stop) ===
    canFocus: visible

    function focusFirstChild() {
        if (!root.canFocus)
            return false;
        tile.forceActiveFocus();
        return true;
    }

    // Vertical focus handoff (_navigateUp/_navigateDown) is inherited from the
    // Widget base's shared focusChain helper — no local copy.

    FocusFrame {
        id: tile
        focus: true
        // Left-aligned tile with a small inset — matches where the row cards start
        // (NavigableRow's leading margin) and guarantees left-edge cushion for the
        // centred focus-pop.
        x: Units.spacingSM
        implicitWidth: root._tileWidth
        implicitHeight: root._tileHeight
        width: implicitWidth
        height: implicitHeight
        radius: Units.radiusMD
        focused: (tile.activeFocus && !InputMode.mouseMode) || (tileMouse.containsMouse && InputMode.mouseMode)

        // Scroll-into-view on focus is handled by the Widget base's single-stop
        // auto-emit of ensureVisibleRequested (firstRow is null here), so no manual
        // emit — that would double-fire with the base.

        Accessible.role: Accessible.Button
        Accessible.name: "Steam Remote Play"
        Accessible.description: "Open Steam"
        Accessible.focusable: true
        Accessible.onPressAction: root.launchRequested()

        Rectangle {
            anchors.fill: parent
            radius: Units.radiusMD
            color: Theme.surface

            ColumnLayout {
                anchors.centerIn: parent
                width: parent.width - Units.spacingXL * 2
                spacing: Units.spacingMD

                // System Steam icon when the theme provides one, else a controller
                // glyph (the target device has no icon theme — see gotchas).
                Item {
                    Layout.alignment: Qt.AlignHCenter
                    Layout.preferredWidth: Units.iconSizeLG * 2
                    Layout.preferredHeight: Units.iconSizeLG * 2

                    Image {
                        id: steamIcon
                        anchors.fill: parent
                        source: "image://icon/steam"
                        fillMode: Image.PreserveAspectFit
                        visible: status === Image.Ready
                    }
                    Text {
                        anchors.centerIn: parent
                        visible: steamIcon.status !== Image.Ready
                        text: "\u{1F3AE}"
                        font.pixelSize: Math.round(Units.iconSizeLG * 1.4)
                        color: Theme.textPrimary
                    }
                }

                Text {
                    Layout.fillWidth: true
                    horizontalAlignment: Text.AlignHCenter
                    text: "Steam Remote Play"
                    font.pixelSize: Theme.fontBody
                    font.bold: true
                    color: Theme.textPrimary
                    wrapMode: Text.WordWrap
                }

                Text {
                    Layout.fillWidth: true
                    horizontalAlignment: Text.AlignHCenter
                    text: "Open Steam"
                    font.pixelSize: Theme.fontSmall
                    color: Theme.textSecondary
                    wrapMode: Text.WordWrap
                }
            }
        }

        MouseArea {
            id: tileMouse
            anchors.fill: parent
            hoverEnabled: true
            cursorShape: Qt.PointingHandCursor
            // Match the cards: flip to mouse mode only on a genuine pointer move.
            onPositionChanged: mouse => {
                let p = mapToItem(null, mouse.x, mouse.y);
                InputMode.pointerMoved(p.x, p.y);
            }
            onClicked: {
                InputMode.enterMouseMode();
                tile.forceActiveFocus();
                root.launchRequested();
            }
        }

        Keys.onReturnPressed: {
            InputMode.exitMouseMode();
            root.launchRequested();
        }
        Keys.onEnterPressed: {
            InputMode.exitMouseMode();
            root.launchRequested();
        }
        // Accept the event only when the inherited chain walk moved focus; a failed
        // hand-off leaves it unaccepted so the key can bubble (matches NavigableRow).
        Keys.onUpPressed: event => {
            InputMode.exitMouseMode();
            event.accepted = root._navigateUp();
        }
        Keys.onDownPressed: event => {
            InputMode.exitMouseMode();
            event.accepted = root._navigateDown();
        }
        Keys.onEscapePressed: event => {
            InputMode.exitMouseMode();
            root.escaped();
            event.accepted = true;
        }
    }
}
