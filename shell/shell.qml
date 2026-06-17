import Quickshell
import Quickshell.Io
import Quickshell.Wayland
import QtQuick
import QtQuick.Layouts
import "components" as Components

ShellRoot {
    id: root

    property string state: "idle"
    property bool overlayDrawerOpen: false
    property var _applications: []
    property var _layout: null

    // #193 launch-overlay state — drives the dedicated Overlay-layer "Launching…"
    // window below. Shown from launchStarted until windowConfirmed (or a safety
    // timeout), so the previous app never bleeds through the launch gap.
    property bool _launchOverlayActive: false
    property string _launchAppName: ""
    property string _launchAppIcon: ""

    // Emitted on any user activity (controller, keyboard, mouse) so child
    // components can reset their own inactivity timers without referencing IDs
    // across Variants scope boundaries.
    signal userActivityDetected

    // Declarative state machine — ShellRoot doesn't inherit Item,
    // so we host states/transitions on an internal Item wrapper.
    Item {
        id: stateMachine
        state: root.state

        states: [
            State {
                name: "idle"
                PropertyChanges {
                    target: root
                    // restoreEntryValues:false — when idle is entered with the
                    // overlay drawer open (Home-tap over an app, then Home from
                    // the nav), force it closed but do NOT restore the prior
                    // `true` on exit. Without this, launching the next app
                    // restores overlayDrawerOpen=true and the drawer reopens
                    // over the fresh app.
                    restoreEntryValues: false
                    overlayDrawerOpen: false
                }
            },
            State {
                name: "launching"
            },
            State {
                name: "streaming"
            },
            State {
                name: "reconnecting"
            },
            State {
                name: "appRunning"
            }
        ]

        transitions: [
            Transition {
                from: "*"
                to: "idle"
            }
        ]
    }

    Process {
        id: streamQuitProc
        onExited: {
            Components.NotificationManager.info("stream", "Stream Session Ended");
        }
    }

    // === Hoisted auto-suspend idle timer (issue #162) ===
    // Lives at the shell root so it is always running, independent of which
    // settings page (if any) is visible. Restarted whenever sleepTimerMinutes
    // changes (folds in the Ea coupled fix from #162).
    //
    // Activity that resets the countdown:
    //   - InputManager intent signals: controllerWake, intentHome, intentHomeTap,
    //     intentHomeHold (global/automation intents — these are the wakeup surface)
    //   - Shell state transitions: returnToShell() (launching / returning to idle)
    //   - Any real keypress: ShellLayout's root Keys.onPressed observer emits
    //     userActivity() for every non-auto-repeat key (D-pad / A / B / arrows /
    //     Enter / Esc) without consuming the event, so ordinary navigation now
    //     keeps the shell awake (#162). Wired via ShellLayout.onUserActivity below.
    // The timer fires only when idle (state === "idle") to avoid suspending
    // during an active stream or app session.

    Components.AutoSuspendController {
        id: autoSuspend
        shellState: root.state
    }

    function _resetIdleTimer() {
        autoSuspend.resetTimer();
    }

    // #193: backstop so the launch overlay can't get stuck if a window never maps
    // (launch failed silently, app exited instantly, etc.). The window poller hides
    // it the moment the app actually appears, so on the happy path this never
    // fires — it's sized longer than a worst-case cold flatpak launch (~15-20s) so
    // it never pre-empts a slow-but-valid start (which was the 1Password-bleeds-
    // through-for-15s symptom).
    Timer {
        id: launchOverlayTimeout
        interval: 30000
        repeat: false
        onTriggered: root._launchOverlayActive = false
    }

    Component.onCompleted: {
        inputManager.grab();
        inputManager.startListening();
    }

    Components.InputManager {
        id: inputManager
        onForceQuitRequested: root.forceQuit()
        onEndSessionRequested: inputManager.endSession()
        onSuspendStreamRequested: {
            if (root.state === "streaming" || root.state === "reconnecting")
                streamManager.suspend();
        }
        onControllerWake: {
            root._resetIdleTimer();
            root.userActivityDetected();
            // Only wake the AV system if the user has enabled wake-on-controller (#130).
            if (root.state === "idle" && Components.SettingsStore.wakeOnController)
                avController.wake();
        }

        // --- Control-surface intents (de-overloaded "home") ---
        // intent:home is the GLOBAL escape (keyboard Super+Escape / automation):
        // always leave the running app, regardless of focus. Fires instantly.
        onIntentHome: {
            root._resetIdleTimer();
            root.userActivityDetected();
            root.returnToShell();
        }

        // intent:home-tap is the gamepad Home neutral (short press). The shell
        // owns the focus, so it decides: over a running app -> toggle the app
        // overlay drawer (the full return-to-shell is Home-hold / intent:home);
        // on the home screen -> toggle the nav drawer (the `menu` action).
        onIntentHomeTap: {
            root._resetIdleTimer();
            root.userActivityDetected();
            if (root.state === "appRunning") {
                root.overlayDrawerOpen = !root.overlayDrawerOpen;
            } else if (root.state === "idle" && root._layout) {
                // Only wake the AV system if the user has enabled wake-on-controller (#130).
                if (Components.SettingsStore.wakeOnController)
                    avController.wake();
                root._layout.toggleMenu();
            }
        }

        // intent:home-hold = reset. Fired by the gamepad Home long-press AND by
        // keyboard Super+Backspace.
        onIntentHomeHold: {
            root._resetIdleTimer();
            root.userActivityDetected();
            root.resetToHome();
        }

        // intent:menu toggles the nav drawer. Fired by bare Super (keyboard),
        // the gamepad Home-tap on the home screen, and the on-screen menu
        // button. Home-screen only: a bare Super press also precedes a
        // Super+<key> chord, so over a running app `menu` is a deliberate no-op
        // (the chord's intent:home/home-hold does the real work, no overlay flash).
        onIntentMenu: {
            root.userActivityDetected();
            if (root.state === "idle" && root._layout)
                root._layout.toggleMenu();
        }

        // intent:settings / intent:power open their panels from the home screen.
        onIntentSettings: {
            root.userActivityDetected();
            if (root.state === "idle" && root._layout) {
                root._layout.settingsPanel.visible = true;
                root._layout.settingsPanel.forceActiveFocus();
            }
        }
        onIntentPower: {
            root.userActivityDetected();
            if (root.state === "idle" && root._layout) {
                root._layout.powerOverlay.opened = true;
                root._layout.powerOverlay.forceActiveFocus();
            }
        }

        // Deep-link intent handlers — open a specific view in one command.
        // All guarded by root.state === "idle" to match coarse intents.
        onIntentSettingsPage: page => {
            root.userActivityDetected();
            if (root.state === "idle" && root._layout) {
                let ok = root._layout.settingsPanel.openSectionById(page);
                if (!ok)
                    console.log("shell: unknown settings page deep-link:", page);
            }
        }
        onIntentOverlay: target => {
            root.userActivityDetected();
            if (!root._layout)
                return;
            // The Session QAM opens on the home screen AND over a running local
            // app — it rides the shell's overlay surface (the window maps while
            // it's open, dimming the app via the drawer scrim) and returns to
            // the app on close. The other overlays stay idle-only for now.
            if (target === "session") {
                // Toggle: the View button (and Super+Right) both open and close
                // the QAM — a second press while it's open closes it.
                if (root.state === "idle" || root.state === "appRunning") {
                    if (root._layout.sessionQam.opened)
                        root._layout.sessionQam.close();
                    else
                        root._layout.sessionQam.open();
                }
                return;
            }
            if (root.state === "idle") {
                if (target === "volume")
                    root._layout.volumeOverlay.openAt(null);
                else if (target === "network")
                    root._layout.networkOverlay.openAt(null);
                else
                    console.log("shell: unknown overlay target deep-link:", target);
            }
        }
        onIntentApp: appId => {
            root.userActivityDetected();
            if (root.state === "idle") {
                let apps = root._applications || [];
                let match = null;
                for (let i = 0; i < apps.length; i++) {
                    if (apps[i].wmClass && apps[i].wmClass === appId) {
                        match = apps[i];
                        break;
                    }
                }
                if (match)
                    appLifecycle.checkAndLaunchApp(match);
                else
                    console.log("shell: no app for deep-link:", appId);
            }
        }
    }

    Components.AVController {
        id: avController
        shellState: root.state
    }

    Components.AppLifecycleManager {
        id: appLifecycle
        shellState: root.state
        applications: root._applications
        onAppLaunched: {
            root.state = "appRunning";
            // #99: short haptic confirmation on a successful app launch.
            if (inputManager)
                inputManager.rumblePulse(120);
        }
        onAppClosed: {
            appLifecycle.runningAppClass = "";
            root.returnToShell();
        }
        // App failed to launch (non-zero exit from the launcher process).
        // Stronger double-ish pulse so the failure is felt, not just logged.
        onAppLaunchFailed: {
            root._launchOverlayActive = false;
            launchOverlayTimeout.stop();
            if (inputManager)
                inputManager.rumblePulse(250);
        }
        // #193: show the "Launching…" overlay the instant a launch begins, hide
        // it once the window is confirmed mapped (or the safety timeout fires).
        onLaunchStarted: app => {
            root._launchAppName = (app && app.name) ? app.name : "";
            root._launchAppIcon = (app && app.icon) ? app.icon : "";
            root._launchOverlayActive = true;
            launchOverlayTimeout.restart();
        }
        onWindowConfirmed: {
            root._launchOverlayActive = false;
            launchOverlayTimeout.stop();
        }
    }

    Components.StreamManager {
        id: streamManager
        shellState: root.state
        onStreamStarted: {
            root.state = "streaming";
            // #99: short haptic confirmation on stream launch.
            if (inputManager)
                inputManager.rumblePulse(120);
        }
        onStreamEnded: {
            Components.NotificationManager.info("stream", "Stream Suspended");
            root.returnToShell();
        }
        onStreamSuspended: {
            Components.NotificationManager.info("stream", "Stream Suspended");
            root.returnToShell();
        }
        onStreamCrashed: attempts => {
            root.state = "reconnecting";
            // #99: stronger pulse to signal the stream dropped / is reconnecting.
            if (inputManager)
                inputManager.rumblePulse(250);
        }
        onStreamFailed: {
            root.state = "idle";
            inputManager.grab();
            // #99: stronger pulse on terminal stream failure.
            if (inputManager)
                inputManager.rumblePulse(250);
        }
        onRequestOverlayShow: msg => {
            if (root._layout)
                root._layout.overlay.show(msg);
        }
        onRequestOverlayHide: {
            if (root._layout)
                root._layout.overlay.hide();
        }
        onRequestInputRelease: inputManager.release()
        onRequestInputHandoff: inputManager.handoff()
        onRequestInputGrab: inputManager.grab()
        onSessionConflictDetected: (runningApp, hostName) => {
            if (root._layout) {
                root._layout.sessionDialog.runningApp = runningApp;
                root._layout.sessionDialog.hostName = hostName;
                root._layout.sessionDialog.opened = true;
            }
        }
        onSessionCheckCancelled: {
            root.state = "idle";
            inputManager.grab();
            if (root._layout)
                root._layout.focusHome();
        }
    }

    function forceQuit() {
        streamManager.forceKill();
        if (root.state === "appRunning")
            closeAndReturnToShell();
        appLifecycle.runningAppClass = "";
        root.state = "idle";
        inputManager.grab();
        root._launchOverlayActive = false;
        launchOverlayTimeout.stop();
        if (root._layout) {
            root._layout.overlay.hide();
            root._layout.sessionDialog.opened = false;
            root._layout.navDrawer.opened = false;
            root._layout.settingsPanel.visible = false;
            root._layout.notificationCenter.opened = false;
            root._layout.powerOverlay.opened = false;
            root._layout.focusHome();
        }
    }

    function returnToShell() {
        root.state = "idle";
        inputManager.grab();
        root._resetIdleTimer();
        root._launchOverlayActive = false;
        launchOverlayTimeout.stop();
        if (root._layout) {
            root._layout.overlay.hide();
            root._layout.settingsPanel.visible = false;
            root._layout.powerOverlay.opened = false;
            root._layout.focusHome();
        }
    }

    function closeAndReturnToShell() {
        appLifecycle.closeApp();
        returnToShell();
    }

    // Full reset to a clean home screen. Triggered by intent:home-hold — the
    // gamepad Home-hold and the keyboard Super+Backspace. Over a running app it
    // returns to the shell; on the home screen it dismisses every
    // overlay/drawer and refocuses Home.
    function resetToHome() {
        if (root.state === "appRunning") {
            root.returnToShell();
        } else if (root.state === "idle" && root._layout) {
            root._layout.navDrawer.opened = false;
            root._layout.settingsPanel.visible = false;
            root._layout.notificationCenter.opened = false;
            root._layout.powerOverlay.opened = false;
            root._layout.focusHome();
        }
    }

    Variants {
        model: Quickshell.screens

        PanelWindow {
            required property var modelData
            screen: modelData
            // Map the shell surface over an app while a drawer is open OR still
            // animating, so the close slide-out plays before the window unmaps
            // (otherwise the drawer vanishes in place over an app). Key off the
            // drawers' `.active` bool — NOT `.visible`, which Qt couples to
            // parent-chain visibility and would break this very binding.
            visible: (root.state !== "appRunning" && root.state !== "streaming" && root.state !== "reconnecting" && root.state !== "launching") || root.overlayDrawerOpen || layout.sessionQam.active || layout.overlayNavDrawer.active

            anchors {
                top: true
                bottom: true
                left: true
                right: true
            }

            color: root.state === "appRunning" ? "transparent" : Components.Theme.background
            // Exclusive keyboard focus so non-Hyprland-bound keys (arrows,
            // Enter, Esc, etc.) reach focused QML widgets. Without this,
            // the compositor gives keyboard input to whatever non-layer
            // window happens to have focus.
            WlrLayershell.keyboardFocus: WlrKeyboardFocus.Exclusive

            Binding {
                target: Components.NotificationManager
                property: "shellVisible"
                value: root.state === "idle"
            }

            Components.ShellLayout {
                id: layout
                anchors.fill: parent
                Component.onCompleted: {
                    root._layout = layout;
                    root._layout.focusHome();
                }
                shellState: root.state
                runningWindows: appLifecycle.runningWindows
                pads: inputManager.pads
                runningAppClass: appLifecycle.runningAppClass
                overlayDrawerOpen: root.overlayDrawerOpen
                avSystemOn: avController.systemOn
                onApplicationsChanged: root._applications = applications
                onStreamRequested: target => {
                    root.state = "launching";
                    avController.forceWake();
                    streamManager.launch(target);
                }
                onStreamQuitRequested: target => {
                    let argv = Components.StreamProviders.active.quitArgs(target);
                    if (argv.length > 0) {
                        streamQuitProc.command = argv;
                        streamQuitProc.running = true;
                    }
                }
                onAppLaunchRequested: app => appLifecycle.checkAndLaunchApp(app)
                onAppFocusRequested: address => appLifecycle.focusByAddress(address)
                onAppCloseRequested: address => appLifecycle.closeByAddress(address)
                onReturnToShellRequested: root.returnToShell()
                onUserActivity: {
                    root._resetIdleTimer();
                    root.userActivityDetected();
                }
                onOverlayDrawerClosed: {
                    root.overlayDrawerOpen = false;
                }

                Connections {
                    target: layout.sessionDialog
                    function onResumeRequested() {
                        layout.sessionDialog.opened = false;
                        streamManager.resumeSession();
                    }
                    function onQuitRequested() {
                        layout.sessionDialog.opened = false;
                        streamManager.quitAndRelaunch();
                    }
                    function onCancelled() {
                        layout.sessionDialog.opened = false;
                        streamManager.cancelSessionCheck();
                    }
                }

                // === OLED auto-dim overlay (issue #143) ===
                // Instantiated once per screen window so it covers the full
                // panel. The dim-delay timer inside DimOverlay is independent of
                // shellIdleTimer (suspend path) and reset by any activity:
                //   - Wayland key/gamepad nav: ShellLayout.userActivity → root.userActivityDetected
                //   - Controller daemon wake:  InputManager.controllerWake → root.userActivityDetected
                //   - Gamepad/keyboard intents: InputManager.onIntent* → root.userActivityDetected
                //     (Home/Menu/Settings/Power + deep-links — so nav-only input cannot dim mid-use)
                // Using root.userActivityDetected (a signal on ShellRoot, the QML
                // document root) avoids id-scope problems with Variants delegates.
                Components.DimOverlay {
                    id: dimOverlay
                }

                Connections {
                    target: root
                    function onUserActivityDetected() {
                        dimOverlay.resetDimTimer();
                    }
                }
            }
        }
    }

    // Screenshot-flash overlay (#166). A dedicated layer-shell window on the
    // Overlay layer, so the flash shows even over a fullscreen app or stream:
    // the main shell window is hidden in those states, so a flash parented to it
    // never renders. Mapped ONLY for the duration of a flash — an always-present
    // overlay surface would block Hyprland's direct scanout and hurt game/stream
    // latency. Click-through (empty input mask) and no keyboard focus, so it
    // never intercepts input.
    Variants {
        model: Quickshell.screens

        PanelWindow {
            id: flashWindow
            required property var modelData
            screen: modelData
            visible: false

            anchors {
                top: true
                bottom: true
                left: true
                right: true
            }

            color: "transparent"
            exclusionMode: ExclusionMode.Ignore
            WlrLayershell.layer: WlrLayer.Overlay
            WlrLayershell.keyboardFocus: WlrKeyboardFocus.None
            // Empty input mask -> fully click-through; input passes to the app
            // or shell beneath.
            mask: Region {}

            Components.ScreenshotFlash {
                id: screenshotFlash
                anchors.fill: parent
                onFinished: flashWindow.visible = false
            }

            Connections {
                target: inputManager
                function onScreenshotFlash() {
                    flashWindow.visible = true;
                    screenshotFlash.flash();
                }
            }
        }
    }

    // Launching overlay (#193). A dedicated Overlay-layer window, like the
    // screenshot flash: during a launch the main shell surface is unmapped
    // (`visible:false` in appRunning), so an opaque "Launching…" surface parented
    // to it would never render and the previously-open app would bleed through
    // the ~2s window-detect gap. This window is opaque (covers that app), sits on
    // the Overlay layer (above even a fullscreen lingering app), and is mapped
    // only while a launch is in flight. Click-through + no keyboard focus.
    Variants {
        model: Quickshell.screens

        PanelWindow {
            required property var modelData
            screen: modelData
            visible: root._launchOverlayActive

            anchors {
                top: true
                bottom: true
                left: true
                right: true
            }

            color: "transparent"
            exclusionMode: ExclusionMode.Ignore
            WlrLayershell.layer: WlrLayer.Overlay
            WlrLayershell.keyboardFocus: WlrKeyboardFocus.None
            mask: Region {}

            Components.LaunchOverlay {
                anchors.fill: parent
                appName: root._launchAppName
                appIcon: root._launchAppIcon
            }
        }
    }
}
