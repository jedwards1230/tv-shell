import QtQuick
import QtQuick.Layouts
import Quickshell.Services.Mpris

// Slim now-playing transport strip for the redesigned home screen (#249). A
// lighter re-presentation of MediaWidget: the same MPRIS player selection,
// capability guards, transport actions, and home-tile focus contract, rendered
// as a single compact row (small art thumb · "title · artist" · Open + Prev /
// Play-Pause / Next) instead of the full-width card with the big art tile,
// "Now Playing" label, and progress bar. Sits between the hero and the Continue
// rail and collapses to zero height when no MPRIS player is on the bus.
FocusScope {
    id: root

    // Vertical focus chain neighbours (set by the host).
    property var previousRow: null
    property var nextRow: null

    // Home-screen widget toggle (Settings ▸ Widgets ▸ Spotify/media).
    property bool widgetEnabled: true

    signal escaped
    signal contextRequested
    signal openAppRequested(string desktopEntry, string identity)

    // === Active player selection (mirrors MediaWidget) ===
    readonly property var _players: Mpris.players ? Mpris.players.values : []
    readonly property var player: {
        let list = root._players;
        if (!list || list.length === 0)
            return null;
        for (let i = 0; i < list.length; i++) {
            if (list[i] && list[i].isPlaying)
                return list[i];
        }
        return list[0];
    }

    readonly property bool hasPlayer: player !== null
    readonly property bool isPlaying: hasPlayer && player.isPlaying

    // === Home-tile focus contract ===
    readonly property bool regionFocused: activeFocus

    function focusFirstChild() {
        if (!visible)
            return false;
        forceActiveFocus();
        return true;
    }

    readonly property string playerDesktopEntry: hasPlayer && player.desktopEntry ? player.desktopEntry : ""
    readonly property string playerIdentity: hasPlayer && player.identity ? player.identity : ""

    // Focus index: 0 = Open app, 1 = Prev, 2 = Play/Pause, 3 = Next.
    property int _btn: 2

    readonly property bool _canPrev: hasPlayer && player.canGoPrevious
    readonly property bool _canNext: hasPlayer && player.canGoNext
    readonly property bool _canToggle: hasPlayer && (player.canTogglePlaying || player.canPlay || player.canPause)
    readonly property bool _canOpen: hasPlayer && (playerDesktopEntry !== "" || playerIdentity !== "")

    readonly property bool _shown: hasPlayer && widgetEnabled
    implicitHeight: _shown ? card.implicitHeight : 0
    visible: _shown
    height: implicitHeight

    // Skip-track glyph (two triangles + bar), centred by construction — the same
    // Canvas approach MediaWidget uses because the Unicode glyphs won't centre.
    function _paintSkip(ctx, w, h, color, dir) {
        ctx.reset();
        ctx.clearRect(0, 0, w, h);
        ctx.fillStyle = color;
        if (dir < 0) {
            ctx.translate(w, 0);
            ctx.scale(-1, 1);
        }
        const triW = w * 0.36;
        const triH = h * 0.78;
        const barW = w * 0.13;
        const gap = w * 0.04;
        const totalW = triW * 2 + gap * 2 + barW;
        let x = (w - totalW) / 2;
        const ty = (h - triH) / 2;
        const tri = function (tx) {
            ctx.beginPath();
            ctx.moveTo(tx, ty);
            ctx.lineTo(tx, ty + triH);
            ctx.lineTo(tx + triW, ty + triH / 2);
            ctx.closePath();
            ctx.fill();
        };
        tri(x);
        x += triW + gap;
        tri(x);
        x += triW + gap;
        ctx.fillRect(x, ty, barW, triH);
    }

    function _activate() {
        if (!root.hasPlayer)
            return;
        switch (root._btn) {
        case 0:
            if (root._canOpen)
                root.openAppRequested(root.playerDesktopEntry, root.playerIdentity);
            break;
        case 1:
            if (root._canPrev)
                root.player.previous();
            break;
        case 2:
            if (root._canToggle)
                root.player.togglePlaying();
            break;
        case 3:
            if (root._canNext)
                root.player.next();
            break;
        }
    }

    Keys.onPressed: event => {
        switch (event.key) {
        case Qt.Key_Left:
            Theme.exitMouseMode();
            if (root._btn > 0)
                root._btn--;
            event.accepted = true;
            break;
        case Qt.Key_Right:
            Theme.exitMouseMode();
            if (root._btn < 3)
                root._btn++;
            event.accepted = true;
            break;
        case Qt.Key_Up:
            Theme.exitMouseMode();
            if (root.previousRow) {
                root.previousRow.forceActiveFocus();
                event.accepted = true;
            }
            break;
        case Qt.Key_Down:
            Theme.exitMouseMode();
            if (root.nextRow) {
                root.nextRow.forceActiveFocus();
                event.accepted = true;
            }
            break;
        case Qt.Key_Return:
        case Qt.Key_Enter:
            Theme.exitMouseMode();
            root._activate();
            event.accepted = true;
            break;
        case Qt.Key_Tab:
            Theme.exitMouseMode();
            if (root.hasPlayer)
                root.contextRequested();
            event.accepted = true;
            break;
        case Qt.Key_Escape:
        case Qt.Key_B:
            if (event.key === Qt.Key_B && event.modifiers)
                break;
            root.escaped();
            event.accepted = true;
            break;
        }
    }

    FocusFrame {
        id: card
        width: parent.width
        implicitHeight: cardRow.implicitHeight + Units.spacingMD * 2
        focused: root.activeFocus && !Theme.mouseMode
        scaleEnabled: false

        RowLayout {
            id: cardRow
            anchors {
                fill: parent
                margins: Units.spacingMD
            }
            spacing: Units.spacingLG

            // === Album art thumb ===
            Rectangle {
                Layout.preferredWidth: Units.iconSizeLG
                Layout.preferredHeight: Units.iconSizeLG
                Layout.alignment: Qt.AlignVCenter
                radius: Units.radiusSM
                color: Theme.surface
                clip: true

                Image {
                    id: artImage
                    anchors.fill: parent
                    source: root.hasPlayer && root.player.trackArtUrl ? root.player.trackArtUrl : ""
                    fillMode: Image.PreserveAspectCrop
                    asynchronous: true
                    cache: true
                    visible: status === Image.Ready
                }

                Text {
                    anchors.centerIn: parent
                    visible: artImage.status !== Image.Ready
                    text: "♪" // musical note
                    font.pixelSize: Theme.fontTitle
                    color: Theme.textMuted
                }
            }

            // === Track line (title · artist) ===
            MarqueeText {
                Layout.fillWidth: true
                Layout.alignment: Qt.AlignVCenter
                Layout.preferredHeight: Theme.fontBody * 1.4
                animate: root.activeFocus
                text: {
                    if (!root.hasPlayer)
                        return "";
                    let t = root.player.trackTitle || "Unknown title";
                    let a = root.player.trackArtist || "";
                    return a !== "" ? t + "  •  " + a : t;
                }
                font.pixelSize: Theme.fontBody
                font.bold: true
                color: Theme.textPrimary
            }

            // === Transport controls ===
            RowLayout {
                Layout.alignment: Qt.AlignVCenter
                spacing: Units.spacingMD

                // Open app (index 0) — compact pill.
                Rectangle {
                    id: openButton
                    Layout.preferredHeight: Units.gridUnit * 2.0
                    Layout.preferredWidth: openRow.implicitWidth + Units.spacingMD * 2
                    Layout.alignment: Qt.AlignVCenter
                    radius: height / 2
                    readonly property bool focused: root.activeFocus && !Theme.mouseMode && root._btn === 0
                    color: focused ? Theme.crimson : Theme.surface
                    opacity: root._canOpen ? 1.0 : 0.35
                    border.width: focused ? Units.borderMedium : Units.borderThin
                    border.color: focused ? Theme.focusBorder : Theme.surfaceBorder
                    Accessible.role: Accessible.Button
                    Accessible.name: "Open " + (root.playerIdentity || "app")
                    Accessible.focusable: true
                    Accessible.onPressAction: {
                        if (root._canOpen)
                            root.openAppRequested(root.playerDesktopEntry, root.playerIdentity);
                    }
                    Behavior on color {
                        ColorAnimation {
                            duration: 150
                        }
                    }

                    RowLayout {
                        id: openRow
                        anchors.centerIn: parent
                        spacing: Units.spacingXS

                        Text {
                            text: "⛶" // open-fullscreen glyph
                            font.pixelSize: Theme.fontBody
                            color: openButton.focused ? Theme.textOnDark : Theme.textPrimary
                        }
                        Text {
                            text: "Open"
                            font.pixelSize: Theme.fontCaption
                            font.bold: true
                            color: openButton.focused ? Theme.textOnDark : Theme.textPrimary
                        }
                    }

                    MouseArea {
                        anchors.fill: parent
                        hoverEnabled: true
                        cursorShape: Qt.PointingHandCursor
                        onPositionChanged: mouse => {
                            let p = mapToItem(null, mouse.x, mouse.y);
                            Theme.pointerMoved(p.x, p.y);
                        }
                        onClicked: {
                            Theme.enterMouseMode();
                            root._btn = 0;
                            if (root._canOpen)
                                root.openAppRequested(root.playerDesktopEntry, root.playerIdentity);
                        }
                    }
                }

                // Prev (index 1)
                Rectangle {
                    Layout.preferredWidth: Units.gridUnit * 2.0
                    Layout.preferredHeight: Units.gridUnit * 2.0
                    radius: width / 2
                    readonly property bool focused: root.activeFocus && !Theme.mouseMode && root._btn === 1
                    color: focused ? Theme.surfaceHover : "transparent"
                    opacity: root._canPrev ? 1.0 : 0.35
                    border.width: focused ? Units.borderMedium : 0
                    border.color: Theme.focusBorder
                    Accessible.role: Accessible.Button
                    Accessible.name: "Previous track"
                    Accessible.focusable: true
                    Accessible.onPressAction: {
                        if (root._canPrev)
                            root.player.previous();
                    }
                    Behavior on color {
                        ColorAnimation {
                            duration: 150
                        }
                    }

                    Canvas {
                        anchors.centerIn: parent
                        width: parent.width * 0.4
                        height: parent.height * 0.4
                        antialiasing: true
                        property color iconColor: Theme.textPrimary
                        onIconColorChanged: requestPaint()
                        onWidthChanged: requestPaint()
                        onHeightChanged: requestPaint()
                        onPaint: root._paintSkip(getContext("2d"), width, height, iconColor, -1)
                    }

                    MouseArea {
                        anchors.fill: parent
                        hoverEnabled: true
                        cursorShape: Qt.PointingHandCursor
                        onPositionChanged: mouse => {
                            let p = mapToItem(null, mouse.x, mouse.y);
                            Theme.pointerMoved(p.x, p.y);
                        }
                        onClicked: {
                            Theme.enterMouseMode();
                            root._btn = 1;
                            if (root._canPrev)
                                root.player.previous();
                        }
                    }
                }

                // Play / Pause (index 2)
                Rectangle {
                    Layout.preferredWidth: Units.gridUnit * 2.4
                    Layout.preferredHeight: Units.gridUnit * 2.4
                    radius: width / 2
                    readonly property bool focused: root.activeFocus && !Theme.mouseMode && root._btn === 2
                    color: focused ? Theme.crimson : Theme.surface
                    opacity: root._canToggle ? 1.0 : 0.35
                    border.width: focused ? Units.borderMedium : Units.borderThin
                    border.color: focused ? Theme.focusBorder : Theme.surfaceBorder
                    Accessible.role: Accessible.Button
                    Accessible.name: root.isPlaying ? "Pause" : "Play"
                    Accessible.focusable: true
                    Accessible.onPressAction: {
                        if (root._canToggle)
                            root.player.togglePlaying();
                    }
                    Behavior on color {
                        ColorAnimation {
                            duration: 150
                        }
                    }

                    Canvas {
                        id: playPauseIcon
                        anchors.centerIn: parent
                        width: parent.width * 0.4
                        height: parent.height * 0.4
                        antialiasing: true
                        property color iconColor: parent.focused ? Theme.textOnDark : Theme.textPrimary
                        property bool playing: root.isPlaying
                        onIconColorChanged: requestPaint()
                        onPlayingChanged: requestPaint()
                        onWidthChanged: requestPaint()
                        onHeightChanged: requestPaint()
                        onPaint: {
                            const ctx = getContext("2d");
                            ctx.reset();
                            ctx.clearRect(0, 0, width, height);
                            ctx.fillStyle = iconColor;
                            const w = width;
                            const h = height;
                            if (playing) {
                                const barW = w * 0.30;
                                const gap = w * 0.16;
                                const x0 = (w - (barW * 2 + gap)) / 2;
                                const rad = barW * 0.28;
                                const bar = function (x) {
                                    ctx.beginPath();
                                    ctx.moveTo(x + rad, 0);
                                    ctx.arcTo(x + barW, 0, x + barW, h, rad);
                                    ctx.arcTo(x + barW, h, x, h, rad);
                                    ctx.arcTo(x, h, x, 0, rad);
                                    ctx.arcTo(x, 0, x + barW, 0, rad);
                                    ctx.closePath();
                                    ctx.fill();
                                };
                                bar(x0);
                                bar(x0 + barW + gap);
                            } else {
                                const tw = w * 0.9;
                                const th = h * 0.98;
                                const bx = w / 2 - tw / 3;
                                const ty = (h - th) / 2;
                                ctx.beginPath();
                                ctx.moveTo(bx, ty);
                                ctx.lineTo(bx, ty + th);
                                ctx.lineTo(bx + tw, ty + th / 2);
                                ctx.closePath();
                                ctx.fill();
                            }
                        }
                    }

                    MouseArea {
                        anchors.fill: parent
                        hoverEnabled: true
                        cursorShape: Qt.PointingHandCursor
                        onPositionChanged: mouse => {
                            let p = mapToItem(null, mouse.x, mouse.y);
                            Theme.pointerMoved(p.x, p.y);
                        }
                        onClicked: {
                            Theme.enterMouseMode();
                            root._btn = 2;
                            if (root._canToggle)
                                root.player.togglePlaying();
                        }
                    }
                }

                // Next (index 3)
                Rectangle {
                    Layout.preferredWidth: Units.gridUnit * 2.0
                    Layout.preferredHeight: Units.gridUnit * 2.0
                    radius: width / 2
                    readonly property bool focused: root.activeFocus && !Theme.mouseMode && root._btn === 3
                    color: focused ? Theme.surfaceHover : "transparent"
                    opacity: root._canNext ? 1.0 : 0.35
                    border.width: focused ? Units.borderMedium : 0
                    border.color: Theme.focusBorder
                    Accessible.role: Accessible.Button
                    Accessible.name: "Next track"
                    Accessible.focusable: true
                    Accessible.onPressAction: {
                        if (root._canNext)
                            root.player.next();
                    }
                    Behavior on color {
                        ColorAnimation {
                            duration: 150
                        }
                    }

                    Canvas {
                        anchors.centerIn: parent
                        width: parent.width * 0.4
                        height: parent.height * 0.4
                        antialiasing: true
                        property color iconColor: Theme.textPrimary
                        onIconColorChanged: requestPaint()
                        onWidthChanged: requestPaint()
                        onHeightChanged: requestPaint()
                        onPaint: root._paintSkip(getContext("2d"), width, height, iconColor, 1)
                    }

                    MouseArea {
                        anchors.fill: parent
                        hoverEnabled: true
                        cursorShape: Qt.PointingHandCursor
                        onPositionChanged: mouse => {
                            let p = mapToItem(null, mouse.x, mouse.y);
                            Theme.pointerMoved(p.x, p.y);
                        }
                        onClicked: {
                            Theme.enterMouseMode();
                            root._btn = 3;
                            if (root._canNext)
                                root.player.next();
                        }
                    }
                }
            }
        }
    }
}
