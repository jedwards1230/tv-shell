.pragma library

// Merge running windows (AppLifecycleManager.runningWindows) + recent apps
// (RecentsTracker.recentApps) into a deduped, ordered "resume" list for the
// nav-drawer hero zone. Running-first, sorted by focusHistoryId ascending;
// unmatched recents appended.
//
// `matcher` is an object exposing execBasename(str) and normalize(str)
// (the WindowMatcher singleton at runtime; a plain stub in tests) — passing it
// in keeps this module QML-free and headless-testable.
//
// Ported VERBATIM from HomeScreen.qml's `_recentModel` merge/dedup
// (runningMatchesRecent + resolveRecentIcon + the running/recents assembly),
// MINUS the HomeScreen-specific widget-shadowing filter (root._widgets /
// hideFromRecent) — that suppresses apps an on-screen home widget already
// represents, and the drawer hosts no home widgets. HomeScreen could later adopt
// this module to DRY the duplicated merge.
//
// Entry shape (identical to HomeScreen's, consumed by AppCard):
//   { windowClass, address, name, icon, exec, comment, running, focusHistoryId }
function build(running, recents, allApps, matcher) {
    running = running || [];
    recents = recents || [];
    allApps = allApps || [];

    function runningMatchesRecent(win, recent) {
        let cls = (win.windowClass || "").toLowerCase();
        let execBase = matcher.execBasename(recent.exec || "");
        let appName = (recent.name || "").toLowerCase();
        let winName = (win.name || "").toLowerCase();
        if (winName !== "" && winName === appName)
            return true;
        if (execBase !== "") {
            if (cls === execBase || matcher.normalize(cls) === matcher.normalize(execBase))
                return true;
            if (cls !== "" && (execBase.indexOf(cls) >= 0 || cls.indexOf(execBase) >= 0))
                return true;
        }
        if (appName !== "" && (cls === appName || matcher.normalize(cls) === matcher.normalize(appName)))
            return true;
        return false;
    }
    function resolveRecentIcon(rec) {
        let rexec = matcher.execBasename(rec.exec || "");
        for (let i = 0; i < allApps.length; i++) {
            let a = allApps[i];
            if (a.name && rec.name && a.name === rec.name)
                return a.icon || "";
            if (rexec !== "" && matcher.execBasename(a.exec || "") === rexec)
                return a.icon || "";
        }
        return rec.icon || "";
    }

    let runningEntries = [];
    let matchedRecentIndices = new Set();
    for (let r = 0; r < running.length; r++) {
        let win = running[r];
        for (let j = 0; j < recents.length; j++) {
            if (runningMatchesRecent(win, recents[j]))
                matchedRecentIndices.add(j);
        }
        runningEntries.push({
            windowClass: win.windowClass,
            address: win.address || "",
            name: win.title || win.name || win.windowClass,
            icon: win.icon || "",
            exec: "",
            comment: "",
            running: true,
            focusHistoryId: (win.focusHistoryId !== undefined) ? win.focusHistoryId : 9999
        });
    }
    runningEntries.sort(function (a, b) {
        return a.focusHistoryId - b.focusHistoryId;
    });

    let result = runningEntries.slice();
    for (let k = 0; k < recents.length; k++) {
        if (matchedRecentIndices.has(k))
            continue;
        let rec = recents[k];
        result.push({
            windowClass: "",
            address: "",
            name: rec.name || "",
            icon: resolveRecentIcon(rec),
            exec: rec.exec || "",
            comment: rec.comment || "",
            running: false,
            focusHistoryId: 9999
        });
    }
    return result;
}
