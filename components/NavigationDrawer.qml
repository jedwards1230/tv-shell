import QtQuick
import QtQuick.Layouts
import Quickshell.Io

Drawer {
    id: root
    edge: "left"
    drawerWidth: 960

    signal forceQuitRequested()
    signal settingsRequested()

    property string _grabState: "grabbed"
    property int _focusSection: 0  // 0=nav, 1=bottom

    Process {
        id: forceQuitProcess
        command: ["bash", "-c", "pkill -f moonlight; pkill -f steam; true"]
    }

    Process {
        id: grabProcess
        command: ["python3", "-c", "import socket,os; s=socket.socket(socket.AF_UNIX,socket.SOCK_STREAM); s.connect(os.environ.get('GAME_SHELL_SOCK','/run/user/'+str(os.getuid())+'/game-shell-input.sock')); s.sendall(b'grab\\n'); print(s.recv(64).decode().strip()); s.close()"]
        stdout: SplitParser {
            onRead: (line) => { if (line.trim() === "ok") root._grabState = "grabbed" }
        }
    }

    Process {
        id: releaseProcess
        command: ["python3", "-c", "import socket,os; s=socket.socket(socket.AF_UNIX,socket.SOCK_STREAM); s.connect(os.environ.get('GAME_SHELL_SOCK','/run/user/'+str(os.getuid())+'/game-shell-input.sock')); s.sendall(b'release\\n'); print(s.recv(64).decode().strip()); s.close()"]
        stdout: SplitParser {
            onRead: (line) => { if (line.trim() === "ok") root._grabState = "released" }
        }
    }

    Process {
        id: statusProcess
        command: ["python3", "-c", "import socket,os; s=socket.socket(socket.AF_UNIX,socket.SOCK_STREAM); s.connect(os.environ.get('GAME_SHELL_SOCK','/run/user/'+str(os.getuid())+'/game-shell-input.sock')); s.sendall(b'status\\n'); print(s.recv(64).decode().strip()); s.close()"]
        stdout: SplitParser {
            onRead: (line) => {
                let parts = line.trim().split(":")
                if (parts.length >= 2)
                    root._grabState = parts[1]
            }
        }
    }

    onOpenedChanged: {
        if (opened) {
            navList.currentIndex = 0
            root._focusSection = 0
            navFocusTimer.restart()
            statusProcess.running = true
        }
    }

    Timer {
        id: navFocusTimer
        interval: 50
        onTriggered: navList.forceActiveFocus()
    }

    ColumnLayout {
        anchors.fill: parent
        spacing: 0

        // === Clock + Date Header ===
        Item {
            Layout.fillWidth: true
            Layout.preferredHeight: 280

            ColumnLayout {
                anchors.centerIn: parent
                spacing: 12

                Text {
                    id: drawerClock
                    font.pixelSize: Theme.fontHero * 0.7
                    font.bold: true
                    color: Theme.textPrimary
                    Layout.alignment: Qt.AlignHCenter

                    Timer {
                        interval: 1000
                        running: root.opened
                        repeat: true
                        triggeredOnStart: true
                        onTriggered: {
                            let now = new Date()
                            drawerClock.text = now.toLocaleTimeString(Qt.locale(), "h:mm AP")
                        }
                    }
                }

                Text {
                    id: drawerDate
                    font.pixelSize: Theme.fontBody
                    color: Theme.textSecondary
                    Layout.alignment: Qt.AlignHCenter

                    Timer {
                        interval: 60000
                        running: root.opened
                        repeat: true
                        triggeredOnStart: true
                        onTriggered: {
                            let now = new Date()
                            drawerDate.text = now.toLocaleDateString(Qt.locale(), "dddd, MMMM d")
                        }
                    }
                }
            }
        }

        Rectangle { Layout.fillWidth: true; height: 2; color: Theme.surfaceBorder }

        // === Top Navigation Items ===
        ListView {
            id: navList
            Layout.fillWidth: true
            Layout.preferredHeight: contentHeight
            model: [
                { label: "Home",       icon: "\u{1F3E0}", action: "home" },
                { label: "Settings",   icon: "⚙",    action: "settings" },
                { label: "Force Quit", icon: "⏹",    action: "forceQuit" }
            ]
            focus: root._focusSection === 0
            interactive: false
            currentIndex: 0

            delegate: Rectangle {
                required property int index
                required property var modelData
                width: navList.width
                height: 120
                color: navList.currentIndex === index && navList.activeFocus ? Theme.surfaceHover : "transparent"
                Behavior on color { ColorAnimation { duration: 150 } }

                Rectangle {
                    anchors.left: parent.left
                    anchors.verticalCenter: parent.verticalCenter
                    width: 4; height: parent.height - 16; radius: 2
                    color: Theme.focusBorder
                    visible: navList.currentIndex === index && navList.activeFocus
                }

                RowLayout {
                    anchors.fill: parent
                    anchors.leftMargin: 48
                    anchors.rightMargin: 48
                    spacing: 24

                    Text {
                        text: modelData.icon
                        font.pixelSize: Theme.fontBody
                        Layout.preferredWidth: 64
                        horizontalAlignment: Text.AlignHCenter
                    }
                    Text {
                        text: modelData.label
                        font.pixelSize: Theme.fontBody
                        color: modelData.action === "forceQuit" ? Theme.crimson : Theme.textPrimary
                        font.bold: modelData.action === "forceQuit"
                        Layout.fillWidth: true
                    }
                }

                MouseArea {
                    anchors.fill: parent
                    hoverEnabled: true
                    cursorShape: Qt.PointingHandCursor
                    onClicked: root._activateNav(index)
                }
            }

            Keys.onDownPressed: {
                if (currentIndex < count - 1) currentIndex++
                else { root._focusSection = 1; bottomList.forceActiveFocus() }
            }
            Keys.onUpPressed: { if (currentIndex > 0) currentIndex-- }
            Keys.onReturnPressed: root._activateNav(currentIndex)
        }

        // === Spacer — pushes bottom items down ===
        Item { Layout.fillWidth: true; Layout.fillHeight: true }

        // === Bottom Section: Controller + Settings ===
        Rectangle { Layout.fillWidth: true; height: 2; color: Theme.surfaceBorder }

        ListView {
            id: bottomList
            Layout.fillWidth: true
            Layout.preferredHeight: contentHeight
            model: [
                { label: "Controller", icon: "\u{1F3AE}", action: "toggleGrab", type: "toggle" },
                { label: "Settings",   icon: "⚙",    action: "settings",   type: "action" }
            ]
            focus: root._focusSection === 1
            interactive: false
            currentIndex: 0

            delegate: Rectangle {
                required property int index
                required property var modelData
                width: bottomList.width
                height: 120
                color: bottomList.currentIndex === index && bottomList.activeFocus ? Theme.surfaceHover : "transparent"
                Behavior on color { ColorAnimation { duration: 150 } }

                Rectangle {
                    anchors.left: parent.left
                    anchors.verticalCenter: parent.verticalCenter
                    width: 4; height: parent.height - 16; radius: 2
                    color: Theme.focusBorder
                    visible: bottomList.currentIndex === index && bottomList.activeFocus
                }

                RowLayout {
                    anchors.fill: parent
                    anchors.leftMargin: 48
                    anchors.rightMargin: 48
                    spacing: 24

                    Text {
                        text: modelData.icon
                        font.pixelSize: Theme.fontBody
                        Layout.preferredWidth: 64
                        horizontalAlignment: Text.AlignHCenter
                    }

                    ColumnLayout {
                        Layout.fillWidth: true
                        spacing: 4

                        Text {
                            text: modelData.label
                            font.pixelSize: Theme.fontBody
                            color: Theme.textPrimary
                        }
                        Text {
                            visible: modelData.type === "toggle"
                            text: root._grabState === "grabbed" ? "Currently capturing keys..." : "Key capture off"
                            font.pixelSize: Theme.fontSmall
                            color: Theme.textMuted
                        }
                    }
                }

                MouseArea {
                    anchors.fill: parent
                    hoverEnabled: true
                    cursorShape: Qt.PointingHandCursor
                    onClicked: root._activateBottom(index)
                }
            }

            Keys.onUpPressed: {
                if (currentIndex > 0) currentIndex--
                else { root._focusSection = 0; navList.forceActiveFocus() }
            }
            Keys.onDownPressed: { if (currentIndex < count - 1) currentIndex++ }
            Keys.onReturnPressed: root._activateBottom(currentIndex)
        }
    }

    function _activateNav(index) {
        let items = navList.model
        if (index < 0 || index >= items.length) return
        switch (items[index].action) {
            case "home":
                root.opened = false; root.closed(); break
            case "settings":
                root.opened = false; root.closed(); root.settingsRequested(); break
            case "forceQuit":
                forceQuitProcess.running = true; root.forceQuitRequested(); break
        }
    }

    function _activateBottom(index) {
        let items = bottomList.model
        if (index < 0 || index >= items.length) return
        switch (items[index].action) {
            case "toggleGrab":
                if (root._grabState === "grabbed") releaseProcess.running = true
                else grabProcess.running = true
                break
            case "settings":
                root.opened = false; root.closed(); root.settingsRequested(); break
        }
    }
}
