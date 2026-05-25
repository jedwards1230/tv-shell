import Quickshell
import Quickshell.Io
import Quickshell.Wayland
import QtQuick
import QtQuick.Layouts
import "components" as Components

ShellRoot {
    id: root

    property string state: "idle"
    property var currentTarget: null
    property int crashCount: 0
    property var targets: []

    Process {
        id: loadTargets
        command: ["python3", "-c", "import json,yaml,os; f=open(os.environ.get('GAME_SHELL_TARGETS','/opt/game-shell/targets.yaml')); d=yaml.safe_load(f); print(json.dumps(d.get('targets',[])))"]
        stdout: SplitParser {
            onRead: (line) => {
                try { root.targets = JSON.parse(line) }
                catch(e) { console.log("Failed to parse targets:", e) }
            }
        }
    }

    Component.onCompleted: { loadTargets.running = true }

    Timer {
        id: crashResetTimer
        interval: 300000
        running: root.state === "streaming"
        onTriggered: { root.crashCount = 0 }
    }

    Process {
        id: avWake
        command: ["/usr/local/bin/living-room-cec", "on"]
        onExited: (exitCode, exitStatus) => {
            if (root.state === "launching") launchMoonlight()
        }
    }

    Process {
        id: moonlight
        onExited: (exitCode, exitStatus) => {
            if (exitCode === 0) {
                root.state = "idle"
                overlay.hide()
                grabInput()
            } else {
                root.crashCount++
                if (root.crashCount < 5) {
                    root.state = "reconnecting"
                    overlay.show("Reconnecting...")
                    overlay.attemptCount = root.crashCount
                    reconnectTimer.start()
                } else {
                    root.state = "idle"
                    overlay.show("Stream failed after 5 attempts")
                    errorDismissTimer.start()
                    grabInput()
                }
            }
        }
    }

    Timer {
        id: reconnectTimer
        interval: 2000
        onTriggered: launchMoonlight()
    }

    Timer {
        id: errorDismissTimer
        interval: 5000
        onTriggered: { overlay.hide() }
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
        id: endSession
        command: ["/usr/local/bin/end-game-session"]
    }

    function launchStream(target) {
        root.currentTarget = target
        root.state = "launching"
        root.crashCount = 0
        overlay.show("Launching " + target.name + "...")
        avWake.running = true
    }

    function launchMoonlight() {
        let args = ["moonlight", "stream", currentTarget.host, currentTarget.app]
        if (currentTarget.resolution === "3840x2160") args.push("--4k")
        if (currentTarget.fps) { args.push("--fps"); args.push(String(currentTarget.fps)) }
        if (currentTarget.hdr) args.push("--hdr")
        if (currentTarget.codec) { args.push("--video-codec"); args.push(currentTarget.codec) }
        args.push("--display-mode", "borderless")
        args.push("--no-quit-after")
        args.push("--no-frame-pacing")

        moonlight.command = args
        releaseInput()
        root.state = "streaming"
        overlay.hide()
        moonlight.running = true
    }

    function grabInput() { inputGrab.running = true }
    function releaseInput() { inputRelease.running = true }

    Variants {
        model: Quickshell.screens

        PanelWindow {
            required property var modelData
            screen: modelData

            anchors {
                top: true
                bottom: true
                left: true
                right: true
            }

            color: Components.Theme.background
            focusable: true

            ColumnLayout {
                anchors.fill: parent
                spacing: 0

                Components.StatusBar {
                    Layout.fillWidth: true
                    shellState: root.state
                    onSettingsClicked: {
                        settingsPanel.visible = true
                        settingsPanel.forceActiveFocus()
                    }
                }

                Item {
                    Layout.fillWidth: true
                    Layout.fillHeight: true

                    Components.HomeScreen {
                        id: homeScreen
                        anchors.fill: parent
                        visible: root.state === "idle"
                        targets: root.targets
                        shellState: root.state
                        focus: root.state === "idle" && !settingsPanel.visible

                        onStreamRequested: (target) => root.launchStream(target)
                        onSettingsRequested: {
                            settingsPanel.visible = true
                            settingsPanel.forceActiveFocus()
                        }
                    }

                    Components.SettingsPanel {
                        id: settingsPanel
                        anchors.fill: parent
                        onClosed: {
                            settingsPanel.visible = false
                            homeScreen.forceActiveFocus()
                        }
                    }

                    Components.StreamOverlay {
                        id: overlay
                        anchors.fill: parent
                    }
                }
            }
        }
    }
}
