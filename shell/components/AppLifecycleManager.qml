import Quickshell.Io
import QtQuick
import "prewarm.js" as Prewarm

Item {
    id: root

    property string runningAppClass: ""
    property var runningWindows: []
    // Signature of the last published runningWindows; gate reassignment on it
    // so an unchanged poll doesn't rebuild the home row and drop controller focus.
    property string _runningWindowsSig: ""
    property var applications: []
    property string shellState: ""

    // Prewarm key list of apps to silently prewarm at login (#238), bound from
    // SettingsStore in shell.qml. An entry is normally a StartupWMClass; for
    // desktop entries that declare none (e.g. Steam) it is the exec basename —
    // see prewarm.js keyFor().
    property var prewarmApps: []
    // One-shot guard for the login prewarm pass — set once a poll with BOTH a
    // usable window snapshot and a usable process snapshot has decided every
    // candidate. A poll missing either snapshot decides nothing and leaves this
    // false so the next poll retries.
    property bool _prewarmDone: false
    // { issued: {key:true} } — owned here, computed by prewarm.js. `issued` is the
    // in-flight dedup for launches WE issue: a key is marked the instant a launch
    // is dispatched, so a window/process that takes seconds to appear can never be
    // double-launched.
    property var _prewarmState: ({
            issued: ({})
        })
    // Staggered launch queue: apps resolved by _runPrewarm, dequeued one at a
    // time by prewarmStagger to avoid a thundering herd of cold starts (#238).
    property var _prewarmQueue: []

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
    // wmClass of the app currently being launched, tracked while
    // `_awaitingWindow` so a live `hypr:activewindow` event can confirm the
    // launch the moment that class becomes active — the fallback for apps
    // whose window was already mapped before this launch (a single-instance
    // app re-invoked via deep-link never produces a "new" window, so the
    // window-appear detectors below can't confirm it; an activewindow event
    // naming the same class can).
    property string _pendingLaunchClass: ""

    // One-shot guard for the STARTUP idle-adoption pass in windowPoller (below).
    // A quickshell restart mid-session boots the state machine `idle` even when
    // an app is already live and focused — the daemon's presenter keeps
    // emulating gamepad input over it, so the controller is locked inside an app
    // the shell doesn't know it's hosting. The shell's escape contract only
    // works in `appRunning` (see shell.qml: overlayOverApp / onIntentHomeTap /
    // resetToHome all gate on it), so the shell MUST adopt that foreground app.
    // This flag makes the poller do that boot-under-app adoption exactly ONCE,
    // on the first idle poll — never as a steady-state re-adopt: a deliberate
    // return-to-shell (returnToShell) leaves the app running in the background
    // AND still the compositor's active window, so a repeating poll adopt would
    // bounce the user straight back into the app they just escaped. Ongoing
    // out-of-band focus adoption is instead event-driven (the hypr:activewindow
    // handler), which a deliberate escape never re-triggers.
    property bool _startupAdoptionDone: false

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
            root._pendingLaunchClass = "";
            root.windowConfirmed();
        }
    }

    // Shell-side ADOPTION of a focused external app (no daemon protocol change).
    //
    // The full escape contract in shell.qml is gated on `state === "appRunning"`:
    // Meta HOLD -> intent:home-hold -> resetToHome -> returnToShell (grab +
    // focusHome), and Meta TAP over an app -> the controllable overlay drawer via
    // onIntentHomeTap -> overlayOverApp -> setOverlayFocus(true). BOTH are no-ops
    // while the shell is `idle`. But the daemon's follow-focus can put a focused
    // external app (e.g. Plex, class `tv.plex.Plex`) under the input presenter —
    // emulating keyboard/mouse from the gamepad — for an app the shell never
    // launched: (a) an app focused out-of-band of the shell's own launch flow, or
    // (b) a quickshell restart mid-session under an already-running app. In both
    // the shell stays `idle`, so NEITHER tap nor hold escapes and the user is
    // locked inside the app with no controller path back.
    //
    // Adopting closes that gap WITHOUT any daemon/protocol change: set the SAME
    // `runningAppClass` the launch flow sets and fire the SAME `appLaunched()`
    // signal (shell.qml onAppLaunched: state = "appRunning"). Once appRunning +
    // runningAppClass is set, the existing escape + overlay-drawer contract just
    // works, and the existing teardown path (the poller's appClosed check below,
    // which keys off runningAppClass, plus the `_maxMisses` disappearance
    // handling) returns the shell to idle when the adopted window goes away — an
    // adopted app is indistinguishable from a launched one to those paths.
    //
    // Guardrails: only adopt while `idle` (idempotent no-op otherwise, so an
    // event-then-poll double fire can't churn), and skip the shell's own surfaces
    // (empty / "quickshell") and prelaunch/transient classes — the SAME filters
    // the launch-detect scans use, not a reinvented set.
    function _maybeAdoptIdleApp(cls) {
        if (root.shellState !== "idle")
            return;
        if (!cls || cls === "" || cls.indexOf("quickshell") >= 0)
            return;
        if (root._prelaunchClasses.indexOf(cls) >= 0)
            return;
        root.runningAppClass = cls;
        // Drive the exact launch-flow signal path so state -> "appRunning".
        appLaunched();
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
        root._pendingLaunchClass = (app.wmClass || "").toLowerCase();
        root.launchStarted(app);
        snapshotClients.running = true;
        appRunner._appName = app.name || "";
        // Launch-time atomic placement: the `[fullscreen]` exec-rule prefix makes
        // Hyprland map the app's first window fullscreen from the start, before
        // any event round-trips — nothing to correct post-hoc. This is the
        // primary kiosk fullscreen guarantee for a fresh launch; the static
        // `windowrule = fullscreen` + the daemon's openwindow backstop remain as
        // defense-in-depth. Exec-rule syntax verified against Hyprland
        // src/config/supplementary/executor/Executor.cpp (`args[0] == '['`).
        appRunner.command = ["hyprctl", "dispatch", "exec", "[fullscreen] " + (app.exec || app.name)];
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

        // A foreground launch satisfies any prewarm entry for the same app —
        // record it so the prewarm pass can't launch a second copy in the gap
        // before this one's process and window actually appear.
        root._markPrewarmIssued(app);

        appLaunched();
    }

    // Mark `app`'s prewarm key as already-launched for this session, so no
    // prewarm pass can dispatch a duplicate while its window is still mapping.
    function _markPrewarmIssued(app) {
        let key = Prewarm.keyFor(app, WindowMatcher);
        if (key === "")
            return;
        let st = root._prewarmState;
        st.issued[key] = true;
        root._prewarmState = st;
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
        // Resuming an existing window (not a fresh launch): clear in-flight launch
        // tracking so a delayed closewindow for a PRIOR launch's address can't
        // fire appClosed() on this app (#203). No address is known here, so the
        // poll fallback handles this window's eventual close.
        root._foregroundAddress = "";
        root._awaitingWindow = false;
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
        // Track the focused window's class for appClosed detection. Only proceed
        // if the address is a currently-known window — otherwise we'd track a
        // foreground address with no matching runningAppClass, and a later
        // closewindow for it would fire appClosed() on the wrong app (#203).
        var found = false;
        for (let i = 0; i < runningWindows.length; i++) {
            if (runningWindows[i].address === address) {
                runningAppClass = runningWindows[i].windowClass;
                found = true;
                break;
            }
        }
        if (!found)
            return;
        // This window is now the foreground app: track ITS address so the fast
        // closewindow path targets it, and a stale prior-launch address can't
        // fire appClosed() here. Not awaiting a new window on a resume (#203).
        root._foregroundAddress = address;
        root._awaitingWindow = false;
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

    // Resume an app that's ALREADY running at a known address while ALSO
    // re-delivering its launch command — for single-instance apps (e.g. Steam)
    // where invoking the app again is how a deep-link (steam://) navigates the
    // running instance rather than spawning a new window. Mirrors
    // focusByAddress (the "recent apps" Focus action) but additionally fires
    // the exec first, so a deep-link to an already-running instance both
    // navigates AND raises the window in one call — no waiting on a new-window
    // event that a single-instance app will never produce.
    function redeliverAndFocus(app, address) {
        if (app && app.exec) {
            redeliverProcess.command = ["hyprctl", "dispatch", "exec", app.exec];
            redeliverProcess.running = true;
        }
        focusByAddress(address);
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

    // Fire-and-forget exec redelivery for redeliverAndFocus() above — a plain
    // one-shot dispatch, no exit-code handling needed (the focusByAddress call
    // that follows it owns the actual focus/appRunning transition).
    Process {
        id: redeliverProcess
    }

    // Fire-and-forget background prewarm launcher (#238). Uses the `[silent]`
    // exec-rule prefix so Hyprland opens the window WITHOUT focusing it — the app
    // starts in the background, never entering the foreground launch state machine
    // (no launchStarted/overlay/appLaunched/recents). A non-zero exit is logged but
    // does NOT fire the failure haptic (prewarm is silent by construction).
    Process {
        id: prewarmRunner
        property string _appName: ""
        command: ["echo"]
        onExited: exitCode => {
            if (exitCode !== 0)
                ErrorLog.log("app", "Failed to prewarm " + (_appName || "application"), "Command: " + prewarmRunner.command.join(" ") + "\nExit code: " + exitCode, _appName);
        }
    }

    // Process-table snapshot for prewarm dedup (#238 follow-up). One cheap call;
    // `-eo comm=` lists every process's NAME ONLY, with no header and, crucially,
    // no arguments — matching over a full cmdline for "steam" would also hit
    // steamwebhelper / srt-logger / pv-adverb / steam-runtime-launcher-service
    // and silently suppress a legitimate prewarm forever. prewarm.js compares
    // these names EXACTLY (and against the 15-char kernel truncation).
    Process {
        id: prewarmProcScan
        // The window snapshot this scan is being paired with, held across the
        // async gap so both halves describe the same moment.
        property var clients: []
        command: ["ps", "-eo", "comm="]
        stdout: SplitParser {
            property var collected: []
            onRead: line => {
                let name = line.trim();
                if (name !== "")
                    collected.push(name);
            }
        }
        onExited: exitCode => {
            let names = prewarmProcScan.stdout.collected;
            prewarmProcScan.stdout.collected = [];
            let clients = prewarmProcScan.clients;
            prewarmProcScan.clients = [];
            // A failed `ps` yields no usable list — hand null through so the
            // decision is skipped entirely rather than made on bad data.
            root._evaluatePrewarm(clients, exitCode === 0 ? names : null);
        }
    }

    // Launch `app` SILENTLY in the background (#238). The `[silent]` exec-rule
    // prefix is the proven production-hack incantation: it opens the window
    // UNFOCUSED so it never steals focus or enters the foreground path. This is a
    // PURE background exec — it deliberately does NOT emit launchStarted/appLaunched,
    // set _awaitingWindow/_pendingLaunchClass/runningAppClass, snapshot clients, or
    // touch _launchedApps.
    function prewarmApp(app) {
        if (!app)
            return;
        // Belt-and-braces: evaluate() already marked this key issued before it
        // reached the queue, but prewarmApp is public, so re-mark here too.
        root._markPrewarmIssued(app);
        prewarmRunner._appName = app.name || "";
        prewarmRunner.command = ["hyprctl", "dispatch", "exec", "[silent] " + (app.exec || app.name)];
        prewarmRunner.running = true;
    }

    // Login prewarm trigger (#238), driven from the first idle poll AFTER the
    // startup-adoption pass (see the windowPoller wiring for the ordering
    // rationale). Mapped windows alone are NOT enough to dedup against: an app
    // launched out-of-band moments earlier has no window for 10-15s (a Plex HTPC
    // cold start), which is how prewarm used to launch a second copy. So this
    // takes a process-table snapshot to pair with the window list, and defers the
    // decision to _evaluatePrewarm below. `clients` MUST be a real array — a poll
    // error gives us no window list, so that poll decides nothing and we retry.
    function _runPrewarm(clients) {
        if (root._prewarmDone)
            return;
        let apps = root.applications || [];
        let list = root.prewarmApps || [];
        if (apps.length === 0 || list.length === 0)
            return;
        if (!Array.isArray(clients))
            return;
        // A scan from the previous poll is still in flight — let it finish rather
        // than restarting the Process and losing its half-collected output.
        if (prewarmProcScan.running)
            return;
        prewarmProcScan.clients = clients;
        prewarmProcScan.running = true;
    }

    // Second half of the prewarm trigger, invoked once the process scan returns.
    // The decision logic itself lives in the pure, headless-tested prewarm.js.
    // `procNames` is null when the scan failed — evaluate() then decides nothing,
    // because a missing process list is NOT evidence that nothing is running and
    // acting on it is exactly how a double launch happens.
    function _evaluatePrewarm(clients, procNames) {
        if (root._prewarmDone)
            return;
        let res = Prewarm.evaluate(root.prewarmApps || [], root.applications || [], clients, procNames, root._prewarmState, WindowMatcher);
        if (!res.decided)
            return;                       // bad snapshot — retry on the next poll
        root._prewarmState = res.state;
        if (res.launch.length > 0) {
            root._prewarmQueue = (root._prewarmQueue || []).concat(res.launch);
            prewarmStagger.start();
        }
        // Window + process dedup is a COMPLETE answer for every candidate — there
        // is nothing left to settle or re-check — so the pass is over.
        root._prewarmDone = true;
    }

    // Dequeues one prewarm app every 600ms (triggeredOnStart → the first fires
    // immediately), stopping itself when the queue drains. The stagger avoids
    // launching every prewarm app at once (a thundering herd of flatpak cold
    // starts); it is NOT a blocking sleep (#238).
    Timer {
        id: prewarmStagger
        interval: 600
        repeat: true
        triggeredOnStart: true
        onTriggered: {
            // If the prior silent launch hasn't exited yet (plausible at login
            // under contention), skip this tick WITHOUT shifting the queue —
            // setting prewarmRunner.running = true on an already-true property is a
            // no-op that would silently drop this entry's launch. Retry next tick.
            if (prewarmRunner.running)
                return;
            let q = root._prewarmQueue || [];
            if (q.length === 0) {
                prewarmStagger.stop();
                return;
            }
            let app = q.shift();
            root._prewarmQueue = q;
            root.prewarmApp(app);
        }
    }

    Process {
        id: focusWindowAddr
        property string addr: ""
        command: ["hyprctl", "dispatch", "focuswindow", "address:" + addr]
        // No client-side fullscreen re-assertion here (kiosk phase 2, see
        // docs/KIOSK_WINDOW_MODEL.md): Hyprland's on_focus_under_fullscreen
        // already swaps fullscreen to this window atomically, and the daemon's
        // idempotent `fullscreen 0 set` enforcement is the sole writer. A
        // client-side toggle raced both and could un-fullscreen the window
        // (the split-view bug when a backgrounded app was tiled alongside it).
        onExited: exitCode => {
            // A failed focuswindow (the target vanished mid-resume) means the app
            // is gone. Fullscreen is deliberately NOT re-asserted here: the
            // compositor swaps fullscreen to the newly-focused window itself via
            // misc:on_focus_under_fullscreen=1, and the daemon is the single
            // idempotent backstop. The old `fullscreen 0` toggle here raced them
            // and could flip a resumed (already-fullscreen) window back OUT of
            // fullscreen — which, with a second app backgrounded, is exactly what
            // produced the two-app split view (docs/KIOSK_WINDOW_MODEL.md).
            if (exitCode !== 0 && root.shellState === "appRunning")
                root.appClosed();
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
        // No client-side fullscreen re-assertion here (kiosk phase 2, see
        // docs/KIOSK_WINDOW_MODEL.md): Hyprland's on_focus_under_fullscreen
        // already swaps fullscreen to this window atomically, and the daemon's
        // idempotent `fullscreen 0 set` enforcement is the sole writer. A
        // client-side read-then-toggle raced that writer and could un-fullscreen
        // the window instead of confirming it.
        onExited: exitCode => {
            // See focusWindowAddr above: no QML fullscreen re-assertion on resume.
            // on_focus_under_fullscreen=1 swaps fullscreen to the focused window
            // and the daemon is the single idempotent backstop; a toggle here
            // raced them into the split-view bug (docs/KIOSK_WINDOW_MODEL.md).
            if (exitCode !== 0 && root.shellState === "appRunning")
                root.appClosed();
        }
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
            // Set of window classes currently present — used by the
            // launched-app fast-path below to detect a still-running tracked app
            // without re-running the full WindowMatcher scan.
            let seenClasses = {};
            // One entry PER WINDOW (no class dedup) so the home row can show a
            // card per running window and focus/close each one individually.
            for (let i = 0; i < clients.length; i++) {
                let c = clients[i];
                let cls = c["class"] || "";
                if (cls === "" || cls.indexOf("quickshell") >= 0)
                    continue;

                seenClasses[cls] = true;

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

            // One-shot STARTUP idle-adoption (escape contract): the event-driven
            // activewindow adoption above only fires on a focus CHANGE, which a
            // quickshell restart mid-session under an already-running app never
            // produces (the app was focused before the shell even started, so no
            // event arrives). Catch that boot-under-app case from the FIRST idle
            // poll instead — the client list is the source of truth here even
            // with no activewindow event yet. Adopt the current foreground window
            // (lowest Hyprland focusHistoryId == most-recently-focused). `windows`
            // is already stripped of empty/"quickshell" classes, and
            // _maybeAdoptIdleApp re-applies every filter. Guarded one-shot by
            // _startupAdoptionDone so this cannot become a steady-state re-adopt:
            // a deliberate return-to-shell leaves the app backgrounded but still
            // the compositor's active window, and a repeating poll adopt would
            // bounce the user right back into it (see _startupAdoptionDone).
            if (!root._startupAdoptionDone && root.shellState === "idle") {
                root._startupAdoptionDone = true;
                let fgClass = "";
                let fgHist = 1000000;
                for (let i = 0; i < windows.length; i++) {
                    if (windows[i].focusHistoryId < fgHist) {
                        fgHist = windows[i].focusHistoryId;
                        fgClass = windows[i].windowClass;
                    }
                }
                if (fgClass !== "")
                    root._maybeAdoptIdleApp(fgClass);
            }

            // Login prewarm (#238) — fire strictly AFTER the one-shot startup
            // adoption above. RATIONALE: on the first idle poll, _startupAdoptionDone
            // runs with NO prewarmed windows present (we haven't launched yet) → it
            // adopts nothing → sets the one-shot done. Only THEN does prewarm launch.
            // A later poll that sees a prewarmed (unfocused) window can't re-trigger
            // adoption (it's one-shot), so a silently-prewarmed background app is
            // never mis-adopted into appRunning. The poll succeeding IS the readiness
            // signal (Hyprland answering + app list loaded) — replacing the deploy
            // hack's crude fixed `sleep 10` — and hands _runPrewarm the live `clients`
            // list, which it pairs with a process-table scan before deciding. Note
            // _runPrewarm is ASYNC (it awaits that scan); the adoption above is
            // synchronous and already finished, so the ordering still holds.
            if (root.shellState === "idle" && !root._prewarmDone && root.applications.length > 0 && root.prewarmApps.length > 0)
                root._runPrewarm(clients);

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
                // Backstop for a launch waiting on a window that will NEVER be
                // "new" — e.g. a single-instance app (Steam) already running
                // before this launch, re-invoked via deep-link. The window-
                // appear detectors (detectClient/windowPoller's prelaunch-
                // novelty check) can't see it since its class predates the
                // launch; the class becoming ACTIVE is just as valid a
                // confirmation.
                if (root._awaitingWindow && root._pendingLaunchClass !== "" && root.activeWindowClass.toLowerCase() === root._pendingLaunchClass) {
                    root.runningAppClass = root.activeWindowClass;
                    root._confirmWindow();
                }
                // Idle-adoption (escape contract): if the shell believes it is
                // idle but a non-shell window just BECAME the compositor's active
                // window, the daemon's follow-focus is (or is about to be)
                // emulating gamepad input over an app the shell never launched.
                // Adopt it into appRunning so the existing escape works (see
                // _maybeAdoptIdleApp). This is the ongoing, event-driven path for
                // out-of-band focus; it is loop-safe because a deliberate return-
                // to-shell does NOT emit a fresh activewindow event naming the app
                // class, so escaping can never re-trigger adoption here.
                root._maybeAdoptIdleApp(root.activeWindowClass);
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
                if (tracked[key] && tracked[key].app && WindowMatcher.matchesApp(tracked[key].app, w)) {
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
            // Malformed JSON payload — log and fall through to the poll fallback.
            console.warn("AppLifecycleManager: malformed hypr:openwindow payload:", e);
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
