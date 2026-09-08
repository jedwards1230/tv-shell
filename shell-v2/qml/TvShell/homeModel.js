// The home screen, as a pure function of (catalog, core snapshot).
//
// This module is the whole of "the core owns state, the shell renders it". The
// shell holds no idea of what is running: it asks the core for one `screen-state`
// snapshot and this function turns that snapshot plus the catalog into rails.
// There is no cache to go stale, no second opinion to disagree with the core,
// and nothing here asks the compositor anything.
//
// THE RULES
//
//   H1  An app the core lists in `focusable_apps` is running. That list is the
//       only input to "is it running" — not a launch we remember making, not a
//       pid we kept. If the core does not list it, it is not running, however
//       recently we launched it.
//   H2  A running app with no catalog row still appears, titled "App <id>". The
//       core is the authority on what exists; hiding a running app because the
//       shell's cosmetic file has not heard of it would make an app unreachable
//       from the shell that started it.
//   H3  The shell's own app id NEVER appears in any rail. The shell is always
//       focusable — it is on screen — so without this the home screen offers you
//       a card that switches to the home screen.
//   H4  The Apps rail is the whole catalog in catalog order, running or not. A
//       home screen whose tiles move when something starts is a home screen you
//       cannot use from muscle memory.
//   H5  The action is decided here and nowhere else: `show` for a running app,
//       `launch` otherwise. A caller sends `entry.action + " " + entry.appId`
//       and makes no decision of its own.
//   H6  A missing snapshot (core down, first frame, a reply that did not parse)
//       is not an error state: Continue is empty, Apps is unaffected, and the
//       screen is usable. The shell renders without the core; it just knows
//       less.
//   H7  An empty rail is omitted — no header over nothing — but `rails` is
//       always an array.
//   H8  When every rail is empty the model says `empty: true`. The screen
//       renders one focusable empty-state cell in that case, which is what makes
//       "focus is unplaceable on home" unreachable: there is always at least one
//       cell. This rule is the model half of the stranding guarantee that
//       focusGraph.js R4 is the router half of.
//
// tests/qml/tst_homemodel.qml asserts each of these.
.pragma library

var CONTINUE = "continue";
var APPS = "apps";

// Pull the running app ids out of a `screen-state` reply.
//
// Defensive about shape rather than trusting it: this crosses a process
// boundary, and a core that changed its JSON should degrade to "nothing is
// running" rather than throwing inside a binding and blanking the screen.
function runningIds(snapshot) {
    if (!snapshot || typeof snapshot !== "object")
        return [];
    var apps = snapshot.focusable_apps;
    if (!Array.isArray(apps))
        return [];
    var out = [];
    for (var i = 0; i < apps.length; ++i) {
        var id = apps[i];
        if (typeof id === "number" && isFinite(id))
            out.push(id);
    }
    return out;
}

// The app id currently on screen, or null. Read from the snapshot's `on_screen`
// object — the core's own resolved answer — and NEVER from
// `focused_app_atom_diagnostic`, which the core's docs mark as diagnostic
// because it reads empty under an input-focus overlay. Reading the diagnostic
// atom would make the header say "nothing is running" every time the drawer is
// open.
function onScreenId(snapshot) {
    if (!snapshot || typeof snapshot !== "object")
        return null;
    var on = snapshot.on_screen;
    if (!on || typeof on !== "object")
        return null;
    return (typeof on.app_id === "number" && isFinite(on.app_id)) ? on.app_id : null;
}

function _entry(appId, catalogById, running) {
    var meta = catalogById[String(appId)];
    return {
        "appId": appId,
        "title": meta ? meta.title : ("App " + appId), // H2
        "subtitle": meta ? meta.subtitle : "",
        "accent": meta ? meta.accent : "",
        "running": running,
        "action": running ? "show" : "launch" // H5
    };
}

// Build the home model.
//
//   entries    catalog entries, from catalog.parse().entries
//   snapshot   a parsed `screen-state` reply, or null (H6)
//   shellAppId the id the shell tagged its own base window with (H3)
//
// Returns { rails: [ { id, title, entries: [...] } ], empty: bool }.
function build(entries, snapshot, shellAppId) {
    var catalog = Array.isArray(entries) ? entries : [];
    var byId = {};
    for (var i = 0; i < catalog.length; ++i)
        byId[String(catalog[i].appId)] = catalog[i];

    var running = runningIds(snapshot);
    var runningSet = {};
    for (var j = 0; j < running.length; ++j)
        runningSet[String(running[j])] = true;

    var cont = [];
    for (var k = 0; k < running.length; ++k) {
        if (running[k] === shellAppId)
            continue; // H3
        cont.push(_entry(running[k], byId, true));
    }

    var apps = [];
    for (var m = 0; m < catalog.length; ++m) {
        var id = catalog[m].appId;
        if (id === shellAppId)
            continue; // H3 — in the catalog too, not only in the core's list
        apps.push(_entry(id, byId, runningSet[String(id)] === true)); // H4
    }

    var rails = [];
    if (cont.length > 0)
        rails.push({ "id": CONTINUE, "title": "Continue", "entries": cont }); // H7
    if (apps.length > 0)
        rails.push({ "id": APPS, "title": "Apps", "entries": apps });

    return { "rails": rails, "empty": rails.length === 0 }; // H8
}

// The command line for activating an entry. One place, so no caller ever builds
// a core command by string-concatenating a verb it chose itself (H5).
function commandFor(entry) {
    if (!entry || typeof entry.action !== "string" || typeof entry.appId !== "number")
        return "";
    return entry.action + " " + entry.appId;
}
