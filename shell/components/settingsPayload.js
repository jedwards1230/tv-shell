.pragma library

// Pure builder for the daemon `set-config` payload — the minimal object that
// carries ONLY the keys that just changed.
//
// WHY MINIMAL MATTERS: the daemon shallow-merges the payload into settings.json
// at the TOP-LEVEL key granularity (daemon/src/config.rs merge_config: each key
// in the body replaces that key wholesale; unsent keys are preserved). A former
// full-snapshot save serialized EVERY schema key from the in-memory store on any
// single change, so a save racing a concurrent external edit (SSH/Ansible) wrote
// the store's stale cache over every key. Sending just the changed key(s) means
// an unrelated key an external editor touched is never round-tripped.
//
// Granularity note: per-widget config lives under the single `widgets` key, so a
// change to one widget sends the whole `widgets` subtree ({widgets: <subtree>}).
// That is correct and required — the daemon can't merge below a top-level key, so
// the subtree must be sent whole; it still leaves every NON-widget key untouched.
//
// Kept as a pure .pragma library function (no Quickshell) so it unit-tests
// headless (tst_settingspayload.qml) with zero drift from production.

// Build the payload for `changedKeys`, reading current values from `values`
// (`values[key]`; a plain object in tests, the SettingsStore item in prod).
// Skips unknown keys and daemon-owned (noSave) keys — those must never be
// written back by the QML client. `schema` is SettingsStore._schema:
// [{ key, noSave? }, …].
function buildSavePayload(schema, changedKeys, values) {
    var writable = {};
    for (var i = 0; i < schema.length; i++) {
        if (!schema[i].noSave)
            writable[schema[i].key] = true;
    }
    var out = {};
    for (var j = 0; j < changedKeys.length; j++) {
        var k = changedKeys[j];
        if (writable[k] === true)
            out[k] = values[k];
    }
    return out;
}
