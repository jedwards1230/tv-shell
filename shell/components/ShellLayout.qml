import Quickshell.Io
import QtQuick
import "../settings"

FocusScope {
    id: root

    property string shellState: ""
    property var targets: StreamProviders.active.targets
    property var runningWindows: []
    property string runningAppClass: ""
    property bool overlayDrawerOpen: false
    property bool avSystemOn: false

    property var applications: AppDiscoveryManager.applications
    property var pads: []
    property alias homeScreen: homeScreen
    property alias libraryScreen: libraryScreen
    property alias settingsApp: settingsApp
    property alias navDrawer: navDrawer
    property alias overlay: overlay
    property alias notificationCenter: notificationCenter
    property alias powerOverlay: powerOverlay
    property alias volumeOverlay: volumeOverlay
    property alias networkOverlay: networkOverlay
    property alias sessionQam: sessionQam
    property alias overlayNavDrawer: overlayNavDrawer

    signal streamRequested(var target)
    signal streamQuitRequested(var target)
    signal appLaunchRequested(var app)
    signal appFocusRequested(string address)
    signal appCloseRequested(string address)
    signal returnToShellRequested
    signal overlayDrawerClosed
    // Forwarded from HomeScreen.userActivity — any B-press / Escaped navigation.
    // Lets shell.qml reset the auto-suspend idle timer on real user interaction.
    signal userActivity

    // Session conflict dialog — driven by StreamManager signals
    property alias sessionDialog: sessionDialog

    function focusHome() {
        homeFocusTimer.restart();
    }

    // Reset the home screen to its default focus position (first card of the
    // first visible row). Exposed for the future screensaver hook (issue #156);
    // shell.qml's resetToHome() / returnToShell() reset path uses focusHome()
    // (above), not this function.
    //
    // A single Qt.callLater defers both steps until the current event-loop
    // iteration completes — layout and declarative focus bindings have settled
    // by then, so moonlightRow's focus: binding cannot steal focus after
    // recentsRow.forceActiveFocus() is called inside focusDefaultPosition().
    function focusDefaultPosition() {
        Qt.callLater(function () {
            if (!homeScreen.visible || settingsApp.visible || navDrawer.opened || notificationCenter.opened || powerOverlay.opened || networkOverlay.opened || volumeOverlay.opened || sessionQam.opened)
                return;
            homeScreen.forceActiveFocus();
            homeScreen.focusDefaultPosition();
        });
    }

    // Toggle the nav drawer — the focus-scoped `menu` action. Converges every
    // drawer-toggle surface: the gamepad Home-tap (shell.qml's onIntentHomeTap
    // when idle), the on-screen menu button, and the keyboard Tab path below.
    // If an overlay is open, the first press dismisses it instead of opening the
    // drawer. (The global return-to-shell escape lives in shell.qml's
    // onIntentHome, NOT here — `menu` never leaves a running app.)
    function toggleMenu() {
        if (powerOverlay.opened) {
            powerOverlay.opened = false;
            homeFocusTimer.restart();
        } else if (volumeOverlay.opened) {
            volumeOverlay.opened = false;
            homeFocusTimer.restart();
        } else if (networkOverlay.opened) {
            networkOverlay.opened = false;
            homeFocusTimer.restart();
        } else if (notificationCenter.opened) {
            notificationCenter.opened = false;
            homeFocusTimer.restart();
        } else {
            navDrawer.opened = !navDrawer.opened;
            if (navDrawer.opened)
                navDrawer.forceActiveFocus();
            else
                homeFocusTimer.restart();
        }
    }

    // Keyboard drawer toggle (Tab), independent of Home. First-class keyboard
    // co-primary: the K400 on the couch opens the drawer without a controller.
    // Only meaningful on the home screen; when an app owns focus the shell
    // window isn't focused, so this never fires over a stream.
    Keys.onTabPressed: {
        if (root.shellState === "idle")
            toggleMenu();
    }

    // --- Keyboard debug capture (replaces the deleted daemon keys: snoop) ---
    // Rolling list of currently-held keyboard keys, read straight from Wayland
    // `Keys` on the layout root. The daemon stopped snooping the keyboard in
    // Phase 2, so the debug pane's keyboard half is now QML-native. Only used
    // when Theme.controllerDebug is on; the handlers never consume the event
    // (accepted stays false) so navigation is unaffected.
    property var _debugKeyNames: []

    function _debugKeyName(event) {
        // Friendly labels for the keys most worth seeing; fall back to the raw
        // text, else the numeric key code.
        switch (event.key) {
        case Qt.Key_Up:
            return "↑";
        case Qt.Key_Down:
            return "↓";
        case Qt.Key_Left:
            return "←";
        case Qt.Key_Right:
            return "→";
        case Qt.Key_Return:
        case Qt.Key_Enter:
            return "Enter";
        case Qt.Key_Escape:
            return "Esc";
        case Qt.Key_Tab:
            return "Tab";
        case Qt.Key_Backspace:
            return "Backspace";
        case Qt.Key_Space:
            return "Space";
        case Qt.Key_Meta:
        case Qt.Key_Super_L:
        case Qt.Key_Super_R:
            return "Meta";
        case Qt.Key_Control:
            return "Ctrl";
        case Qt.Key_Shift:
            return "Shift";
        case Qt.Key_Alt:
            return "Alt";
        }
        if (event.text && event.text.trim().length > 0)
            return event.text.toUpperCase();
        return "0x" + event.key.toString(16);
    }

    Keys.onPressed: event => {
        if (Theme.controllerDebug && !event.isAutoRepeat) {
            let name = _debugKeyName(event);
            let names = root._debugKeyNames.slice();
            if (names.indexOf(name) === -1) {
                names.push(name);
                root._debugKeyNames = names;
                debugOverlay.currentKeys = names.join(" + ");
            }
        }
        // Any real keypress is user activity — reset the auto-suspend idle
        // countdown so the shell never sleeps mid-navigation (#162). This is
        // the central, non-consuming observer: directional/A/B nav keys are
        // accepted by the focused row's Keys handlers, but this FocusScope-root
        // handler still sees every event first. We DON'T consume it
        // (event.accepted = false below) so navigation is unaffected. Skip
        // auto-repeat so a held key doesn't fire the signal on every tick; the
        // shell.qml side already gates the actual restart on state === "idle".
        if (!event.isAutoRepeat)
            root.userActivity();
        event.accepted = false;
    }

    Keys.onReleased: event => {
        if (Theme.controllerDebug && !event.isAutoRepeat) {
            let name = _debugKeyName(event);
            let names = root._debugKeyNames.filter(n => n !== name);
            root._debugKeyNames = names;
            debugOverlay.currentKeys = names.join(" + ");
        }
        event.accepted = false;
    }

    HomeScreen {
        id: homeScreen
        anchors.fill: parent
        visible: root.shellState === "idle" && !libraryScreen.visible
        targets: root.targets
        shellState: root.shellState
        focus: root.shellState === "idle" && !libraryScreen.visible && !settingsApp.visible && !navDrawer.opened && !notificationCenter.opened && !powerOverlay.opened && !networkOverlay.opened && !volumeOverlay.opened && !sessionQam.opened

        runningWindows: root.runningWindows
        pads: root.pads

        onStreamRequested: target => root.streamRequested(target)
        onStreamQuitRequested: target => root.streamQuitRequested(target)
        onAppLaunchRequested: app => root.appLaunchRequested(app)
        onAppFocusRequested: address => root.appFocusRequested(address)
        onAppCloseRequested: address => root.appCloseRequested(address)
        onLibraryRequested: {
            libraryScreen.visible = true;
            libraryScreen.forceActiveFocus();
            libraryScreen.focusDefaultPosition();
        }
        onSettingsRequested: settingsApp.open()
        onNetworkRequested: anchorRect => networkOverlay.openAt(anchorRect)
        onVolumeRequested: anchorRect => volumeOverlay.openAt(anchorRect)
        onNotificationCenterRequested: {
            notificationCenter.opened = true;
            notificationCenter.forceActiveFocus();
        }
        onPowerRequested: {
            powerOverlay.opened = true;
            powerOverlay.forceActiveFocus();
        }
    }

    Connections {
        target: homeScreen
        function onUserActivity() {
            root.userActivity();
        }
    }

    // === Library (secondary browse surface, #249) ===
    // Full app + streaming catalog, opened from the home "All Apps" entry. Sits
    // over the home screen (which hides while it's visible); B/Escape closes it
    // and returns focus to home (SettingsApp-style lifecycle).
    LibraryScreen {
        id: libraryScreen
        anchors.fill: parent
        visible: false
        z: 30
        targets: root.targets
        shellState: root.shellState
        focus: libraryScreen.visible

        // Picking anything in the Library launches it and dismisses the surface,
        // so returning to the shell (idle) lands back on Home, not a stale catalog.
        onStreamRequested: target => {
            libraryScreen.visible = false;
            root.streamRequested(target);
        }
        onStreamQuitRequested: target => root.streamQuitRequested(target)
        onAppLaunchRequested: app => {
            libraryScreen.visible = false;
            root.appLaunchRequested(app);
        }
        onAppFocusRequested: address => {
            libraryScreen.visible = false;
            root.appFocusRequested(address);
        }
        onAppCloseRequested: address => root.appCloseRequested(address)
        onUserActivity: root.userActivity()
        onClosed: {
            libraryScreen.visible = false;
            homeFocusTimer.restart();
        }
    }

    // Safety net: if the shell leaves idle for any reason while the Library is
    // open (e.g. an externally-started stream), drop the surface so it never
    // floats over a running app.
    Connections {
        target: root
        function onShellStateChanged() {
            if (root.shellState !== "idle")
                libraryScreen.visible = false;
        }
    }

    SettingsApp {
        id: settingsApp
        anchors.fill: parent
        onClosed: {
            settingsApp.close();
            homeFocusTimer.restart();
        }
    }

    // When an anchored Volume/Network popover closes, return focus to the nav
    // drawer's QuickActions row if the drawer is still open (the popover was
    // launched from the drawer and the drawer stayed visible underneath);
    // otherwise fall back to the home screen (home-launched case).
    function _returnFocusAfterOverlay() {
        if (navDrawer.opened)
            navDrawer.focusQuickActions();
        else if (overlayNavDrawer.opened)
            overlayNavDrawer.focusQuickActions();
        else
            homeFocusTimer.restart();
    }

    Timer {
        id: homeFocusTimer
        interval: 50
        onTriggered: {
            if (notificationCenter.opened || errorLogViewer.opened || powerOverlay.opened || volumeOverlay.opened || networkOverlay.opened || sessionQam.opened || settingsApp.visible || libraryScreen.visible)
                return;
            homeScreen.forceActiveFocus();
        }
    }

    StreamOverlay {
        id: overlay
        anchors.fill: parent
    }

    SessionDialog {
        id: sessionDialog
    }

    // === Navigation Drawer (idle state) ===
    NavigationDrawer {
        id: navDrawer
        z: 50
        visible: root.shellState === "idle"
        onSettingsRequested: {
            navDrawer.opened = false;
            settingsApp.open();
        }
        onNotificationCenterRequested: {
            navDrawer.opened = false;
            notificationCenter.opened = true;
            notificationCenter.forceActiveFocus();
        }
        onPowerRequested: {
            navDrawer.opened = false;
            powerOverlay.opened = true;
            powerOverlay.forceActiveFocus();
        }
        onNetworkRequested: anchorRect => {
            // Leave the drawer open; the overlay (higher z) paints on top.
            networkOverlay.openAt(anchorRect);
        }
        onVolumeRequested: anchorRect => {
            volumeOverlay.openAt(anchorRect);
        }
        onHomeSelected: {
            navDrawer.opened = false;
            settingsApp.close();
            homeFocusTimer.restart();
        }
        onClosed: {
            navDrawer.opened = false;
            homeFocusTimer.restart();
        }
    }

    // === Notification Stack ===
    NotificationStack {
        anchors.right: parent.right
        anchors.top: parent.top
        anchors.rightMargin: Units.spacingXL
        anchors.topMargin: Units.gridUnit * 9
        z: 45
    }

    // === Error Log Viewer ===
    ErrorLogViewer {
        id: errorLogViewer
        z: 60
    }

    // === Notification Center ===
    NotificationCenter {
        id: notificationCenter
        z: 60
        onErrorLogRequested: {
            errorLogViewer.opened = true;
            errorLogViewer.forceActiveFocus();
            notificationCenter.opened = false;
        }
    }

    // === Power Overlay ===
    PowerOverlay {
        id: powerOverlay
        z: 60
        onCancelled: {
            powerOverlay.opened = false;
            homeFocusTimer.restart();
        }
    }

    // === Volume Overlay ===
    // z above the nav drawer (z:50) so the anchored popover paints in front of
    // it while the drawer stays open underneath (#118).
    VolumeOverlay {
        id: volumeOverlay
        z: 70
        onOpenedChanged: {
            if (!volumeOverlay.opened)
                root._returnFocusAfterOverlay();
        }
    }

    // === Network Overlay ===
    NetworkOverlay {
        id: networkOverlay
        z: 70
        onOpenedChanged: {
            if (!networkOverlay.opened)
                root._returnFocusAfterOverlay();
        }
    }

    // === Session QAM (right-edge drawer, #218) ===
    // z above the nav drawer so it paints over a still-open left drawer.
    SessionQAM {
        id: sessionQam
        z: 70
        onClosed: {
            sessionQam.opened = false;
            // Idle: restore home focus. appRunning: just unmap — the shell
            // window hides (its `visible` keys off sessionQam.opened) and the
            // app regains keyboard focus on its own.
            if (root.shellState === "idle")
                homeFocusTimer.restart();
        }
        onNotificationCenterRequested: {
            sessionQam.opened = false;
            notificationCenter.opened = true;
            notificationCenter.forceActiveFocus();
        }
    }

    // === Overlay Drawer (appRunning state) ===
    Item {
        anchors.fill: parent
        // Stay visible through the close slide-out: gate on the drawer's
        // `active` (open || animating), not just overlayDrawerOpen, or the
        // subtree hides instantly and the drawer vanishes in place over an app.
        visible: root.shellState === "appRunning" && (root.overlayDrawerOpen || overlayNavDrawer.active)
        z: 50

        DimmedBackdrop {
            dimLevel: 0.5
            onClicked: root.overlayDrawerClosed()
        }

        NavigationDrawer {
            id: overlayNavDrawer
            overlayMode: true
            opened: root.overlayDrawerOpen
            onHomeSelected: {
                root.returnToShellRequested();
            }
            onSettingsRequested: {
                root.overlayDrawerClosed();
                root.returnToShellRequested();
                settingsApp.open();
            }
            onNotificationCenterRequested: {
                root.returnToShellRequested();
                notificationCenter.opened = true;
                notificationCenter.forceActiveFocus();
            }
            onPowerRequested: {
                root.returnToShellRequested();
                powerOverlay.opened = true;
                powerOverlay.forceActiveFocus();
            }
            onNetworkRequested: anchorRect => {
                // Quick popover over the running app — keep the overlay drawer
                // open underneath (window stays mapped) and anchor to this
                // glyph. Do NOT return to the shell (that kicked the user to
                // Home and stranded the popover). Closing it returns focus to
                // the drawer via _returnFocusAfterOverlay().
                networkOverlay.openAt(anchorRect);
            }
            onVolumeRequested: anchorRect => {
                volumeOverlay.openAt(anchorRect);
            }
            onClosed: {
                root.overlayDrawerClosed();
            }
        }

        Keys.onEscapePressed: {
            root.overlayDrawerClosed();
        }
    }

    Connections {
        target: notificationCenter
        function onOpenedChanged() {
            if (!notificationCenter.opened && !errorLogViewer.opened)
                homeFocusTimer.restart();
        }
    }

    Connections {
        target: errorLogViewer
        function onOpenedChanged() {
            if (!errorLogViewer.opened)
                homeFocusTimer.restart();
        }
    }

    Connections {
        target: powerOverlay
        function onOpenedChanged() {
            if (!powerOverlay.opened)
                homeFocusTimer.restart();
        }
    }

    // --- Debug Input Overlay ---
    // Controller `buttons:` come from the daemon subscribe stream; keyboard
    // keys are read QML-side from Wayland `Keys` (see the root key-capture
    // handlers below) since the daemon no longer snoops the keyboard.
    // See docs/IPC_PROTOCOL.md.
    Item {
        id: debugOverlay
        anchors.fill: parent
        visible: Theme.controllerDebug
        z: 100

        property string currentCombo: ""
        property string currentKeys: ""
        property string displayInput: ""
        property bool showingInput: false

        readonly property string currentInput: {
            // A gamepad button and the key it synthesizes both surface here —
            // currentCombo from the daemon `buttons:` stream, currentKeys from the
            // Wayland Keys capture. When a face button's synthesized key isn't
            // consumed by a focused handler (e.g. the X face → BTN "X" + KEY_X "X"),
            // both panes read the same label; don't print it twice.
            if (currentCombo !== "" && currentKeys !== "" && currentCombo !== currentKeys)
                return currentCombo + " + " + currentKeys;
            return currentCombo !== "" ? currentCombo : currentKeys;
        }

        SocketClient {
            id: debugSubscribe
            // Subscribe to the daemon's controller `buttons:` stream over a
            // native Quickshell socket (SocketClient, #97). The keyboard half of
            // the pane no longer comes from the daemon — the daemon stopped
            // snooping the keyboard (Phase 2). Keyboard keys are captured
            // QML-side from Wayland `Keys` on the layout root and fed into
            // `debugOverlay.currentKeys`. Auto-reconnects on drop.
            subscribe: true
            onLineReceived: line => {
                if (line === "subscribed")
                    return;
                if (line.startsWith("buttons:")) {
                    debugOverlay.currentCombo = line.substring(8).trim();
                }
            }
        }

        // Pin displayInput to the latest non-empty value and hold it
        // briefly so a quick tap is still readable.
        onCurrentInputChanged: {
            if (currentInput !== "") {
                displayInput = currentInput;
                showingInput = true;
                inputFadeTimer.restart();
            }
        }

        Timer {
            id: inputFadeTimer
            interval: 1500
            onTriggered: {
                if (debugOverlay.currentInput === "")
                    debugOverlay.showingInput = false;
            }
        }

        onVisibleChanged: {
            if (!visible) {
                debugSubscribe.stop();
                showingInput = false;
                currentCombo = "";
                currentKeys = "";
                displayInput = "";
                root._debugKeyNames = [];
            } else {
                debugSubscribe.start();
            }
        }

        Rectangle {
            anchors.right: parent.right
            anchors.bottom: parent.bottom
            anchors.rightMargin: 48
            anchors.bottomMargin: 48
            width: comboText.implicitWidth + 64
            height: comboText.implicitHeight + 32
            radius: Units.radiusMD
            color: Qt.rgba(0, 0, 0, 0.8)
            border.width: 2
            border.color: Theme.ember
            visible: debugOverlay.showingInput
            opacity: debugOverlay.currentInput !== "" ? 1.0 : 0.4

            Behavior on opacity {
                NumberAnimation {
                    duration: 300
                }
            }

            Text {
                id: comboText
                anchors.centerIn: parent
                text: debugOverlay.displayInput
                font.pixelSize: Theme.fontBody
                font.bold: true
                color: Theme.textOnDark
            }
        }
    }
}
