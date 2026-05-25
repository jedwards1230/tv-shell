import QtQuick
import QtQuick.Layouts
import Quickshell.Io

Item {
    id: root

    property var targets: []
    property var applications: []
    property var recentApps: []
    property string shellState: "idle"

    signal streamRequested(var target)
    signal appLaunchRequested(var app)
    signal settingsRequested()

    // Load installed applications
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

    // Load recent launches
    Process {
        id: loadRecents
        command: ["python3", "-c", `
import json, os
path = os.path.expanduser('~/.local/share/game-shell/recents.json')
try:
    with open(path) as f:
        data = json.load(f)
    print(json.dumps(data[:15]))
except:
    print('[]')
`]
        stdout: SplitParser {
            onRead: (line) => {
                try { root.recentApps = JSON.parse(line) }
                catch(e) { root.recentApps = [] }
            }
        }
    }

    Component.onCompleted: {
        loadApps.running = true
        loadRecents.running = true
    }

    // App launcher + recents tracker
    Process {
        id: appLauncher
        command: ["echo"]
    }

    function launchApp(app) {
        appLauncher.command = ["hyprctl", "dispatch", "exec", app.exec || app.name]
        appLauncher.running = true
        recentsTracker.command = ["python3", "-c",
            "import json,os,time; p=os.path.expanduser('~/.local/share/game-shell/recents.json'); os.makedirs(os.path.dirname(p),exist_ok=True); " +
            "d=[]; " +
            "try:\n with open(p) as f: d=json.load(f)\nexcept: pass\n" +
            "entry={'name':'" + (app.name||"").replace("'","\\'") + "','exec':'" + (app.exec||"").replace("'","\\'") + "','comment':'" + (app.comment||"").replace("'","\\'") + "','time':time.time()}; " +
            "d=[e for e in d if e.get('name')!=entry['name']]; d.insert(0,entry); d=d[:20]; " +
            "open(p,'w').write(json.dumps(d,indent=2))"
        ]
        recentsTracker.running = true
        recentsReloadTimer.start()
    }

    Process { id: recentsTracker; command: ["echo"] }
    Timer { id: recentsReloadTimer; interval: 500; onTriggered: loadRecents.running = true }

    ColumnLayout {
        anchors.fill: parent
        anchors.margins: Theme.padding
        spacing: 24

        // === Hero Clock Area ===
        RowLayout {
            Layout.fillWidth: true
            Layout.preferredHeight: 140
            spacing: 32

            // Clock + date (left side)
            ColumnLayout {
                spacing: 4

                Text {
                    id: heroClockText
                    font.pixelSize: Theme.fontHero + 24
                    font.bold: true
                    color: Theme.textPrimary

                    Timer {
                        interval: 1000
                        running: true
                        repeat: true
                        triggeredOnStart: true
                        onTriggered: {
                            let now = new Date()
                            heroClockText.text = now.toLocaleTimeString(Qt.locale(), "h:mm AP")
                        }
                    }
                }

                Text {
                    id: heroDateText
                    font.pixelSize: Theme.fontBody
                    color: Theme.textSecondary

                    Timer {
                        interval: 60000
                        running: true
                        repeat: true
                        triggeredOnStart: true
                        onTriggered: {
                            let now = new Date()
                            heroDateText.text = now.toLocaleDateString(Qt.locale(), "dddd, MMMM d")
                        }
                    }
                }
            }

            Item { Layout.fillWidth: true }

            // Status icons (right side)
            StatusIcons {
                Layout.alignment: Qt.AlignTop | Qt.AlignRight
                onSettingsRequested: root.settingsRequested()
            }
        }

        // === Recents Row ===
        Text {
            visible: root.recentApps.length > 0
            text: "Recent"
            font.pixelSize: Theme.fontTitle
            font.bold: true
            color: Theme.textPrimary
        }

        Item {
            id: recentsContainer
            visible: root.recentApps.length > 0
            Layout.fillWidth: true
            Layout.preferredHeight: visible ? Theme.rowHeight : 0

            ListView {
                id: recentsRow
                anchors.fill: parent
                anchors.topMargin: -16
                anchors.bottomMargin: -16
                orientation: ListView.Horizontal
                spacing: Theme.cardSpacing
                focus: visible
                clip: false

                model: root.recentApps

                delegate: AppCard {
                    required property int index
                    required property var modelData
                    height: Theme.cardHeight
                    width: Theme.cardWidth
                    app: modelData
                    focus: index === recentsRow.currentIndex
                    onActivated: root.launchApp(modelData)
                }

                Keys.onReturnPressed: {
                    if (recentsRow.currentItem)
                        root.launchApp(recentsRow.currentItem.modelData)
                }
                Keys.onDownPressed: moonlightRow.forceActiveFocus()
                Keys.onEscapePressed: root.settingsRequested()
            }
        }

        // === Moonlight Row ===
        Text {
            text: "Moonlight"
            font.pixelSize: Theme.fontTitle
            font.bold: true
            color: Theme.textPrimary
        }

        Item {
            Layout.fillWidth: true
            Layout.preferredHeight: Theme.rowHeight

            ListView {
                id: moonlightRow
                anchors.fill: parent
                anchors.topMargin: -16
                anchors.bottomMargin: -16
                orientation: ListView.Horizontal
                spacing: Theme.cardSpacing
                focus: !recentsRow.visible
                clip: false

                model: root.targets

                delegate: StreamCard {
                    required property int index
                    required property var modelData
                    height: Theme.cardHeight
                    width: Theme.cardWidth
                    target: modelData
                    focus: index === moonlightRow.currentIndex
                    onActivated: root.streamRequested(modelData)
                }

                Keys.onReturnPressed: {
                    if (moonlightRow.currentItem)
                        root.streamRequested(moonlightRow.currentItem.modelData)
                }
                Keys.onUpPressed: recentsRow.visible ? recentsRow.forceActiveFocus() : null
                Keys.onDownPressed: appsRow.forceActiveFocus()
                Keys.onEscapePressed: root.settingsRequested()
            }
        }

        // === Applications Row ===
        Text {
            text: "Applications"
            font.pixelSize: Theme.fontTitle
            font.bold: true
            color: Theme.textPrimary
        }

        Item {
            Layout.fillWidth: true
            Layout.fillHeight: true
            Layout.minimumHeight: Theme.rowHeight

            ListView {
                id: appsRow
                anchors.fill: parent
                anchors.topMargin: -16
                anchors.bottomMargin: -16
                orientation: ListView.Horizontal
                spacing: Theme.cardSpacing
                clip: false

                model: root.applications

                delegate: AppCard {
                    required property int index
                    required property var modelData
                    height: Theme.cardHeight
                    width: Theme.cardWidth
                    app: modelData
                    focus: index === appsRow.currentIndex
                    onActivated: root.launchApp(modelData)
                }

                Keys.onReturnPressed: {
                    if (appsRow.currentItem)
                        root.launchApp(appsRow.currentItem.modelData)
                }
                Keys.onUpPressed: moonlightRow.forceActiveFocus()
                Keys.onEscapePressed: root.settingsRequested()
            }
        }

        // === Hint Bar ===
        Text {
            text: "A: Launch  |  B: Settings  |  ←→: Scroll  |  ↑↓: Switch Row"
            font.pixelSize: Theme.fontHint
            color: Theme.textMuted
            Layout.alignment: Qt.AlignHCenter
            Layout.bottomMargin: 16
        }
    }
}
