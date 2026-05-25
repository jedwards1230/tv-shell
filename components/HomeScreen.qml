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
        // Launch via hyprctl
        appLauncher.command = ["hyprctl", "dispatch", "exec", app.exec || app.name]
        appLauncher.running = true
        // Track in recents
        recentsTracker.command = ["python3", "-c",
            "import json,os,time; p=os.path.expanduser('~/.local/share/game-shell/recents.json'); os.makedirs(os.path.dirname(p),exist_ok=True); " +
            "d=[]; " +
            "try:\n with open(p) as f: d=json.load(f)\nexcept: pass\n" +
            "entry={'name':'" + (app.name||"").replace("'","\\'") + "','exec':'" + (app.exec||"").replace("'","\\'") + "','comment':'" + (app.comment||"").replace("'","\\'") + "','time':time.time()}; " +
            "d=[e for e in d if e.get('name')!=entry['name']]; d.insert(0,entry); d=d[:20]; " +
            "open(p,'w').write(json.dumps(d,indent=2))"
        ]
        recentsTracker.running = true
        // Reload recents after brief delay
        recentsReloadTimer.start()
    }

    Process { id: recentsTracker; command: ["echo"] }
    Timer { id: recentsReloadTimer; interval: 500; onTriggered: loadRecents.running = true }

    ColumnLayout {
        anchors.fill: parent
        anchors.margins: Theme.padding
        spacing: 24

        // Recents row (only if there are recent items)
        Text {
            visible: root.recentApps.length > 0
            text: "Recent"
            font.pixelSize: Theme.fontTitle
            font.bold: true
            color: Theme.textPrimary
        }

        ListView {
            id: recentsRow
            visible: root.recentApps.length > 0
            Layout.fillWidth: true
            Layout.preferredHeight: visible ? Theme.rowHeight : 0
            orientation: ListView.Horizontal
            spacing: Theme.cardSpacing
            clip: true
            focus: visible

            model: root.recentApps

            delegate: AppCard {
                required property int index
                required property var modelData
                height: recentsRow.height - 20
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

        // Moonlight row
        Text {
            text: "Moonlight"
            font.pixelSize: Theme.fontTitle
            font.bold: true
            color: Theme.textPrimary
        }

        ListView {
            id: moonlightRow
            Layout.fillWidth: true
            Layout.preferredHeight: Theme.rowHeight
            orientation: ListView.Horizontal
            spacing: Theme.cardSpacing
            clip: true
            focus: !recentsRow.visible

            model: root.targets

            delegate: StreamCard {
                required property int index
                required property var modelData
                height: moonlightRow.height - 20
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

        // Applications row
        Text {
            text: "Applications"
            font.pixelSize: Theme.fontTitle
            font.bold: true
            color: Theme.textPrimary
        }

        ListView {
            id: appsRow
            Layout.fillWidth: true
            Layout.fillHeight: true
            Layout.minimumHeight: Theme.rowHeight
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
            }

            Keys.onReturnPressed: {
                if (appsRow.currentItem)
                    root.launchApp(appsRow.currentItem.modelData)
            }
            Keys.onUpPressed: moonlightRow.forceActiveFocus()
            Keys.onEscapePressed: root.settingsRequested()
        }

        Text {
            text: "A: Launch  |  B: Settings  |  ←→: Scroll  |  ↑↓: Switch Row"
            font.pixelSize: Theme.fontHint
            color: Theme.textSecondary
            Layout.alignment: Qt.AlignHCenter
            Layout.bottomMargin: 16
        }
    }
}
