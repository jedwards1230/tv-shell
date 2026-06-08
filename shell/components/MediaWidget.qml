import QtQuick
import QtQuick.Layouts
import Quickshell.Services.Mpris

// Now-playing media widget (#22). Surfaces the active MPRIS player — Spotify
// desktop, browsers, any spec-compliant player — with cover art, track
// metadata, a progress bar, and transport controls (prev / play-pause / next).
//
// MPRIS integration is via Quickshell's built-in `Quickshell.Services.Mpris`
// module: `Mpris.players` is a model of every player on the session D-Bus bus,
// and each `MprisPlayer` exposes trackTitle/trackArtist/trackAlbum/trackArtUrl,
// length/position, isPlaying + play()/pause()/togglePlaying()/next()/previous().
// We pick the first player that is actually playing, else the first available
// one, so the widget follows whatever is currently making sound.
//
// Controller model (mirrors the row idiom in HomeScreen): when focused, the
// transport strip owns Left/Right (move between Prev/Play/Next) and A/Return
// (activate). Up/Down hand focus to previousRow/nextRow so the widget slots
// into the home-screen vertical focus chain like any NavigableRow. B/Escape
// bubbles up (emit escaped) so Home's default-focus reset still works.
FocusScope {
    id: root

    // Vertical focus chain neighbours (set by the host, same contract as
    // NavigableRow). Either may be null.
    property var previousRow: null
    property var nextRow: null

    signal escaped

    // === Active player selection ===
    // Prefer a player that is currently playing; fall back to the first
    // available player (paused/stopped) so metadata still shows. null when no
    // MPRIS player is on the bus — the widget then collapses to zero height.
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

    // Transport focus index: 0 = Prev, 1 = Play/Pause, 2 = Next.
    property int _btn: 1

    // Capability guards — a player may not advertise every control.
    readonly property bool _canPrev: hasPlayer && player.canGoPrevious
    readonly property bool _canNext: hasPlayer && player.canGoNext
    readonly property bool _canToggle: hasPlayer && (player.canTogglePlaying || player.canPlay || player.canPause)

    implicitHeight: hasPlayer ? card.implicitHeight : 0
    visible: hasPlayer
    height: implicitHeight

    function _activate() {
        if (!root.hasPlayer)
            return;
        switch (root._btn) {
        case 0:
            if (root._canPrev)
                root.player.previous();
            break;
        case 1:
            if (root._canToggle)
                root.player.togglePlaying();
            break;
        case 2:
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
            if (root._btn < 2)
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
        case Qt.Key_Escape:
        case Qt.Key_B:
            if (event.key === Qt.Key_B && event.modifiers)
                break;
            root.escaped();
            event.accepted = true;
            break;
        }
    }

    Rectangle {
        id: card
        width: parent.width
        implicitHeight: cardRow.implicitHeight + Units.spacingLG * 2
        radius: Theme.cardRadius
        color: Theme.cardBackground
        border.width: root.activeFocus && !Theme.mouseMode ? Units.borderMedium : Units.borderThin
        border.color: root.activeFocus && !Theme.mouseMode ? Theme.focusBorder : Theme.surfaceBorder

        Behavior on border.color {
            ColorAnimation {
                duration: 150
            }
        }

        RowLayout {
            id: cardRow
            anchors {
                fill: parent
                margins: Units.spacingLG
            }
            spacing: Units.spacingXL

            // === Album art ===
            Rectangle {
                Layout.preferredWidth: Units.iconSizeXL
                Layout.preferredHeight: Units.iconSizeXL
                Layout.alignment: Qt.AlignVCenter
                radius: Units.radiusMD
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

                // Fallback glyph when no art is available.
                Text {
                    anchors.centerIn: parent
                    visible: artImage.status !== Image.Ready
                    text: "♪" // musical note
                    font.pixelSize: Units.iconSizeLG
                    color: Theme.textMuted
                }
            }

            // === Track metadata ===
            ColumnLayout {
                Layout.fillWidth: true
                Layout.alignment: Qt.AlignVCenter
                spacing: Units.spacingSM

                Text {
                    text: "Now Playing"
                    font.pixelSize: Theme.fontCaption
                    font.bold: true
                    color: Theme.textMuted
                }

                MarqueeText {
                    Layout.fillWidth: true
                    Layout.preferredHeight: Theme.fontTitle * 1.3
                    animate: root.activeFocus
                    text: root.hasPlayer && root.player.trackTitle ? root.player.trackTitle : "Unknown title"
                    font.pixelSize: Theme.fontTitle
                    font.bold: true
                    color: Theme.textPrimary
                }

                MarqueeText {
                    Layout.fillWidth: true
                    Layout.preferredHeight: Theme.fontBody * 1.3
                    animate: root.activeFocus
                    text: {
                        if (!root.hasPlayer)
                            return "";
                        let artist = root.player.trackArtist || "";
                        let album = root.player.trackAlbum || "";
                        if (artist && album)
                            return artist + "  •  " + album;
                        return artist || album;
                    }
                    font.pixelSize: Theme.fontBody
                    color: Theme.textSecondary
                }

                // === Progress bar ===
                // MprisPlayer.position does NOT advance on its own — reading it
                // only re-polls the underlying player when `positionChanged`
                // fires (which Quickshell emits on seek/state changes, e.g.
                // play/pause). The poll Timer below nudges it once a second
                // while playing so the fill tracks live. Guard against
                // length <= 0 (live streams / unknown duration) by collapsing
                // the fill to zero.
                Rectangle {
                    id: progressTrack
                    Layout.fillWidth: true
                    Layout.topMargin: Units.spacingXS
                    // Layout-managed height; matches the established bar
                    // pattern in VolumeOverlay (the inner fill reads
                    // parent.height).
                    height: Math.max(4, Units.gridUnit * 0.18)
                    radius: height / 2
                    color: Theme.surfaceHover
                    visible: root.hasPlayer

                    readonly property real _progress: {
                        if (!root.hasPlayer)
                            return 0;
                        let len = root.player.length;
                        if (!len || len <= 0)
                            return 0;
                        let p = root.player.position / len;
                        return Math.max(0, Math.min(1, p));
                    }

                    Rectangle {
                        width: parent.width * parent._progress
                        height: parent.height
                        radius: parent.radius
                        color: Theme.darkMode ? Theme.ember : Theme.navy

                        // Linear glide over the poll interval so the fill
                        // travels continuously between 1s samples instead of
                        // stepping. A seek/track-change is just a larger glide.
                        Behavior on width {
                            NumberAnimation {
                                duration: 1000
                                easing.type: Easing.Linear
                            }
                        }
                    }

                    // Force MprisPlayer.position to re-poll once a second while
                    // playing — without this the bound _progress only updates
                    // on play/pause (the only times Mpris re-emits position).
                    Timer {
                        interval: 1000
                        repeat: true
                        running: root.isPlaying
                        onTriggered: {
                            if (root.hasPlayer)
                                root.player.positionChanged();
                        }
                    }
                }
            }

            // === Transport controls ===
            RowLayout {
                Layout.alignment: Qt.AlignVCenter
                spacing: Units.spacingMD

                // Prev (index 0)
                Rectangle {
                    Layout.preferredWidth: Units.gridUnit * 2.2
                    Layout.preferredHeight: Units.gridUnit * 2.2
                    radius: width / 2
                    readonly property bool focused: root.activeFocus && !Theme.mouseMode && root._btn === 0
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

                    Text {
                        anchors.centerIn: parent
                        text: "⏮" // prev-track glyph
                        font.pixelSize: Theme.fontTitle
                        color: Theme.textPrimary
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
                            if (root._canPrev)
                                root.player.previous();
                        }
                    }
                }

                // Play / Pause (index 1)
                Rectangle {
                    Layout.preferredWidth: Units.gridUnit * 2.8
                    Layout.preferredHeight: Units.gridUnit * 2.8
                    radius: width / 2
                    readonly property bool focused: root.activeFocus && !Theme.mouseMode && root._btn === 1
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

                    // Drawn play/pause icon. The Unicode glyphs (▶/⏸) carry
                    // inconsistent side-bearing and baseline metrics, so no
                    // amount of centerIn/offset lands them reliably centred.
                    // Drawing the shapes puts the play triangle's centroid and
                    // the pause bars exactly at the button centre by construction.
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
                                // Two symmetric rounded bars, centred as a group.
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
                                // Play triangle positioned so its centroid sits
                                // at (w/2, h/2): centroid_x = bx + tw/3 = w/2.
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
                            root._btn = 1;
                            if (root._canToggle)
                                root.player.togglePlaying();
                        }
                    }
                }

                // Next (index 2)
                Rectangle {
                    Layout.preferredWidth: Units.gridUnit * 2.2
                    Layout.preferredHeight: Units.gridUnit * 2.2
                    radius: width / 2
                    readonly property bool focused: root.activeFocus && !Theme.mouseMode && root._btn === 2
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

                    Text {
                        anchors.centerIn: parent
                        text: "⏭" // next-track glyph
                        font.pixelSize: Theme.fontTitle
                        color: Theme.textPrimary
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
                            if (root._canNext)
                                root.player.next();
                        }
                    }
                }
            }
        }
    }
}
