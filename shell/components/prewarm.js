.pragma library

// Pure prewarm decision engine (#238 follow-up). QML-free and headless-testable;
// `matcher` is the WindowMatcher singleton at runtime (matchesApp/execBasename),
// a plain object in tests — same contract as resumeModel.js.
//
// Two defects this fixes:
//  (a) The one-shot pass deduped against MAPPED WINDOWS only, from a single
//      snapshot. An app launched out-of-band ~1s before the pass has no window
//      yet (a Plex HTPC cold start is 10-15s), so prewarm launched a SECOND copy.
//      Fixed by requiring a candidate to be absent for `settlePolls` CONSECUTIVE
//      polls before launching, plus an `issued` set so nothing launches twice.
//  (b) An app whose desktop entry has an empty StartupWMClass (Steam) could
//      never be named or resolved. Fixed by keyFor()'s exec-basename fallback.

// The stable prewarm key for an app: StartupWMClass when present, else the exec
// basename. Steam's .desktop carries no StartupWMClass, so it keys as "steam" —
// which is also what its window class reports, so WindowMatcher dedup still works.
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

// Evaluate one poll.
//   state: { absent: {key:int}, issued: {key:true} }   (caller-owned, immutable)
//   returns { launch: [app...], state: <next state>, pending: int }
// `pending === 0` means nothing is left to decide — the caller stops evaluating.
// `pending === -1` means the snapshot was unusable, so NOTHING was decided and
// the caller must keep the state it already had.
function evaluate(list, apps, clients, state, matcher, settlePolls) {
    var prevAbsent = (state && state.absent) || {};
    var issued = {};
    var k;
    for (k in ((state && state.issued) || {}))
        issued[k] = true;

    var out = {
        launch: [],
        state: {
            absent: {},
            issued: issued
        },
        pending: 0
    };
    if (!Array.isArray(clients) || !apps || !list)
        return {
            launch: [],
            state: state,
            pending: -1
        };

    var seen = {};
    for (var li = 0; li < list.length; li++) {
        var key = list[li];
        if (!key || seen[key])
            continue;               // blanks + hand-edited dupes
        seen[key] = true;
        if (issued[key])
            continue;               // already launched this session
        var app = resolveApp(key, apps, matcher);
        if (!app)
            continue;               // unknown app / typo — silently skip, forever

        var running = false;
        for (var ci = 0; ci < clients.length; ci++) {
            if (matcher.matchesApp(app, clients[ci])) {
                running = true;
                break;
            }
        }
        if (running) {
            out.state.absent[key] = 0;   // someone else has it: never launch
            continue;
        }
        var n = (prevAbsent[key] || 0) + 1;
        out.state.absent[key] = n;
        if (n >= settlePolls) {
            out.launch.push(app);
            out.state.issued[key] = true;
        } else {
            out.pending++;               // still settling
        }
    }
    return out;
}
