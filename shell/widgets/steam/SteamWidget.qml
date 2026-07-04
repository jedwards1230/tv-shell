import QtQuick
import QtQuick.Layouts
import "../lib"
import "../../components"
import "../steamlib"

// Home-screen Steam widget — the LOCAL-Steam poster widget (id `steam`). It
// shows the host's Steam library (the same `steam-library` daemon IPC the
// Moonlight widget's medium/large views poll), but activating a poster launches
// the LOCAL Steam client on THIS machine — it is NOT the Moonlight streaming
// state machine, and it has no server(small) view of its own (that remains
// Moonlight's role). Ships disabled by default (WidgetManifests
// defaultEnabled: false); the user opts in from the Widgets app.
Widget {
    id: root

    // The base defaults size to ""; Steam defaults to the medium poster view.
    size: "medium"

    property var targets: []

    // Emitted when a Steam poster is activated; HomeScreen launches the game
    // locally via `steam steam://nav/games/details/<appid>`.
    signal gameSelected(int appid)
    // Emitted on the X face over the RUNNING game's poster; HomeScreen would open
    // a Resume/Quit popover for a host session — but the local widget has no
    // host session to manage, so HomeScreen intentionally leaves this unwired.
    signal gameContextRequested(int appid)
    // Emitted when the trailing "Open Steam" action chip fires; HomeScreen
    // launches Steam Big Picture locally.
    signal openBigPictureRequested
    // escaped + ensureVisibleRequested are inherited from the Widget base and
    // forwarded by WidgetHost — do NOT redeclare them here.

    readonly property bool _hasTargets: root.targets.length > 0
    // The host whose Steam library this view shows. Taken from the first
    // configured Moonlight target purely because the daemon's `steam-library`
    // IPC is keyed on the same gaming host — this widget never streams to it.
    readonly property string _steamHost: _hasTargets ? (root.targets[0].host || "") : ""
    readonly property real _posterScale: root.size === "large" ? 0.82 : 0.62

    wantVisible: root.widgetEnabled && steamView.hasContent

    implicitWidth: col.implicitWidth
    implicitHeight: root.wantVisible ? col.implicitHeight : 0

    // === Home-tile focus contract (delegates to the hosted view) ===
    firstRow: steamView.firstRow
    lastRow: steamView.lastRow
    canFocus: visible && steamView.canFocus
    readonly property Item runningCard: steamView.runningCard

    function focusFirstChild() {
        if (!root.canFocus)
            return false;
        return steamView.focusFirstChild();
    }

    ColumnLayout {
        id: col
        width: root.width

        SteamLibraryView {
            id: steamView
            // The parent decides whether the widget is enabled; the view owns its
            // own data-driven `visible`. Do NOT bind `visible:` here — doing so
            // clobbers the persist-last-good binding and renders an empty
            // zero-height column when the data is good (see MoonlightWidget).
            viewActive: true
            showSessionIndicator: false
            Layout.fillWidth: true
            posterScale: root._posterScale
            host: root._steamHost
            previousRow: root.previousRow
            nextRow: root.nextRow
            onGameSelected: appid => root.gameSelected(appid)
            onGameContextRequested: appid => root.gameContextRequested(appid)
            onOpenBigPictureRequested: root.openBigPictureRequested()
            onEscaped: root.escaped()
            onEnsureVisibleRequested: item => root.ensureVisibleRequested(item)
        }
    }
}
