import QtQuick
import QtQuick.Layouts
import "lib"

// Home-screen Moonlight widget — the single "jump into streaming" surface. It is
// ONE widget with three sizes that render two different views (the user never
// sees both at once):
//
//   small  = a row of Moonlight *server* cards (StreamCard, one per configured
//            target) with online / active-session status — the glance-and-stream
//            rail. Selecting a server streams it; the context key offers
//            Resume/Quit when a session is live.
//   medium = the Steam library *poster* row (SteamLibraryView) at a smaller
//            poster scale.
//   large  = the same Steam poster row at full poster scale.
//
// The medium/large views are the Steam-library browse (Recently Played / Library
// posters from the host's installed games): selecting a poster launches that game
// on the host (`steam-launch`) then starts the existing single-target Moonlight
// stream. There is exactly one session — cards do not own a stream.
//
// Implements the duck-typed home-tile focus contract (visible / regionFocused /
// canFocus / firstRow / lastRow / focusFirstChild + previousRow/nextRow),
// delegating to whichever sub-view the current size renders, so HomeScreen drives
// it from the same ordered region list as the other widgets.
ColumnLayout {
    id: root

    property Item previousRow: null
    property Item nextRow: null
    property bool widgetEnabled: true
    // "small" (server cards) | "medium" (smaller posters) | "large" (full posters).
    property string size: "medium"

    property var targets: []
    property string shellState: "idle"

    signal escaped
    signal streamRequested(var target)
    signal streamQuitRequested(var target)
    signal ensureVisibleRequested(var item)
    // Raised on the context key (small/server view only); HomeScreen owns the
    // PopoverMenu and reads currentTarget / currentCard / currentHasSession.
    signal contextRequested
    // Emitted when a Steam poster is activated (medium/large view); HomeScreen
    // launches (navigates) the appid then starts the single-target stream IFF
    // none is already live (see HomeScreen.launchSteamGame — one session, ever).
    signal gameSelected(int appid)

    // Whether a Moonlight stream is currently live on the host, mirrored from the
    // `steam-library` reply's `streaming` field (library view only — false in the
    // server view, which has no steam-library poll). HomeScreen reads this to
    // enforce "one session": it only starts a stream on game-select when false.
    readonly property bool streaming: root._libraryView && steamView.streaming

    spacing: Units.spacingMD

    // === View selection ===
    readonly property bool _serverView: root.size === "small"
    readonly property bool _libraryView: !_serverView
    readonly property real _posterScale: root.size === "large" ? 0.82 : 0.62

    readonly property bool _hasTargets: root.targets.length > 0
    // Server view needs a target; library view stands on its own (steam-library
    // health), so it can show even with no Moonlight target configured.
    visible: root.widgetEnabled && (_serverView ? _hasTargets : steamView.visible)

    // === Server-view sizing ===
    readonly property int _cardW: Theme.cardWidth

    // === Home-tile focus contract (delegates to the active sub-view) ===
    readonly property var firstRow: _serverView ? serverRow : steamView.firstRow
    readonly property var lastRow: _serverView ? serverRow : steamView.lastRow
    readonly property bool canFocus: visible && (_serverView ? _hasTargets : steamView.canFocus)
    readonly property bool regionFocused: _serverView ? serverRow.activeFocus : steamView.regionFocused

    // Context-menu passthrough for HomeScreen (server view only).
    readonly property var currentTarget: (_serverView && serverRow.currentIndex >= 0 && serverRow.currentIndex < root.targets.length) ? root.targets[serverRow.currentIndex] : null
    readonly property Item currentCard: _serverView ? serverRow.currentItem : null
    readonly property bool currentHasSession: (_serverView && serverRow.currentItem) ? serverRow.currentItem.hasActiveSession === true : false
    // Host reachable (ping). Used to gate the Resume/Quit stream controls: a
    // stream left running suspends in the background, and the Sunshine session
    // probe (currentHasSession) can't always see it, so "is the host up" is the
    // reliable signal for offering stream management.
    readonly property bool currentOnline: (_serverView && serverRow.currentItem) ? serverRow.currentItem.isOnline === true : false

    function focusFirstChild() {
        if (!root.canFocus)
            return false;
        if (root._serverView)
            return serverRow.focusFirstChild();
        return steamView.focusFirstChild();
    }

    // Title — shown only for the server view; the library view carries its own
    // header (the Recently Played / Library segment tabs).
    Text {
        Layout.fillWidth: true
        visible: root._serverView
        text: "Moonlight"
        font.pixelSize: Theme.fontTitle
        font.bold: true
        color: Theme.textPrimary
    }

    // === small: server cards ===
    NavigableRow {
        id: serverRow
        visible: root._serverView
        Layout.fillWidth: true
        Layout.preferredHeight: root._serverView ? Theme.cardHeight : 0
        keyNavigationWraps: true
        previousRow: root.previousRow
        nextRow: root.nextRow
        model: root.targets
        onActiveFocusChanged: if (activeFocus)
            root.ensureVisibleRequested(this)
        onActivated: {
            if (root.currentTarget)
                root.streamRequested(root.currentTarget);
        }
        onContextRequested: root.contextRequested()
        onEscaped: root.escaped()

        delegate: StreamCard {
            required property int index
            required property var modelData
            width: root._cardW
            height: Theme.cardHeight
            target: modelData
            showProfile: true
            shellState: root.shellState
            focus: index === serverRow.currentIndex
            onActivated: root.streamRequested(modelData)
        }
    }

    // === medium / large: Steam library posters ===
    SteamLibraryView {
        id: steamView
        // The parent decides WHICH view is active (medium/large) via `viewActive`;
        // the child owns its own data-driven `visible`. We must NOT bind
        // `visible:` here — doing so clobbers the persist-last-good binding and
        // renders an empty zero-height column when the library data is good.
        viewActive: root._libraryView
        Layout.fillWidth: true
        posterScale: root._posterScale
        previousRow: root.previousRow
        nextRow: root.nextRow
        onEscaped: root.escaped()
        onGameSelected: appid => root.gameSelected(appid)
        onEnsureVisibleRequested: item => root.ensureVisibleRequested(item)
    }
}
