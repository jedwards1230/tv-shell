import QtQuick
import QtQuick.Layouts
import "../lib"
import "../../components"

// Home-screen Steam Remote Play widget — a single focus-stop launcher card.
//
// This is a LAUNCHER only, not a stream: activating it (A / Return) emits
// `launchRequested`; HomeScreen routes that to a LOCAL app launch of Steam Big
// Picture (window class `steam`), which lands the shell in `appRunning`. Native
// Steam Remote Play is not scriptable per-game (Steam's own Big Picture UI owns
// host/game selection + the Stream button), so the widget just opens Steam RP
// and hands off. Because the launched window is class `steam`, the existing
// class-agnostic kiosk-fullscreen enforcement and the StreamAudioMuter
// (mute-on-background, streamClasses: ["steam"]) cover it for free — this widget
// owns no stream, no shell state, and no provider.
//
// Ships DISABLED by default (WidgetManifests `defaultEnabled: false`); the user
// opts in from the Widgets app. Single-stop: it leaves firstRow/lastRow null so
// WidgetHost targets the widget itself in the vertical focus chain, and overrides
// focusFirstChild to seat focus on the one card.
Widget {
    id: root

    // Default render size; the framework binds this to widgets.steamrp.size.
    size: "medium"

    // Emitted on A / Return / click. HomeScreen turns this into the local Steam
    // Big Picture launch (see its "Steam Remote Play" wiring block).
    signal launchRequested

    // Ask the home Flickable to scroll this widget into view when it gains focus
    // (e.g. arriving from the widget above while below the fold). Mirrors
    // AppsWidget/NavigableRow so focus-follow scrolling works for this card too.
    signal ensureVisibleRequested(var item)

    // small = compact single-line row; medium = row with a subtitle line.
    readonly property bool _compact: root.size === "small"

    wantVisible: root.widgetEnabled
    implicitWidth: card.implicitWidth
    implicitHeight: root.wantVisible ? card.implicitHeight : 0

    // === Home-tile focus contract (single stop) ===
    canFocus: visible

    function focusFirstChild() {
        if (!root.canFocus)
            return false;
        card.forceActiveFocus();
        return true;
    }

    // === Vertical focus handoff (mirrors NavigableRow._navigateUp/_navigateDown) ===
    // WidgetHost wires previousRow/nextRow to the nearest focusable neighbour (or a
    // terminal anchor). The single card below has no ListView to consume Up/Down, so
    // it forwards vertical nav here — walking the chain and forceActiveFocus-ing the
    // first focusable neighbour, skipping any that report unfocusable and no-oping at
    // the ends. Copied faithfully from NavigableRow so the handoff is identical.
    function _focusable(item) {
        return item.canFocus !== undefined ? item.canFocus : item.visible;
    }

    function _navigateUp() {
        var target = root.previousRow;
        while (target) {
            if (root._focusable(target)) {
                target.forceActiveFocus();
                return;
            }
            target = (target.previousRow !== undefined) ? target.previousRow : null;
        }
    }

    function _navigateDown() {
        var target = root.nextRow;
        while (target) {
            if (root._focusable(target)) {
                target.forceActiveFocus();
                return;
            }
            target = (target.nextRow !== undefined) ? target.nextRow : null;
        }
    }

    FocusFrame {
        id: card
        focus: true
        // Bounded, left-aligned launcher card — NOT full-bleed. A full-width card
        // can't take FocusFrame's 1.06 focus-pop without scaling offscreen, and it
        // read as too wide next to the other widgets' card rows. Cap the width and
        // clamp to the widget slot on narrow displays.
        width: Math.min(root.width, Units.gridUnit * 24)
        implicitHeight: root._compact ? Math.round(Units.gridUnit * 3.4) : Math.round(Units.gridUnit * 4.6)
        height: implicitHeight
        focused: (card.activeFocus && !InputMode.mouseMode) || (cardMouse.containsMouse && InputMode.mouseMode)

        // Scroll into view on focus so the home Flickable follows focus onto this
        // widget when it is below the fold (same as the other widgets).
        onActiveFocusChanged: if (card.activeFocus)
            root.ensureVisibleRequested(card)

        Accessible.role: Accessible.Button
        Accessible.name: "Steam Remote Play"
        Accessible.description: "Launch Steam Big Picture"
        Accessible.focusable: true
        Accessible.onPressAction: root.launchRequested()

        MouseArea {
            id: cardMouse
            anchors.fill: parent
            hoverEnabled: true
            cursorShape: Qt.PointingHandCursor
            // Match BaseCard: flip to mouse mode only on a genuine pointer move
            // (global-coords delta) so a card scrolling under a stationary cursor
            // can't hijack controller-nav focus.
            onPositionChanged: mouse => {
                let p = mapToItem(null, mouse.x, mouse.y);
                InputMode.pointerMoved(p.x, p.y);
            }
            onClicked: {
                InputMode.enterMouseMode();
                card.forceActiveFocus();
                root.launchRequested();
            }
        }

        RowLayout {
            anchors.fill: parent
            anchors.margins: Theme.padding
            spacing: Units.spacingLG

            // System Steam icon when the theme provides one, else a controller
            // glyph (the target device has no icon theme — see gotchas).
            Item {
                Layout.preferredWidth: Units.iconSizeLG
                Layout.preferredHeight: Units.iconSizeLG
                Layout.alignment: Qt.AlignVCenter

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
                    font.pixelSize: Math.round(Units.iconSizeLG * 0.7)
                    color: Theme.textPrimary
                }
            }

            ColumnLayout {
                Layout.fillWidth: true
                Layout.alignment: Qt.AlignVCenter
                spacing: Units.spacingXS

                Text {
                    Layout.fillWidth: true
                    text: "Steam Remote Play"
                    font.pixelSize: Theme.fontTitle
                    font.bold: true
                    color: Theme.textPrimary
                    elide: Text.ElideRight
                }
                Text {
                    Layout.fillWidth: true
                    visible: !root._compact
                    text: "Launch Steam Big Picture"
                    font.pixelSize: Theme.fontSmall
                    color: Theme.textSecondary
                    elide: Text.ElideRight
                }
            }

            Text {
                Layout.alignment: Qt.AlignVCenter
                text: "A: Launch"
                font.pixelSize: Theme.fontSmall
                color: Theme.textSecondary
            }
        }

        Keys.onReturnPressed: root.launchRequested()
        Keys.onEnterPressed: root.launchRequested()
        Keys.onEscapePressed: root.escaped()
        // Forward vertical nav to the widget's focus-chain neighbours so the card
        // isn't a dead-end (Down → nextRow, Up → previousRow), walking to the first
        // focusable neighbour exactly like every NavigableRow-based widget.
        Keys.onUpPressed: {
            InputMode.exitMouseMode();
            root._navigateUp();
        }
        Keys.onDownPressed: {
            InputMode.exitMouseMode();
            root._navigateDown();
        }
    }
}
