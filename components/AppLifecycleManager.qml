import Quickshell.Io
import QtQuick

Item {
    id: root

    property string runningAppClass: ""
    property var runningWindows: []
    property var applications: []
    property string shellState: ""

    property var _prelaunchClasses: []
    property var _pendingApp: null

    signal appLaunched()
    signal appClosed()

    function launchDesktopApp(app) {
        runningAppClass = ""
        snapshotClients.running = true
        appRunner.command = ["hyprctl", "dispatch", "exec", app.exec || app.name]
        appRunner.running = true
        detectNewWindow.restart()
        appLaunched()
    }

    function checkAndLaunchApp(app) {
        _pendingApp = app
        windowQuery.running = true
    }

    function closeApp() {
        if (runningAppClass !== "") {
            closeAppWindow.appClass = runningAppClass
            closeAppWindow.running = true
        }
    }

    function focusApp(windowClass) {
        runningAppClass = windowClass
        focusWindow.windowClass = windowClass
        focusWindow.running = true
        appLaunched()
    }

    Process {
        id: closeAppWindow
        property string appClass: ""
        command: ["hyprctl", "dispatch", "closewindow", "class:" + appClass]
    }

    Process {
        id: appRunner
        command: ["echo"]
    }

    HyprctlClients {
        id: snapshotClients
        onClientsReceived: (clients) => {
            root._prelaunchClasses = clients.map(c => c["class"])
        }
        onErrorOccurred: { root._prelaunchClasses = [] }
    }

    HyprctlClients {
        id: detectClient
        onClientsReceived: (clients) => {
            for (let i = 0; i < clients.length; i++) {
                if (root._prelaunchClasses.indexOf(clients[i]["class"]) < 0 && clients[i]["class"] !== "") {
                    root.runningAppClass = clients[i]["class"]
                    break
                }
            }
        }
    }

    Timer {
        id: detectNewWindow
        interval: 2000
        onTriggered: { detectClient.running = true }
    }

    HyprctlClients {
        id: windowQuery
        onClientsReceived: (clients) => {
            root._handleWindowQueryResult(clients)
        }
        onErrorOccurred: { root._handleWindowQueryResult([]) }
    }

    Process {
        id: focusWindow
        property string windowClass: ""
        command: ["hyprctl", "dispatch", "focuswindow", "class:" + windowClass]
        onExited: (exitCode) => {
            if (exitCode !== 0 && root.shellState === "appRunning")
                root.appClosed()
        }
    }

    function _handleWindowQueryResult(clients) {
        let app = _pendingApp
        if (!app) return
        _pendingApp = null

        let matchClass = (app.wmClass || "").toLowerCase()
        let matchExec = (app.exec || "").split(/\s/)[0].split("/").pop().toLowerCase()
        let matchName = (app.name || "").toLowerCase()

        for (let i = 0; i < clients.length; i++) {
            let cls = (clients[i]["class"] || "").toLowerCase()
            let initCls = (clients[i]["initialClass"] || "").toLowerCase()
            if ((matchClass && (cls === matchClass || initCls === matchClass)) ||
                (matchExec && (cls === matchExec || initCls === matchExec)) ||
                (matchName && (cls === matchName || initCls === matchName))) {
                root.runningAppClass = clients[i]["class"]
                focusWindow.windowClass = clients[i]["class"]
                focusWindow.running = true
                appLaunched()
                return
            }
        }

        launchDesktopApp(app)
    }

    HyprctlClients {
        id: windowPoller
        onClientsReceived: (clients) => {
            let apps = (root.applications || [])
            let windows = []
            for (let i = 0; i < clients.length; i++) {
                let c = clients[i]
                if (c["class"] && c["class"] !== "" && c["class"].indexOf("quickshell") < 0) {
                    let cls = c["class"]
                    let iconName = (c["initialClass"] || cls).toLowerCase()
                    let appIcon = iconName
                    for (let j = 0; j < apps.length; j++) {
                        let a = apps[j]
                        let wm = (a.wmClass || "").toLowerCase()
                        let ex = (a.exec || "").split(/\s/)[0].split("/").pop().toLowerCase()
                        if (wm === cls.toLowerCase() || ex === cls.toLowerCase() || (a.name || "").toLowerCase() === cls.toLowerCase()) {
                            appIcon = a.icon || iconName
                            break
                        }
                    }
                    windows.push({
                        windowClass: cls,
                        title: c["title"] || cls,
                        name: c["title"] || cls,
                        icon: appIcon,
                        exec: ""
                    })
                }
            }
            root.runningWindows = windows

            if (root.shellState === "appRunning" && root.runningAppClass !== "") {
                let found = false
                for (let i = 0; i < root.runningWindows.length; i++) {
                    if (root.runningWindows[i].windowClass === root.runningAppClass) {
                        found = true
                        break
                    }
                }
                if (!found) root.appClosed()
            }
        }
        onErrorOccurred: { root.runningWindows = [] }
    }

    Timer {
        id: windowPollTimer
        interval: root.shellState === "appRunning" ? 2000 : 5000
        running: root.shellState === "idle" || root.shellState === "appRunning"
        repeat: true
        triggeredOnStart: true
        onTriggered: { if (!windowPoller.running) windowPoller.running = true }
    }
}
