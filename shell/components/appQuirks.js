.pragma library

.import "prewarm.js" as Prewarm

// Per-app behaviour overrides ("quirks"), keyed by the SAME app identity the
// prewarm engine uses — prewarm.keyFor(): StartupWMClass when the desktop entry
// declares one, else the exec basename. There is exactly one app-identity concept
// in this shell; this module reuses it rather than inventing a second one.
//
// The first quirk is `quitCommand`. Most apps genuinely exit when their window is
// closed (`hyprctl dispatch closewindow`) — Plex does. Some apps treat a window
// close as "minimise to background" and keep running: Steam does (verified on the
// device: the window disappears, but the steam PID and its dozen steamwebhelper
// children survive), and Discord/Spotify behave the same way. For those, closing
// the window makes the drawer's "Quit App" action a lie, so they declare the
// command that actually exits them.
//
// Steam is therefore the FIRST TABLE ENTRY, not a special case in the close path.
// Adding another close-to-tray app is a one-line data change here.
//
// The table is deliberately an object-of-objects rather than a bare command map so
// future per-app quirks (launch flags, resume behaviour, shutdown grace) can be
// added as sibling keys without restructuring this module or its callers.
var APP_QUIRKS = {
    "steam": {
        quitCommand: ["steam", "-shutdown"]
    }
};

// The whole quirk record for an app, or null when the app has no overrides.
// `matcher` is the WindowMatcher singleton at runtime, a plain object in tests —
// same contract as prewarm.js / resumeModel.js, which is what keeps this module
// QML-free and headless-testable.
function quirksFor(app, matcher) {
    if (!app || !matcher)
        return null;
    var key = Prewarm.keyFor(app, matcher);
    if (!key)
        return null;
    return APP_QUIRKS[key] || null;
}

// The command array that actually quits `app`, or null meaning "no override —
// closing the window is a real quit for this app". Callers treat null as the
// signal to keep the existing window-close behaviour.
function quitCommandFor(app, matcher) {
    var q = quirksFor(app, matcher);
    return (q && q.quitCommand && q.quitCommand.length > 0) ? q.quitCommand : null;
}

// The close path is driven from a live window (a Hyprland address + its class),
// not from a desktop entry — the drawer's and HomeScreen's resume rows carry no
// app object. Resolve the window back to its discovered app with the SAME
// WindowMatcher the rest of the shell matches windows with, then look the quirk up
// by that app's identity. Returns null when the window maps to no known app or
// that app has no quit override.
//
// A loose class can match more than one entry (WindowMatcher's last resort is a
// substring test), so we keep scanning past a match that carries no quirk rather
// than letting the first loose hit mask a real one.
function quitCommandForWindow(windowClass, apps, matcher) {
    if (!windowClass || windowClass === "" || !apps || !matcher)
        return null;
    var client = {
        "class": windowClass,
        "initialClass": windowClass
    };
    for (var i = 0; i < apps.length; i++) {
        if (!matcher.matchesApp(apps[i], client))
            continue;
        var cmd = quitCommandFor(apps[i], matcher);
        if (cmd)
            return cmd;
    }
    return null;
}
