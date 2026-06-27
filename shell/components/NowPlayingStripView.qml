import QtQuick
import QtQuick.Layouts
import "lib"

// Now-playing SMALL visual — the compact transport strip (#249). A pure visual
// leaf: all MPRIS selection, capability guards, transport actions, and the
// home-tile focus contract live in the MprisPlayerBase host, handed in via
// `base`. A lighter re-presentation of NowPlayingCard: a single row (small art
// thumb · "title · artist" · Open + Prev / Play-Pause / Next) instead of the
// full-width card with big art, the "Now Playing" label, and the progress bar.
FocusFrame {
    id: card

    // The MprisPlayerBase host that owns the MPRIS state + focus.
    property Item base: null

    width: parent.width
    implicitHeight: cardRow.implicitHeight + Units.spacingMD * 2
    focused: base && base.activeFocus && !InputMode.mouseMode
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
                source: base && base.hasPlayer && base.player.trackArtUrl ? base.player.trackArtUrl : ""
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
            animate: base && base.activeFocus
            text: {
                if (!base || !base.hasPlayer)
                    return "";
                let t = base.player.trackTitle || "Unknown title";
                let a = base.player.trackArtist || "";
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
                readonly property bool focused: base && base.activeFocus && !InputMode.mouseMode && base._btn === 0
                color: focused ? Theme.crimson : Theme.surface
                opacity: base && base._canOpen ? 1.0 : 0.35
                border.width: focused ? Units.borderMedium : Units.borderThin
                border.color: focused ? Theme.focusBorder : Theme.surfaceBorder
                Accessible.role: Accessible.Button
                Accessible.name: "Open " + (base && base.playerIdentity || "app")
                Accessible.focusable: true
                Accessible.onPressAction: {
                    if (base && base._canOpen)
                        base.openAppRequested(base.playerDesktopEntry, base.playerIdentity);
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

                PointerTrackingArea {
                    anchors.fill: parent
                    onActivated: {
                        base._btn = 0;
                        if (base._canOpen)
                            base.openAppRequested(base.playerDesktopEntry, base.playerIdentity);
                    }
                }
            }

            // Prev (index 1)
            Rectangle {
                Layout.preferredWidth: Units.gridUnit * 2.0
                Layout.preferredHeight: Units.gridUnit * 2.0
                radius: width / 2
                readonly property bool focused: base && base.activeFocus && !InputMode.mouseMode && base._btn === 1
                color: focused ? Theme.surfaceHover : "transparent"
                opacity: base && base._canPrev ? 1.0 : 0.35
                border.width: focused ? Units.borderMedium : 0
                border.color: Theme.focusBorder
                Accessible.role: Accessible.Button
                Accessible.name: "Previous track"
                Accessible.focusable: true
                Accessible.onPressAction: {
                    if (base && base._canPrev)
                        base.player.previous();
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
                    onPaint: base._paintSkip(getContext("2d"), width, height, iconColor, -1)
                }

                PointerTrackingArea {
                    anchors.fill: parent
                    onActivated: {
                        base._btn = 1;
                        if (base._canPrev)
                            base.player.previous();
                    }
                }
            }

            // Play / Pause (index 2)
            Rectangle {
                Layout.preferredWidth: Units.gridUnit * 2.4
                Layout.preferredHeight: Units.gridUnit * 2.4
                radius: width / 2
                readonly property bool focused: base && base.activeFocus && !InputMode.mouseMode && base._btn === 2
                color: focused ? Theme.crimson : Theme.surface
                opacity: base && base._canToggle ? 1.0 : 0.35
                border.width: focused ? Units.borderMedium : Units.borderThin
                border.color: focused ? Theme.focusBorder : Theme.surfaceBorder
                Accessible.role: Accessible.Button
                Accessible.name: base && base.isPlaying ? "Pause" : "Play"
                Accessible.focusable: true
                Accessible.onPressAction: {
                    if (base && base._canToggle)
                        base.player.togglePlaying();
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
                    property bool playing: base && base.isPlaying
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

                PointerTrackingArea {
                    anchors.fill: parent
                    onActivated: {
                        base._btn = 2;
                        if (base._canToggle)
                            base.player.togglePlaying();
                    }
                }
            }

            // Next (index 3)
            Rectangle {
                Layout.preferredWidth: Units.gridUnit * 2.0
                Layout.preferredHeight: Units.gridUnit * 2.0
                radius: width / 2
                readonly property bool focused: base && base.activeFocus && !InputMode.mouseMode && base._btn === 3
                color: focused ? Theme.surfaceHover : "transparent"
                opacity: base && base._canNext ? 1.0 : 0.35
                border.width: focused ? Units.borderMedium : 0
                border.color: Theme.focusBorder
                Accessible.role: Accessible.Button
                Accessible.name: "Next track"
                Accessible.focusable: true
                Accessible.onPressAction: {
                    if (base && base._canNext)
                        base.player.next();
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
                    onPaint: base._paintSkip(getContext("2d"), width, height, iconColor, 1)
                }

                PointerTrackingArea {
                    anchors.fill: parent
                    onActivated: {
                        base._btn = 3;
                        if (base._canNext)
                            base.player.next();
                    }
                }
            }
        }
    }
}
