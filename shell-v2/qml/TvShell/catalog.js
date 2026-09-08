// The shell's app catalog, as a pure function of the file's text.
//
// The catalog is display metadata ONLY — what an app is called and how it looks.
// Launch mechanics (argv, env, cgroup scope) belong to the core's `[[app]]`
// table and are never duplicated here; the shell launches by id and the core
// resolves the rest. See src/shellconfig.h for why the shell needs its own file
// at all, and what would remove the need.
//
// File shape:
//
//   { "apps": [ { "id": 9003, "title": "Moonlight", "subtitle": "Stream" } ] }
//
// THE RULES
//
//   C1  parse() never throws and never returns null. Malformed JSON yields an
//       empty catalog and a problem, because a syntax error in a config file
//       must not be the difference between a shell and a black screen.
//   C2  One bad row does not discard the good ones. A row is dropped, and
//       reported, only for the two things that make it unusable: no integer id,
//       or no non-empty title. Everything else has a default.
//   C3  A duplicate id keeps the FIRST row and reports the collision. Last-wins
//       would make the visible catalog depend on file order in a way nobody
//       reading the file would predict.
//   C4  Absence is not an error. Empty text — no file, or an empty one — is an
//       empty catalog with NO problems. A box that has not configured a catalog
//       has not made a mistake.
//   C5  Order is file order, preserved exactly. The home screen's layout is
//       therefore something the user edits, not something the shell sorts.
//
// tests/qml/tst_catalog.qml asserts each of these.
.pragma library

// An id must be a whole number that could be a gamescope app id: 32-bit
// unsigned. A float or a negative is a typo, not a rounding opportunity.
function _validId(v) {
    return typeof v === "number" && isFinite(v) && Math.floor(v) === v && v >= 0 && v <= 4294967295;
}

function _string(v, fallback) {
    return (typeof v === "string" && v.length > 0) ? v : fallback;
}

// Parse `text` into { entries: [...], problems: [string] }.
//
// An entry is { appId: int, title: string, subtitle: string, accent: string }.
// `accent` is "" when unset; the theme picks a default rather than this module
// inventing a colour, so the palette stays in one place.
function parse(text) {
    if (typeof text !== "string" || text.trim().length === 0)
        return { "entries": [], "problems": [] }; // C4

    var doc;
    try {
        doc = JSON.parse(text);
    } catch (e) {
        return { "entries": [], "problems": ["catalog is not valid JSON: " + e] }; // C1
    }
    // Array.isArray first: a bare array passes `typeof === "object"`, so
    // without this a top-level `[...]` would fall through to the `apps`
    // lookup, find nothing, and report a valid empty catalog (C4) for a file
    // that is plainly wrong.
    if (doc === null || typeof doc !== "object" || Array.isArray(doc))
        return { "entries": [], "problems": ["catalog is not a JSON object"] };

    var rows = doc.apps;
    if (rows === undefined)
        return { "entries": [], "problems": [] }; // an object with no apps key is an empty catalog
    if (!Array.isArray(rows))
        return { "entries": [], "problems": ["catalog 'apps' is not an array"] };

    var entries = [];
    var problems = [];
    var seen = {};
    for (var i = 0; i < rows.length; ++i) {
        var row = rows[i];
        if (row === null || typeof row !== "object") {
            problems.push("app " + i + " is not an object");
            continue;
        }
        if (!_validId(row.id)) {
            problems.push("app " + i + " has no usable integer id"); // C2
            continue;
        }
        var title = _string(row.title, "");
        if (title === "") {
            problems.push("app " + row.id + " has no title"); // C2
            continue;
        }
        var key = String(row.id);
        if (seen[key] === true) {
            problems.push("duplicate app id " + row.id + "; keeping the first"); // C3
            continue;
        }
        seen[key] = true;
        entries.push({
            "appId": row.id,
            "title": title,
            "subtitle": _string(row.subtitle, ""),
            "accent": _string(row.accent, "")
        });
    }
    return { "entries": entries, "problems": problems }; // C5: no sort anywhere
}
