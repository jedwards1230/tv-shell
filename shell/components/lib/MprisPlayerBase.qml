import QtQuick
import Quickshell.Services.Mpris
import "../"

// Shared base for the two now-playing home widgets — MediaWidget (full card +
// progress) and NowPlayingStrip (compact strip). Both are the same thing
// behaviourally: pick the active MPRIS player, expose capability guards, run a
// Prev / Play-Pause / Next transport with an "Open app" pill, and slot into the
// home-screen vertical focus chain via the duck-typed home-tile contract
// (regionFocused / focusFirstChild + previousRow/nextRow). Only the visual
// rendering differs, so everything except the card body lives here.
//
// A subclass supplies its visual `FocusFrame` as the single child and wires it
// to `contentCard` so the base can size itself; it draws transport icons by
// calling `root._paintSkip(...)` and reads the shared `_btn` / capability
// guards. This dedups ~300 lines that were previously verbatim in both widgets.
FocusScope {
    id: root

    // Vertical focus chain neighbours (set by the host, same contract as
    // NavigableRow). Either may be null.
    property var previousRow: null
    property var nextRow: null

    // Home-screen widget toggle (Settings ▸ Widgets). When false the widget is
    // hidden and collapses to zero height; the host's merged-model filter then
    // lets the player fall back to the running row.
    property bool widgetEnabled: true

    // The subclass's visual card. Drives the widget's height; null collapses it.
    property Item contentCard: null

    signal escaped
    // Context action (gamepad X / Tab) — the host opens a quit popover.
    signal contextRequested
    // Open the player's desktop app full-screen. The host resolves the
    // identifiers to a launchable app and routes through the normal launch/
    // focus path (focuses the running window, or launches it if not open).
    signal openAppRequested(string desktopEntry, string identity)

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

    // === Home-tile focus contract (mirrors NavigableRow) ===
    // The widget is a single focusable strip, so its "first child" is itself.
    // regionFocused lets HomeScreen's re-anchor net recognise this region.
    readonly property bool regionFocused: activeFocus

    function focusFirstChild() {
        if (!visible)
            return false;
        forceActiveFocus();
        return true;
    }

    // Identifiers used to resolve the player back to its desktop app so we can
    // open it full-screen (and so the host can hide it from the recents row,
    // since the widget already represents it). desktopEntry is the .desktop
    // basename (e.g. "spotify"); identity is the human name (e.g. "Spotify").
    readonly property string playerDesktopEntry: hasPlayer && player.desktopEntry ? player.desktopEntry : ""
    readonly property string playerIdentity: hasPlayer && player.identity ? player.identity : ""

    // Focus index: 0 = Open app, 1 = Prev, 2 = Play/Pause, 3 = Next.
    property int _btn: 2

    // Capability guards — a player may not advertise every control.
    readonly property bool _canPrev: hasPlayer && player.canGoPrevious
    readonly property bool _canNext: hasPlayer && player.canGoNext
    readonly property bool _canToggle: hasPlayer && (player.canTogglePlaying || player.canPlay || player.canPause)
    // Always offer "Open app" when a player is present — even players that
    // don't advertise MPRIS Raise can still be resolved + focused/launched by
    // the host via the desktop entry / identity.
    readonly property bool _canOpen: hasPlayer && (playerDesktopEntry !== "" || playerIdentity !== "")

    readonly property bool _shown: hasPlayer && widgetEnabled
    implicitHeight: _shown && contentCard ? contentCard.implicitHeight : 0
    visible: _shown
    height: implicitHeight

    // Draw a skip-track icon (two triangles + a bar) centred by construction.
    // The Unicode ⏮/⏭ glyphs carry inconsistent side-bearing so anchors.centerIn
    // never lands them centred (same reason play/pause is Canvas-drawn). dir = 1 →
    // forward (triangles point right, bar on right); dir = -1 → back (mirrored).
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
            // Context action (gamepad X): ask the host to open the quit popover.
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
}
