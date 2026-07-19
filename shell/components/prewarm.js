.pragma library

// Pure prewarm decision engine (#238 follow-up). QML-free and headless-testable;
// `matcher` is the WindowMatcher singleton at runtime (matchesApp/execBasename),
// a plain object in tests — same contract as resumeModel.js.
//
// Two defects this fixes:
//  (a) The pass deduped against MAPPED WINDOWS only. A competing out-of-band
//      launch has no window for 10-15s (a Plex HTPC cold start), so the snapshot
//      was honestly empty and prewarm launched a SECOND copy. But that app's
//      PROCESS exists immediately — so we dedup against the process table too.
//      The union of the two signals means "already running or starting", which
//      closes the race with ZERO added delay: prewarm still fires on the first
//      successful idle poll.
//  (b) An app whose desktop entry has an empty StartupWMClass (Steam) could
//      never be named or resolved. Fixed by keyFor()'s exec-basename fallback.

// Linux truncates a process name (/proc/<pid>/comm, and therefore `ps -eo comm=`)
// to 15 characters — visible in real output as `QtWebEngineProc` /
// `steam-runtime-l`. A long-named app must still match, so we compare against the
// truncation too rather than letting it silently never match.
var COMM_MAX_LEN = 15;

// The stable prewarm key for an app: StartupWMClass when present, else the exec
// basename. Steam's .desktop carries no StartupWMClass, so it keys as "steam" —
// which is also what its window class and its process name report, so both dedup
// signals still work.
function keyFor(app, matcher) {
    if (!app)
        return "";
    if (app.wmClass && app.wmClass !== "")
        return app.wmClass;
    return matcher.execBasename(app.exec || "");
}

// Resolve a configured key to a discovered app. EXACT wmClass match wins across
// the whole list first, so existing configs behave byte-identically; only then
// do we fall back to exec-basename among entries with no wmClass.
function resolveApp(key, apps, matcher) {
    for (var i = 0; i < apps.length; i++) {
        if (apps[i].wmClass === key)
            return apps[i];
    }
    for (var j = 0; j < apps.length; j++) {
        var a = apps[j];
        if ((!a.wmClass || a.wmClass === "") && matcher.execBasename(a.exec || "") === key)
            return a;
    }
    return null;
}

// Is `app`'s process already alive? `procNames` is a list of process NAMES only
// (`ps -eo comm=`); this function owns the normalization, so callers hand over
// unprocessed lines and tests can't drift from production.
//
// The comparison is EXACT, never substring — that precision is the whole point.
// A substring match over a full cmdline for "steam" also hits `steamwebhelper`,
// `srt-logger`, `pv-adverb` and `steam-runtime-launcher-service --alongside-steam`,
// any of which would silently suppress a legitimate Steam prewarm forever.
// `steamwebhelper !== steam`, so exact comm matching is immune to all of them.
function matchesProcess(app, procNames, matcher) {
    var base = matcher.execBasename(app.exec || "");
    if (base === "")
        return false;
    var truncated = base.length > COMM_MAX_LEN ? base.substring(0, COMM_MAX_LEN) : base;
    for (var i = 0; i < procNames.length; i++) {
        var p = (procNames[i] || "").trim().toLowerCase();
        if (p === "")
            continue;
        if (p === base || p === truncated)
            return true;
    }
    return false;
}

function matchesWindow(app, clients, matcher) {
    for (var i = 0; i < clients.length; i++) {
        if (matcher.matchesApp(app, clients[i]))
            return true;
    }
    return false;
}

// Decide one poll.
//   state:  { issued: {key:true} }   (caller-owned, treated as immutable)
//   returns { launch: [app...], state: <next state>, decided: bool,
//             skipped: [{key, reason}...] }
//
// `skipped` is DIAGNOSTIC ONLY — it records why each configured key was not
// launched (issued / unknown-app / window-mapped / process-alive) so the pass's
// one decision is legible in the journal (see launchTrace.js). Nothing branches
// on it; adding a reason can never change what gets launched.
//
// `decided: false` means the snapshot was unusable and NOTHING was decided — the
// caller keeps the state it had and retries on the next poll. BOTH the window
// list and the process list are required: a missing process list is not evidence
// that nothing is running, and acting on it is exactly how a double launch
// happens.
//
// On a usable snapshot every candidate is resolved in this one pass — there is no
// settling and no deferral — so prewarm fires as early as the shell can safely
// know Hyprland is answering and the app list has loaded.
function evaluate(list, apps, clients, procNames, state, matcher) {
    var issued = {};
    var k;
    for (k in ((state && state.issued) || {}))
        issued[k] = true;

    if (!Array.isArray(clients) || !Array.isArray(procNames) || !apps || !list)
        return {
            launch: [],
            state: state,
            decided: false,
            skipped: []
        };

    var out = {
        launch: [],
        state: {
            issued: issued
        },
        decided: true,
        skipped: []
    };

    var seen = {};
    for (var li = 0; li < list.length; li++) {
        var key = list[li];
        if (!key || seen[key])
            continue;               // blanks + hand-edited dupes
        seen[key] = true;
        if (issued[key]) {
            out.skipped.push({
                key: key,
                reason: "issued"
            });
            continue;               // already launched this session
        }
        var app = resolveApp(key, apps, matcher);
        if (!app) {
            out.skipped.push({
                key: key,
                reason: "unknown-app"
            });
            continue;               // unknown app / typo — silently skip
        }
        if (matchesWindow(app, clients, matcher)) {
            out.skipped.push({
                key: key,
                reason: "window-mapped"
            });
            continue;               // window mapped: already running
        }
        if (matchesProcess(app, procNames, matcher)) {
            out.skipped.push({
                key: key,
                reason: "process-alive"
            });
            continue;               // process alive: running, or still starting
        }
        out.launch.push(app);
        out.state.issued[key] = true;
    }
    return out;
}
