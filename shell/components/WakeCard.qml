import QtQuick
import QtQuick.Layouts

// Single "Wake <host>" card shown in place of the Steam poster row when the
// streaming host is unavailable (the `steam-library` ServiceMonitor status is
// NOT "ok"). Activating it (A/Return or click) emits `activated`, which the host
// view turns into a `wol <host>` IPC command + a fast availability re-poll.
//
// Controller-focusable like SteamCard: a crimson FocusFrame ring on focus, an
// ember power glyph, and a "Wake <host>" caption. A FocusScope so it slots into
// the SteamLibraryView focus contract (focusFirstChild forces focus onto it).
FocusScope {
    id: root

    // The streaming-target host to wake (IP/hostname). Shown in the caption.
    property string host: ""
    // True briefly after activation while the wake packet is in flight / the
    // shell is fast-polling for the host to come back. Surfaces a "Waking…" label
    // so a second press reads as a no-op rather than a dead button.
    property bool waking: false

    property int cardWidth: Math.round(Theme.cardWidth * 0.62)
    property int cardHeight: Math.round(cardWidth * 1.5)

    // Home-tile focus chain neighbours (mirrors FilterChips/NavigableRow): Up/Down
    // walk these via forceActiveFocus so the wake card slots into the same
    // ordered-region navigation as the poster row it replaces.
    property var previousRow: null
    property var nextRow: null

    signal activated
    signal escaped

    implicitWidth: cardWidth
    implicitHeight: cardHeight

    activeFocusOnTab: true

    readonly property bool isFocused: (activeFocus && !InputMode.mouseMode) || (mouseArea.containsMouse && InputMode.mouseMode)

    Accessible.role: Accessible.Button
    Accessible.name: root.waking ? "Waking " + root.host : "Wake " + root.host
    Accessible.onPressAction: root.activated()

    z: root.isFocused ? 10 : 0

    // Mirror SteamCard: when leaving mouse-mode with the pointer over the card,
    // claim controller focus so the crimson ring lands where the cursor was.
    Connections {
        target: Theme
        function onMouseModeChanged() {
            if (!InputMode.mouseMode && mouseArea.containsMouse)
                root.forceActiveFocus();
        }
    }

    FocusFrame {
        id: frame
        anchors.fill: parent
        focused: root.isFocused
        radius: Units.radiusMD

        Rectangle {
            anchors.fill: parent
            radius: Units.radiusMD
            color: Theme.surface

            ColumnLayout {
                anchors.centerIn: parent
                width: parent.width - Units.spacingXL * 2
                spacing: Units.spacingMD

                // Ember power glyph — distinct from the crimson focus ring, so
                // "wake action" never reads as "focused".
                Text {
                    Layout.alignment: Qt.AlignHCenter
                    text: "⏻" // ⏻ power symbol
                    font.pixelSize: Units.iconSizeLG
                    color: Theme.ember
                    Accessible.ignored: true
                }

                Text {
                    Layout.fillWidth: true
                    horizontalAlignment: Text.AlignHCenter
                    text: root.waking ? "Waking…" : "Wake"
                    font.pixelSize: Theme.fontBody
                    font.bold: true
                    color: Theme.textPrimary
                    elide: Text.ElideRight
                }

                Text {
                    Layout.fillWidth: true
                    horizontalAlignment: Text.AlignHCenter
                    visible: root.host !== ""
                    text: root.host
                    font.pixelSize: Theme.fontSmall
                    color: Theme.textSecondary
                    elide: Text.ElideRight
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

    // Up/Down walk the previousRow/nextRow chains (skipping hidden rows), exactly
    // like FilterChips, so vertical navigation works while the wake card stands in
    // for the poster row. B/Escape bubbles up as `escaped`.
    Keys.onUpPressed: event => {
        InputMode.exitMouseMode();
        var up = root.previousRow;
        while (up) {
            if (up.visible) {
                up.forceActiveFocus();
                event.accepted = true;
                break;
            }
            up = (up.previousRow !== undefined) ? up.previousRow : null;
        }
    }
    Keys.onDownPressed: event => {
        InputMode.exitMouseMode();
        var dn = root.nextRow;
        while (dn) {
            if (dn.visible) {
                dn.forceActiveFocus();
                event.accepted = true;
                break;
            }
            dn = (dn.nextRow !== undefined) ? dn.nextRow : null;
        }
    }
    Keys.onEscapePressed: event => {
        root.escaped();
        event.accepted = true;
    }
}
