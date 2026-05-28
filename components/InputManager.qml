import Quickshell.Io
import QtQuick

// IPC protocol: see docs/IPC_PROTOCOL.md
// Commands used: grab, release, subscribe
// Events handled: combo:force-quit, combo:end-session, combo:suspend-stream, input-mode:*, controller-wake, controller-disconnected, home-press, combo:home-hold, buttons:*
//
// Also owns keyboard Meta/Super key tap-vs-hold detection so the keyboard
// and controller Home button feed the same homePressed/homeHeld signals.
Item {
    id: root

    signal forceQuitRequested
    signal endSessionRequested
    signal suspendStreamRequested
    signal inputModeChanged(string mode)
    signal controllerWake
    signal controllerDisconnected
    signal homePressed
    signal homeHeld

    // Live state for debug overlays. Updated from socket `buttons:` lines
    // (controller) and from handleMetaPress/Release (keyboard).
    property string currentControllerCombo: ""
    property string currentKey: ""

    // Meta key tap-vs-hold tracking.
    property int metaHoldThreshold: 400
    property bool _metaPressed: false
    property bool _metaHeld: false

    function grab() {
        inputGrab.running = true;
    }
    function release() {
        inputRelease.running = true;
    }
    function startListening() {
        comboListener.running = true;
    }
    function endSession() {
        endSessionProc.running = true;
    }

    function isMetaKey(key) {
        return key === Qt.Key_Meta || key === Qt.Key_Super_L || key === Qt.Key_Super_R;
    }

    // Forwarded from a focused FocusScope on Meta key press. Auto-repeat
    // events are ignored so holding doesn't keep restarting the hold timer.
    function handleMetaPress(isAutoRepeat) {
        if (isAutoRepeat)
            return;
        root._metaPressed = true;
        root._metaHeld = false;
        root.currentKey = "Meta";
        metaHoldTimer.restart();
    }

    // Forwarded from a focused FocusScope on Meta key release. Fires
    // homePressed if the timer hasn't elapsed (tap); otherwise the hold
    // signal was already emitted by the timer.
    function handleMetaRelease(isAutoRepeat) {
        if (isAutoRepeat)
            return;
        if (root._metaPressed && !root._metaHeld) {
            metaHoldTimer.stop();
            root.homePressed();
        }
        root._metaPressed = false;
        root._metaHeld = false;
        root.currentKey = "";
    }

    // For Qt.Key_HomePage — emitted by multimedia keyboards and the
    // gamepad daemon's controller-Home uinput emission. Instant tap;
    // hold semantics for controller come via the socket combo:home-hold.
    function simulateHomeTap() {
        root.homePressed();
    }

    Timer {
        id: metaHoldTimer
        interval: root.metaHoldThreshold
        repeat: false
        onTriggered: {
            if (root._metaPressed) {
                root._metaHeld = true;
                root.homeHeld();
            }
        }
    }

    Process {
        id: inputGrab
        command: ["python3", "-c", "import socket,os; s=socket.socket(socket.AF_UNIX,socket.SOCK_STREAM); s.connect(os.environ.get('GAME_SHELL_SOCK','/run/user/'+str(os.getuid())+'/game-shell-input.sock')); s.sendall(b'grab\\n'); print(s.recv(64).decode().strip()); s.close()"]
    }

    Process {
        id: inputRelease
        command: ["python3", "-c", "import socket,os; s=socket.socket(socket.AF_UNIX,socket.SOCK_STREAM); s.connect(os.environ.get('GAME_SHELL_SOCK','/run/user/'+str(os.getuid())+'/game-shell-input.sock')); s.sendall(b'release\\n'); print(s.recv(64).decode().strip()); s.close()"]
    }

    Process {
        id: endSessionProc
        command: ["/usr/local/bin/end-game-session"]
    }

    Process {
        id: comboListener
        command: ["python3", "-c", "import socket,os;s=socket.socket(socket.AF_UNIX,socket.SOCK_STREAM);s.connect(os.environ.get('GAME_SHELL_SOCK','/run/user/'+str(os.getuid())+'/game-shell-input.sock'));s.sendall(b'subscribe\\n');[print(l,flush=True) for d in iter(lambda:s.recv(1024),b'') for l in d.decode().splitlines()]"]
        stdout: SplitParser {
            onRead: line => {
                if (line === "combo:force-quit")
                    root.forceQuitRequested();
                else if (line === "combo:end-session")
                    root.endSessionRequested();
                else if (line === "combo:suspend-stream")
                    root.suspendStreamRequested();
                else if (line === "input-mode:mouse") {
                    Theme.mouseMode = true;
                    root.inputModeChanged("mouse");
                } else if (line === "input-mode:controller") {
                    Theme.mouseMode = false;
                    root.inputModeChanged("controller");
                } else if (line === "controller-wake") {
                    root.controllerWake();
                    NotificationManager.info("controller", "Controller Connected");
                } else if (line === "controller-disconnected") {
                    root.controllerDisconnected();
                    NotificationManager.warn("controller", "Controller Disconnected");
                } else if (line === "home-press")
                    root.homePressed();
                else if (line === "combo:home-hold")
                    root.homeHeld();
                else if (line.startsWith("buttons:"))
                    root.currentControllerCombo = line.substring(8).trim();
            }
        }
        onExited: {
            comboReconnect.start();
        }
    }

    Timer {
        id: comboReconnect
        interval: 2000
        onTriggered: {
            comboListener.running = true;
        }
    }
}
