.pragma library

// Pure, dependency-free config migrator for the widget framework (#249 Phase 3).
// A `.pragma library` JS module: no QML imports, no singletons — every function
// is a deterministic transform on plain objects, so it is unit-testable headless
// (tst_widgetmigrate.qml) and shared by SettingsStore.
//
// It turns the legacy FLAT settings keys (widgetMoonlightEnabled, widgetSpotify*,
// widgetPlex*, widgetRecent*) into the namespaced subtree the framework owns:
//   widgets: { <id>: { enabled, order, size, prefs: {...} } }
// `manifests` is WidgetManifests.manifests (passed in so this file imports nothing).

function _deepCopy(o) {
    return JSON.parse(JSON.stringify(o));
}

// Build the fully-defaulted namespaced subtree from manifest defaults:
//   enabled → manifest.defaultEnabled (absent ⇒ true), order → manifest.defaultOrder,
//   size → the "size" config default, prefs → every non-size config key mapped to
//   its default.
function defaultSubtree(manifests) {
    var out = {};
    for (var i = 0; i < manifests.length; i++) {
        var m = manifests[i];
        var size = "medium";
        var prefs = {};
        var cfg = m.config || [];
        for (var j = 0; j < cfg.length; j++) {
            var c = cfg[j];
            if (c.key === "size")
                size = c.default;
            else
                prefs[c.key] = c.default;
        }
        out[m.id] = {
            // Default-enabled unless the manifest explicitly opts out
            // (defaultEnabled: false) — keeps every existing widget enabled.
            "enabled": m.defaultEnabled !== false,
            "order": m.defaultOrder,
            "size": size,
            "prefs": prefs
        };
    }
    return out;
}

// Return a complete per-widget object: take `w`'s values where present and
// type-correct, else fall back to `base` (a defaultSubtree entry). Never coerces.
function _fillWidget(base, w) {
    var out = {
        "enabled": base.enabled,
        "order": base.order,
        "size": base.size,
        "prefs": {}
    };
    for (var bk in base.prefs)
        out.prefs[bk] = base.prefs[bk];
    if (w && typeof w === "object") {
        if (typeof w.enabled === "boolean")
            out.enabled = w.enabled;
        if (typeof w.order === "number")
            out.order = w.order;
        if (typeof w.size === "string")
            out.size = w.size;
        if (w.prefs && typeof w.prefs === "object") {
            for (var pk in w.prefs)
                out.prefs[pk] = w.prefs[pk];
        }
    }
    return out;
}

// Overlay legacy flat keys from `obj` onto an already-defaulted subtree. The flat
// widgetSpotify* keys map to id "nowplaying"; a flat value is applied only when it
// is PRESENT and type-correct (a missing key keeps the manifest default).
function _overlayLegacy(widgets, obj) {
    if (!obj || typeof obj !== "object")
        return;

    function setEnabled(id, key) {
        if (widgets[id] && typeof obj[key] === "boolean")
            widgets[id].enabled = obj[key];
    }
    function setSize(id, key) {
        if (widgets[id] && typeof obj[key] === "string")
            widgets[id].size = obj[key];
    }
    function setPref(id, prefKey, key) {
        if (widgets[id] && typeof obj[key] === "boolean")
            widgets[id].prefs[prefKey] = obj[key];
    }

    setEnabled("moonlight", "widgetMoonlightEnabled");
    setSize("moonlight", "widgetMoonlightSize");

    setEnabled("nowplaying", "widgetSpotifyEnabled");
    setSize("nowplaying", "widgetSpotifySize");
    setPref("nowplaying", "hideFromRecent", "widgetSpotifyHideFromRecent");

    setEnabled("plex", "widgetPlexEnabled");
    setSize("plex", "widgetPlexSize");
    setPref("plex", "hideFromRecent", "widgetPlexHideFromRecent");

    setEnabled("recent", "widgetRecentEnabled");
    setSize("recent", "widgetRecentSize");
}

// === Immutable per-widget config mutators (#249 / #281) ===
// Each returns a NEW widgets object (deep-cloned), never mutating the input.
// SettingsStore.setWidget / setWidgetPref / setWidgetOrder delegate here and then
// REASSIGN `store.widgets` to the result. Reassignment is what fires the QML
// `widgetsChanged` signal, so every binding reading `SettingsStore.widget(id)`
// (the HomeScreen recent-suppression catalog, the WidgetRegistry enabled/size/
// order entries, the Widgets page rows) re-evaluates. An IN-PLACE mutation of the
// nested object would notify nothing and silently break that live update — keeping
// the mutation here, behind headless immutability tests (tst_widgetreact.qml),
// makes that regression impossible to land without a red test.
//
// `defaults` is `defaultSubtree(manifests)`; it materialises a fully-defaulted
// entry only when the id is entirely absent (matching the old setters' `if
// (!copy[id]) copy[id] = widget(id)` fallback — with no existing entry, widget(id)
// resolves to exactly the default subtree entry).
function _ensureWidget(widgets, id, defaults) {
    if (widgets[id])
        return;
    widgets[id] = (defaults && defaults[id]) ? _deepCopy(defaults[id]) : {
        "enabled": true,
        "order": 0,
        "size": "medium",
        "prefs": {}
    };
}

// Set a top-level per-widget key (enabled / order / size).
function setWidget(widgets, id, key, value, defaults) {
    var copy = _deepCopy(widgets || {});
    _ensureWidget(copy, id, defaults);
    copy[id][key] = value;
    return copy;
}

// Set a per-widget pref (under widgets.<id>.prefs).
function setPref(widgets, id, prefKey, value, defaults) {
    var copy = _deepCopy(widgets || {});
    _ensureWidget(copy, id, defaults);
    if (!copy[id].prefs || typeof copy[id].prefs !== "object")
        copy[id].prefs = {};
    copy[id].prefs[prefKey] = value;
    return copy;
}

// Reorder: widgets.<id>.order = position for each id in orderedIds.
function setOrder(widgets, orderedIds, defaults) {
    var copy = _deepCopy(widgets || {});
    for (var i = 0; i < orderedIds.length; i++) {
        _ensureWidget(copy, orderedIds[i], defaults);
        copy[orderedIds[i]].order = i;
    }
    return copy;
}

// migrate(settingsObj, manifests) → { widgets, changed }.
//   - If settingsObj.widgets is already a non-empty object: fill any missing
//     per-widget keys from defaults (so the subtree is always complete) and
//     return changed:false — existing values are preserved, never clobbered.
//   - Otherwise (absent): build from defaults, overlay the legacy flat keys, and
//     return changed:true so the caller persists the new subtree.
function migrate(settingsObj, manifests) {
    var defaults = defaultSubtree(manifests);
    var existing = (settingsObj && typeof settingsObj === "object") ? settingsObj.widgets : undefined;

    if (existing && typeof existing === "object" && !Array.isArray(existing) && Object.keys(existing).length > 0) {
        var merged = _deepCopy(existing);
        for (var i = 0; i < manifests.length; i++) {
            var id = manifests[i].id;
            merged[id] = _fillWidget(defaults[id], merged[id]);
        }
        return {
            "widgets": merged,
            "changed": false
        };
    }

    var widgets = _deepCopy(defaults);
    _overlayLegacy(widgets, settingsObj);
    return {
        "widgets": widgets,
        "changed": true
    };
}
