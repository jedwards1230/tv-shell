import Quickshell.Io
import QtQuick

Item {
    id: root

    property string runningAppClass: ""
    property var runningWindows: []
    // Signature of the last published runningWindows; gate reassignment on it
    // so an unchanged poll doesn't rebuild the home row and drop controller focus.
    property string _runningWindowsSig: ""
    property var applications: []
    property string shellState: ""

    // Last active-window class reported by the daemon's `hypr:activewindow`
    // subscribe event (empty when no window is focused). Mirrors the compositor's
    // focus state without an extra query.
    property string activeWindowClass: ""
    // Last fullscreen state reported by the daemon's `hypr:fullscreen` event.
    property bool activeWindowFullscreen: false

    property var _prelaunchClasses: []
    // Addresses of windows present at snapshot time, used for address-novelty
    // comparison when an openwindow event arrives (#203).
    property var _prelaunchAddresses: []
    property var _pendingApp: null
    property var _launchedApps: Object.create(null)
    property int _maxMisses: 3
    // Address of the window currently tracked as the foreground app; set when an
    // openwindow event confirms a launch, and cleared on appClosed / return to
    // shell.  A stale address here must not suppress future launches.
    property string _foregroundAddress: ""

    // True between a launch being initiated and its window being confirmed
    // mapped — gates windowConfirmed so it fires exactly once per launch and not
    // on every subsequent poll (#193).
    property bool _awaitingWindow: false

    signal appLaunched
    signal appClosed
    // Emitted when the launcher process exits non-zero (app failed to start).
    // shell.qml uses this for an error haptic (#99); the failure is also logged.
    signal appLaunchFailed
    // #193: emitted the moment a local app launch is initiated (carries the app
    // so the launch overlay can show its name/icon) and once the launched
    // window is confirmed mapped (so the overlay can hide).
    signal launchStarted(var app)
    signal windowConfirmed

    // Fire windowConfirmed exactly once per in-flight launch.
    function _confirmWindow() {
        if (root._awaitingWindow) {
            root._awaitingWindow = false;
            root.windowConfirmed();
        }
    }

    function launchDesktopApp(app) {
        runningAppClass = "";
        // Clear any stale foreground address from a previous launch so it can't
        // suppress openwindow matching for this new launch (#203).
        root._foregroundAddress = "";
        // #193: this is the ONLY true fresh-launch path — show the launch overlay
        // here, not in checkAndLaunchApp, so resuming an already-running app (the
        // focus-existing-window path) never flashes the overlay.
        root._awaitingWindow = true;
        root.launchStarted(app);
        snapshotClients.running = true;
        appRunner._appName = app.name || "";
        appRunner.command = ["hyprctl", "dispatch", "exec", app.exec || app.name];
        appRunner.running = true;
        detectNewWindow.restart();

        // Track launched app for resilient window matching
        let key = (app.wmClass || app.name || "").toLowerCase();
        if (key !== "") {
            let tracked = _launchedApps;
            tracked[key] = {
                app: app,
                misses: 0,
                windowClass: ""
            };
            _launchedApps = tracked;
        }

        appLaunched();
    }

    function checkAndLaunchApp(app) {
        _pendingApp = app;
        windowQuery.running = true;
    }

    function closeApp() {
        if (runningAppClass !== "") {
            closeAppWindow.appClass = runningAppClass;
            closeAppWindow.running = true;
        }
    }

    function closeAppByClass(windowClass) {
        if (windowClass && windowClass !== "") {
            closeAppWindow.appClass = windowClass;
            closeAppWindow.running = true;
        }
    }

    function focusApp(windowClass) {
        runningAppClass = windowClass;
        focusWindow.windowClass = windowClass;
        focusWindow.running = true;
        appLaunched();
    }

    // Address-based focus/close for the per-window home cards. Each running
    // card carries its Hyprland window address, so we target that exact window
    // instead of the first one matching a class.
    function focusByAddress(address) {
        if (!address || address === "")
            return;
        // Track the focused window's class for appClosed detection.
        for (let i = 0; i < runningWindows.length; i++) {
            if (runningWindows[i].address === address) {
                runningAppClass = runningWindows[i].windowClass;
                break;
            }
        }
        focusWindowAddr.addr = address;
        focusWindowAddr.running = true;
        appLaunched();
    }

    function closeByAddress(address) {
        if (address && address !== "") {
            closeWindowAddr.addr = address;
            closeWindowAddr.running = true;
        }
    }

    onShellStateChanged: {
        if (shellState === "idle") {
            if (!windowPoller.running)
                windowPoller.running = true;
        }
    }

    Process {
        id: closeAppWindow
        property string appClass: ""
        command: ["hyprctl", "dispatch", "closewindow", "class:" + appClass]
    }

    Process {
        id: closeWindowAddr
        property string addr: ""
        command: ["hyprctl", "dispatch", "closewindow", "address:" + addr]
    }

    Process {
        id: focusWindowAddr
        property string addr: ""
        command: ["hyprctl", "dispatch", "focuswindow", "address:" + addr]
        onExited: exitCode => {
            if (exitCode !== 0 && root.shellState === "appRunning")
                root.appClosed();
            else
                ensureFullscreen.running = true;
        }
    }

    Process {
        id: appRunner
        property string _appName: ""
        command: ["echo"]
        onExited: (exitCode, exitStatus) => {
            if (exitCode !== 0) {
                let cmd = appRunner.command.join(" ");
                ErrorLog.log("app", "Failed to launch " + (_appName || "application"), "Command: " + cmd + "\nExit code: " + exitCode, _appName);
                root.appLaunchFailed();
            }
        }
    }

    HyprctlClients {
        id: snapshotClients
        onClientsReceived: clients => {
            root._prelaunchClasses = clients.map(c => c["class"]);
            // Also snapshot addresses so openwindow events can check novelty by
            // address rather than by class (#203).
            root._prelaunchAddresses = clients.map(c => c["address"] || "");
        }
        onErrorOccurred: {
            root._prelaunchClasses = [];
            root._prelaunchAddresses = [];
        }
    }

    HyprctlClients {
        id: detectClient
        onClientsReceived: clients => {
            for (let i = 0; i < clients.length; i++) {
                if (root._prelaunchClasses.indexOf(clients[i]["class"]) < 0 && clients[i]["class"] !== "") {
                    root.runningAppClass = clients[i]["class"];

                    // Store discovered window class in _launchedApps
                    let tracked = root._launchedApps;
                    for (let key in tracked) {
                        if (tracked[key].windowClass === "" && WindowMatcher.matchesApp(tracked[key].app, clients[i])) {
                            tracked[key].windowClass = clients[i]["class"];
                            break;
                        }
                    }
                    root._launchedApps = tracked;

                    // New window mapped — hide the launch overlay (#193).
                    root._confirmWindow();
                    break;
                }
            }
        }
    }

    Timer {
        id: detectNewWindow
        interval: 2000
        onTriggered: {
            detectClient.running = true;
        }
    }

    HyprctlClients {
        id: windowQuery
        onClientsReceived: clients => {
            root._handleWindowQueryResult(clients);
        }
        onErrorOccurred: {
            root._handleWindowQueryResult([]);
        }
    }

    Process {
        id: focusWindow
        property string windowClass: ""
        command: ["hyprctl", "dispatch", "focuswindow", "class:" + windowClass]
        onExited: exitCode => {
            if (exitCode !== 0 && root.shellState === "appRunning")
                root.appClosed();
            else
                ensureFullscreen.running = true;
        }
    }

    // Window rule only applies at creation — restore fullscreen on resume
    Process {
        id: ensureFullscreen
        command: ["bash", "-c", "FS=$(hyprctl activewindow -j | grep -o '\"fullscreen\": [0-9]*' | grep -o '[0-9]*'); [ \"$FS\" = \"0\" ] && hyprctl dispatch fullscreen 0; exit 0"]
    }

    function _handleWindowQueryResult(clients) {
        let app = _pendingApp;
        if (!app)
            return;
        _pendingApp = null;

        for (let i = 0; i < clients.length; i++) {
            if (WindowMatcher.matchesApp(app, clients[i])) {
                root.runningAppClass = clients[i]["class"];
                focusWindow.windowClass = clients[i]["class"];
                focusWindow.running = true;
                appLaunched();
                return;
            }
        }

        launchDesktopApp(app);
    }

    HyprctlClients {
        id: windowPoller
        onClientsReceived: clients => {
            let apps = (root.applications || []);
            let windows = [];
            // One entry PER WINDOW (no class dedup) so the home row can show a
            // card per running window and focus/close each one individually.
            for (let i = 0; i < clients.length; i++) {
                let c = clients[i];
                let cls = c["class"] || "";
                if (cls === "" || cls.indexOf("quickshell") >= 0)
                    continue;

                let iconName = (c["initialClass"] || cls).toLowerCase();
                let appIcon = iconName;
                let appName = c["title"] || cls;

                // Use WindowMatcher for icon/name resolution
                for (let j = 0; j < apps.length; j++) {
                    if (WindowMatcher.matchesApp(apps[j], c)) {
                        appIcon = apps[j].icon || iconName;
                        appName = apps[j].name || appName;
                        break;
                    }
                }

                windows.push({
                    windowClass: cls,
                    address: c["address"] || "",
                    title: c["title"] || cls,
                    name: appName,
                    icon: appIcon,
                    // Hyprland focus order (0 = most recently focused); used to
                    // sort the running cards most-recently-used first.
                    focusHistoryId: (c["focusHistoryId"] !== undefined) ? c["focusHistoryId"] : 9999,
                    exec: ""
                });
            }
            // Only publish when the window set actually changed (class/address/
            // name/icon/focus-order). The poll fires every few seconds; a blind
            // reassignment rebuilds the home row's delegates and can drop
            // controller focus to nothing (dead stick until the mouse re-anchors).
            let sig = windows.map(function (w) {
                return w.windowClass + "|" + w.address + "|" + w.name + "|" + w.icon + "|" + w.focusHistoryId;
            }).join(";");
            if (sig !== root._runningWindowsSig) {
                root._runningWindowsSig = sig;
                root.runningWindows = windows;
            }

            // Track miss counts in _launchedApps
            let tracked = root._launchedApps;
            let trackedChanged = false;
            for (let key in tracked) {
                let entry = tracked[key];
                let wc = entry.windowClass;
                let found = false;

                if (wc !== "" && seenClasses[wc]) {
                    found = true;
                } else {
                    // Try matching by app metadata
                    for (let i = 0; i < clients.length; i++) {
                        if (WindowMatcher.matchesApp(entry.app, clients[i])) {
                            found = true;
                            if (wc === "") {
                                entry.windowClass = clients[i]["class"];
                                trackedChanged = true;
                            }
                            break;
                        }
                    }
                }

                if (found) {
                    if (entry.misses > 0) {
                        entry.misses = 0;
                        trackedChanged = true;
                    }
                } else {
                    entry.misses++;
                    trackedChanged = true;
                    if (entry.misses >= root._maxMisses) {
                        delete tracked[key];
                    }
                }
            }
            if (trackedChanged)
                root._launchedApps = tracked;

            // #193: keep scanning for a freshly-launched window that hasn't mapped
            // yet. The one-shot detectNewWindow timer fires once at 2s, so an app
            // slower than that (a cold flatpak launch — Plex HTPC's first start is
            // ~10-15s — sets up the sandbox/runtime before drawing) is missed and
            // runningAppClass stays "", leaving the launch overlay to hide on the
            // fallback timeout before the app actually appears. The poller runs
            // every 2s while appRunning, so adopt the first new non-prelaunch
            // window here: set it as the foreground app and confirm the launch, so
            // the overlay stays up until the window is really on screen.
            if (root._awaitingWindow && root.runningAppClass === "" && root.shellState === "appRunning") {
                for (let i = 0; i < clients.length; i++) {
                    let cls = clients[i]["class"] || "";
                    if (cls === "" || cls.indexOf("quickshell") >= 0)
                        continue;
                    if (root._prelaunchClasses.indexOf(cls) < 0) {
                        root.runningAppClass = cls;
                        root._confirmWindow();
                        break;
                    }
                }
            }

            // Only fire appClosed when in appRunning state and foreground app is truly gone
            if (root.shellState === "appRunning" && root.runningAppClass !== "") {
                let found = false;
                for (let i = 0; i < windows.length; i++) {
                    if (windows[i].windowClass === root.runningAppClass) {
                        found = true;
                        break;
                    }
                }
                if (found) {
                    // Foreground window is present — confirm the launch (#193).
                    // This is the reliable path for a freshly-launched window
                    // that maps after the one-shot detect timer has fired.
                    root._confirmWindow();
                } else {
                    root._awaitingWindow = false;
                    root.appClosed();
                }
            }
        }
        onErrorOccurred: message => {
            console.warn("AppLifecycleManager: window poll error:", message);
        }
    }

    Timer {
        id: windowPollTimer
        interval: root.shellState === "appRunning" ? 2000 : 5000
        running: root.shellState === "idle" || root.shellState === "appRunning"
        repeat: true
        triggeredOnStart: true
        onTriggered: {
            if (!windowPoller.running)
                windowPoller.running = true;
        }
    }

    // Subscribe to the daemon's Hyprland window events (hypr:activewindow,
    // hypr:fullscreen — see docs/IPC_PROTOCOL.md) so window open/close/focus
    // changes are reflected immediately instead of waiting for the next poll
    // tick. The periodic windowPoller above remains the source of truth for the
    // runningWindows model and appClosed detection; these events just kick an
    // extra poll on transitions, so the public behavior is unchanged.
    SocketClient {
        id: hyprEventListener
        // Subscribe stream over a native Quickshell socket (SocketClient, #97);
        // filter to `hypr:` lines (the stream also carries high-frequency
        // buttons:/intent:* events we don't want here). Auto-reconnects on drop.
        subscribe: true
        onLineReceived: line => {
            if (line.indexOf("hypr:activewindow:") === 0) {
                root.activeWindowClass = line.substring("hypr:activewindow:".length);
                root._onHyprWindowEvent();
            } else if (line.indexOf("hypr:fullscreen:") === 0) {
                root.activeWindowFullscreen = line.substring("hypr:fullscreen:".length) === "1";
                root._onHyprWindowEvent();
            } else if (line.indexOf("hypr:openwindow:") === 0) {
                root._onHyprOpenWindow(line.substring("hypr:openwindow:".length));
            } else if (line.indexOf("hypr:closewindow:") === 0) {
                root._onHyprCloseWindow(line.substring("hypr:closewindow:".length));
            }
        }
    }

    function _onHyprWindowEvent() {
        // Kick an immediate poll on window transitions while the shell is the
        // active state owner; the poller itself guards against re-entry.
        if ((root.shellState === "idle" || root.shellState === "appRunning") && !windowPoller.running)
            windowPoller.running = true;
    }

    // Handle a hypr:openwindow event — deterministic ADDRESS-based launch
    // confirmation (#203). Keeps the existing poll/detectNewWindow as fallback.
    //
    // Scope note: full child-PID→window-PID correlation needs moving `exec` into
    // the daemon — out of scope for this PR. This gives deterministic
    // ADDRESS-based correlation for the common case (one in-flight launch) plus
    // keeps the poll fallback for the edge cases.
    //
    // _confirmWindow() is idempotent (no-ops once _awaitingWindow is false), so
    // an event-then-poll double fire is safe.
    function _onHyprOpenWindow(payload) {
        if (!root._awaitingWindow)
            return;
        try {
            var w = JSON.parse(payload);
            var addr = w.address || "";
            // Address-novelty check: only act if this address was not already
            // present in the pre-launch snapshot.
            if (addr === "" || root._prelaunchAddresses.indexOf(addr) >= 0)
                return;

            // Find which tracked app this window satisfies (by WindowMatcher).
            var tracked = root._launchedApps;
            var matched = false;
            for (var key in tracked) {
                if (WindowMatcher.matchesApp(tracked[key].app, w)) {
                    tracked[key].windowClass = w.class || "";
                    matched = true;
                    break;
                }
            }
            // Accept even if no tracked app matched — the window is genuinely
            // new and we were awaiting one.
            root._launchedApps = tracked;
            root.runningAppClass = w.class || root.runningAppClass;
            root._foregroundAddress = addr;
            root._confirmWindow();
        } catch (e) {
            // Malformed JSON payload — fall through to poll fallback.
        }
        // Kick an extra poll so the runningWindows model and appClosed detection
        // see the new window without waiting for the next timer tick.
        root._onHyprWindowEvent();
    }

    // Handle a hypr:closewindow event — immediate appClosed detection (#203).
    // The poll remains the source of truth for runningWindows; this just fires
    // appClosed earlier when the closed address is the tracked foreground app.
    function _onHyprCloseWindow(address) {
        // Clear a stale foreground address when any window closes so it can't
        // suppress future openwindow launches.
        if (root._foregroundAddress === address) {
            root._foregroundAddress = "";
            if (root.shellState === "appRunning" && root.runningAppClass !== "") {
                root._awaitingWindow = false;
                root.appClosed();
                return;
            }
        }
        // Still kick a poll so runningWindows and the appClosed path in the
        // poller remain consistent.
        root._onHyprWindowEvent();
    }

    Component.onCompleted: {
        hyprEventListener.start();
    }
}
