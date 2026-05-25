import QtQuick
import QtQuick.Layouts
import Quickshell.Io

Item {
    id: root

    property var targets: []
    property var applications: []
    property string shellState: "idle"

    signal streamRequested(var target)
    signal appLaunchRequested(var app)
    signal settingsRequested()

    Process {
        id: loadApps
        command: ["python3", "-c", `
import os, json, configparser
apps = []
seen = set()
for d in ['/usr/share/applications', os.path.expanduser('~/.local/share/applications')]:
    if not os.path.isdir(d): continue
    for f in sorted(os.listdir(d)):
        if not f.endswith('.desktop'): continue
        cp = configparser.ConfigParser(interpolation=None)
        cp.read(os.path.join(d, f))
        if not cp.has_section('Desktop Entry'): continue
        if cp.get('Desktop Entry', 'NoDisplay', fallback='false').lower() == 'true': continue
        if cp.get('Desktop Entry', 'Hidden', fallback='false').lower() == 'true': continue
        if cp.get('Desktop Entry', 'Type', fallback='') != 'Application': continue
        name = cp.get('Desktop Entry', 'Name', fallback='')
        if not name or name in seen: continue
        seen.add(name)
        ex = cp.get('Desktop Entry', 'Exec', fallback='')
        for tok in ['%u','%U','%f','%F','%i','%c','%k']:
            ex = ex.replace(tok, '')
        apps.append({'name': name, 'exec': ex.strip(), 'icon': cp.get('Desktop Entry', 'Icon', fallback=''), 'comment': cp.get('Desktop Entry', 'Comment', fallback='')})
apps.sort(key=lambda x: x['name'].lower())
print(json.dumps(apps))
`]
        stdout: SplitParser {
            onRead: (line) => {
                try { root.applications = JSON.parse(line) }
                catch(e) { console.log("Failed to parse apps:", e) }
            }
        }
    }

    Component.onCompleted: { loadApps.running = true }

    Process {
        id: appLauncher
        property string cmd: ""
        command: ["hyprctl", "dispatch", "exec", cmd]
    }

    function launchApp(app) {
        appLauncher.cmd = app.exec || app.name
        appLauncher.command = ["hyprctl", "dispatch", "exec", app.exec || app.name]
        appLauncher.running = true
    }

    ColumnLayout {
        anchors.fill: parent
        anchors.margins: Theme.padding
        spacing: 24

        // Moonlight row
        Text {
            text: "Moonlight"
            font.pixelSize: Theme.fontTitle
            font.bold: true
            color: Theme.text
        }

        ListView {
            id: moonlightRow
            Layout.fillWidth: true
            Layout.preferredHeight: Theme.rowHeight
            orientation: ListView.Horizontal
            spacing: Theme.cardSpacing
            clip: true
            focus: true

            model: root.targets

            delegate: StreamCard {
                required property int index
                required property var modelData
                height: moonlightRow.height - 20
                width: Theme.cardWidth
                target: modelData
                focus: index === moonlightRow.currentIndex
                onActivated: root.streamRequested(modelData)

                MouseArea {
                    anchors.fill: parent
                    hoverEnabled: true
                    cursorShape: Qt.PointingHandCursor
                    onClicked: {
                        moonlightRow.currentIndex = parent.index
                        parent.forceActiveFocus()
                    }
                    onDoubleClicked: root.streamRequested(parent.modelData)
                }
            }

            Keys.onReturnPressed: {
                if (moonlightRow.currentItem)
                    root.streamRequested(moonlightRow.currentItem.modelData)
            }
            Keys.onDownPressed: appsRow.forceActiveFocus()
            Keys.onEscapePressed: root.settingsRequested()
        }

        // Applications row
        Text {
            text: "Applications"
            font.pixelSize: Theme.fontTitle
            font.bold: true
            color: Theme.text
        }

        ListView {
            id: appsRow
            Layout.fillWidth: true
            Layout.preferredHeight: Theme.rowHeight
            orientation: ListView.Horizontal
            spacing: Theme.cardSpacing
            clip: true

            model: root.applications

            delegate: AppCard {
                required property int index
                required property var modelData
                height: appsRow.height - 20
                width: Theme.cardWidth
                app: modelData
                focus: index === appsRow.currentIndex
                onActivated: root.launchApp(modelData)

                MouseArea {
                    anchors.fill: parent
                    hoverEnabled: true
                    cursorShape: Qt.PointingHandCursor
                    onClicked: {
                        appsRow.currentIndex = parent.index
                        parent.forceActiveFocus()
                    }
                    onDoubleClicked: root.launchApp(parent.modelData)
                }
            }

            Keys.onReturnPressed: {
                if (appsRow.currentItem)
                    root.launchApp(appsRow.currentItem.modelData)
            }
            Keys.onUpPressed: moonlightRow.forceActiveFocus()
            Keys.onEscapePressed: root.settingsRequested()
        }

        Item { Layout.fillHeight: true }

        Text {
            text: "A: Launch  |  B: Settings  |  ←→: Scroll  |  ↑↓: Switch Row"
            font.pixelSize: Theme.fontHint
            color: Theme.textDim
            Layout.alignment: Qt.AlignHCenter
            Layout.bottomMargin: 16
        }
    }
}
