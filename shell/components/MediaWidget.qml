import QtQuick
import QtQuick.Layouts
import "lib"

// Now-playing media widget (#22) — the medium (card + progress) renderer. All
// MPRIS selection, capability guards, transport activation, and the home-tile
// focus contract live in MprisPlayerBase; this file supplies only the full-card
// visual: big cover art, "Now Playing" label, track metadata, a live progress
// bar, and the Prev / Play-Pause / Next transport with an "Open app" pill.
MprisPlayerBase {
    id: root
    contentCard: card

    // VISUAL CHANGE: replaces the old border-trick cardGlow Rectangle with
    // FocusFrame's real MultiEffect blur glow — needs on-device screenshot
    // verification to confirm the glow appearance matches the design intent.
    FocusFrame {
        id: card
        width: parent.width
        implicitHeight: cardRow.implicitHeight + Units.spacingLG * 2
        focused: root.activeFocus && !InputMode.mouseMode
        scaleEnabled: false

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

                // Open app (index 0). A labeled pill — visually distinct from the
                // circular transport buttons — that opens the player's desktop
                // app full-screen via the host's launch/focus path.
                Rectangle {
                    id: openButton
                    Layout.preferredHeight: Units.gridUnit * 2.2
                    Layout.preferredWidth: openRow.implicitWidth + Units.spacingLG * 2
                    Layout.alignment: Qt.AlignVCenter
                    radius: height / 2
                    readonly property bool focused: root.activeFocus && !InputMode.mouseMode && root._btn === 0
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
                        spacing: Units.spacingSM

                        Text {
                            text: "⛶" // open-fullscreen glyph
                            font.pixelSize: Theme.fontTitle
                            color: openButton.focused ? Theme.textOnDark : Theme.textPrimary
                        }
                        Text {
                            text: "Open"
                            font.pixelSize: Theme.fontBody
                            font.bold: true
                            color: openButton.focused ? Theme.textOnDark : Theme.textPrimary
                        }
                    }

                    PointerTrackingArea {
                        anchors.fill: parent
                        onActivated: {
                            root._btn = 0;
                            if (root._canOpen)
                                root.openAppRequested(root.playerDesktopEntry, root.playerIdentity);
                        }
                    }
                }

                // Prev (index 1)
                Rectangle {
                    Layout.preferredWidth: Units.gridUnit * 2.2
                    Layout.preferredHeight: Units.gridUnit * 2.2
                    radius: width / 2
                    readonly property bool focused: root.activeFocus && !InputMode.mouseMode && root._btn === 1
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

                    PointerTrackingArea {
                        anchors.fill: parent
                        onActivated: {
                            root._btn = 1;
                            if (root._canPrev)
                                root.player.previous();
                        }
                    }
                }

                // Play / Pause (index 2)
                Rectangle {
                    Layout.preferredWidth: Units.gridUnit * 2.8
                    Layout.preferredHeight: Units.gridUnit * 2.8
                    radius: width / 2
                    readonly property bool focused: root.activeFocus && !InputMode.mouseMode && root._btn === 2
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

                    PointerTrackingArea {
                        anchors.fill: parent
                        onActivated: {
                            root._btn = 2;
                            if (root._canToggle)
                                root.player.togglePlaying();
                        }
                    }
                }

                // Next (index 3)
                Rectangle {
                    Layout.preferredWidth: Units.gridUnit * 2.2
                    Layout.preferredHeight: Units.gridUnit * 2.2
                    radius: width / 2
                    readonly property bool focused: root.activeFocus && !InputMode.mouseMode && root._btn === 3
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

                    PointerTrackingArea {
                        anchors.fill: parent
                        onActivated: {
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
