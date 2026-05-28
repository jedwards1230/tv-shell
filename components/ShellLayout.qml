import Quickshell.Io
import QtQuick

FocusScope {
    id: root

    property string shellState: ""
    property var targets: []
    property var runningWindows: []
    property string runningAppClass: ""
    property bool overlayDrawerOpen: false
    property bool avSystemOn: false
    property bool avWaking: false

    property var applications: homeScreen.applications
    property alias homeScreen: homeScreen
    property alias settingsPanel: settingsPanel
    property alias navDrawer: navDrawer
    property alias overlay: overlay
    property alias notificationCenter: notificationCenter

    signal streamRequested(var target)
    signal streamQuitRequested(var target)
    signal appLaunchRequested(var app)
    signal appFocusRequested(string windowClass)
    signal appCloseRequested(string windowClass)
    signal returnToShellRequested
    signal overlayDrawerClosed

    // Owns tap/hold detection for the keyboard Meta/Super key and exposes
    // shared debug state. Set from shell.qml.
    property var inputManager: null

    // Session conflict dialog — driven by StreamManager signals
    property alias sessionDialog: sessionDialog

    function focusHome() {
        homeFocusTimer.restart();
    }

    // Drawer-toggle behavior for the home-tap action (controller Home tap,
    // Meta key tap, Qt.Key_HomePage). Public so shell.qml's onHomePressed
    // handler can call it from idle state.
    function handleHomeTap() {
        if (notificationCenter.opened) {
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

    Keys.onPressed: event => {
        if (event.key === Qt.Key_HomePage) {
            if (root.inputManager)
                root.inputManager.simulateHomeTap();
            event.accepted = true;
        } else if (root.inputManager && root.inputManager.isMetaKey(event.key)) {
            root.inputManager.handleMetaPress(event.isAutoRepeat);
            event.accepted = true;
        }
    }

    Keys.onReleased: event => {
        if (root.inputManager && root.inputManager.isMetaKey(event.key)) {
            root.inputManager.handleMetaRelease(event.isAutoRepeat);
            event.accepted = true;
        }
    }

    HomeScreen {
        id: homeScreen
        anchors.fill: parent
        visible: root.shellState === "idle"
        targets: root.targets
        shellState: root.shellState
        focus: root.shellState === "idle" && !settingsPanel.visible && !navDrawer.opened && !notificationCenter.opened

        runningWindows: root.runningWindows

        onStreamRequested: target => root.streamRequested(target)
        onStreamQuitRequested: target => root.streamQuitRequested(target)
        onAppLaunchRequested: app => root.appLaunchRequested(app)
        onAppFocusRequested: windowClass => root.appFocusRequested(windowClass)
        onAppCloseRequested: windowClass => root.appCloseRequested(windowClass)
        onSettingsRequested: {
            settingsPanel.visible = true;
            settingsPanel.forceActiveFocus();
        }
        onNotificationCenterRequested: {
            notificationCenter.opened = true;
            notificationCenter.forceActiveFocus();
        }
    }

    SettingsPanel {
        id: settingsPanel
        anchors.fill: parent
        onClosed: {
            settingsPanel.visible = false;
            homeFocusTimer.restart();
        }
    }

    Timer {
        id: homeFocusTimer
        interval: 50
        onTriggered: {
            if (notificationCenter.opened || errorLogViewer.opened)
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
            settingsPanel.visible = true;
            settingsPanel.forceActiveFocus();
        }
        onNotificationCenterRequested: {
            navDrawer.opened = false;
            notificationCenter.opened = true;
            notificationCenter.forceActiveFocus();
        }
        onHomeSelected: {
            navDrawer.opened = false;
            settingsPanel.visible = false;
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

    // === Overlay Drawer (appRunning state) ===
    Item {
        anchors.fill: parent
        visible: root.shellState === "appRunning" && root.overlayDrawerOpen
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
                settingsPanel.visible = true;
                settingsPanel.forceActiveFocus();
            }
            onNotificationCenterRequested: {
                root.returnToShellRequested();
                notificationCenter.opened = true;
                notificationCenter.forceActiveFocus();
            }
            onClosed: {
                root.overlayDrawerClosed();
            }
        }

        Keys.onEscapePressed: {
            root.overlayDrawerClosed();
        }
    }

    // --- AV Wake Overlay ---
    DimmedBackdrop {
        visible: root.avWaking
        z: 40
        dimLevel: 0.7
        message: "Waking AV System..."
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

    // --- Debug Input Overlay ---
    // Reads live state from InputManager (controller combos via socket
    // buttons:* events, keyboard via Meta key tap/hold handlers). No own
    // socket subscription — InputManager is the single source of truth.
    Item {
        id: debugOverlay
        anchors.fill: parent
        visible: Theme.controllerDebug
        z: 100

        // Last non-empty input — kept on screen briefly after release so a
        // quick tap is still readable.
        property string displayInput: ""
        property bool showingInput: false

        readonly property string controllerCombo: root.inputManager ? root.inputManager.currentControllerCombo : ""
        readonly property string keyName: root.inputManager ? root.inputManager.currentKey : ""
        readonly property string currentInput: {
            if (controllerCombo !== "" && keyName !== "")
                return controllerCombo + " + " + keyName;
            return controllerCombo !== "" ? controllerCombo : keyName;
        }

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
                showingInput = false;
                displayInput = "";
            }
        }

        Rectangle {
            anchors.right: parent.right
            anchors.bottom: parent.bottom
            anchors.rightMargin: 48
            anchors.bottomMargin: 48
            width: comboText.implicitWidth + 64
            height: comboText.implicitHeight + 32
            radius: 16
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
