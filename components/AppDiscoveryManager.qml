pragma Singleton
import Quickshell.Io
import QtQuick

// Discovers locally installed applications by scanning XDG .desktop entries.
//
// IMPORTANT: root MUST be Item (not QtObject). Quickshell 0.3.0 cannot host
// Process children inside a QtObject singleton, and this manager needs a
// Process to shell out to the desktop-entry scanner.
//
// Single source of truth for the `applications` model consumed by HomeScreen's
// Applications row and by AppLifecycleManager (for window icon/name matching,
// via ShellLayout).
Item {
    id: manager

    // Sorted list of installed apps: [{name, exec, icon, comment, wmClass}]
    property var applications: []
    property bool loading: false

    function refresh() {
        loading = true;
        loadApps.running = true;
    }

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
        apps.append({'name': name, 'exec': ex.strip(), 'icon': cp.get('Desktop Entry', 'Icon', fallback=''), 'comment': cp.get('Desktop Entry', 'Comment', fallback=''), 'wmClass': cp.get('Desktop Entry', 'StartupWMClass', fallback='')})
apps.sort(key=lambda x: x['name'].lower())
print(json.dumps(apps))
`]
        stdout: SplitParser {
            onRead: line => {
                try {
                    manager.applications = JSON.parse(line);
                } catch (e) {
                    console.log("AppDiscoveryManager: failed to parse apps:", e);
                }
                manager.loading = false;
            }
        }
    }

    Component.onCompleted: refresh()
}
