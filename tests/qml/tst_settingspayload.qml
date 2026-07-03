import QtQuick
import QtTest
import "../../shell/components/settingsPayload.js" as SettingsPayload

// Unit test for the minimal set-config payload builder (per-key settings diff).
//
// The bug this closes: SettingsStore.save() used to serialize EVERY schema key
// from the in-memory store on any single change, so a save racing a concurrent
// external edit to settings.json wrote the store's stale cache over unrelated
// keys. buildSavePayload emits ONLY the changed key(s) — the daemon shallow-merge
// then preserves everything the payload omits. These assertions lock:
//   1. minimality  — the payload contains exactly the changed key(s), nothing else
//   2. noSave drop  — daemon-owned keys (keyBindings) are never written back
//   3. unknown drop — a key not in the schema is skipped
//   4. widgets whole-subtree passthrough (the one-key granularity for per-widget)
//   5. values are read from the supplied store snapshot (not the changedKeys)
//
// The real settingsPayload.js is imported by source path (zero drift). The schema
// is a representative fixture mirroring SettingsStore._schema's shape
// ({key, noSave?}) — the builder is schema-agnostic, so a fixture suffices.
TestCase {
    id: tc
    name: "SettingsPayload"

    // Representative schema (shape-faithful subset of SettingsStore._schema).
    readonly property var schema: [
        {
            key: "themeMode"
        },
        {
            key: "controllerDebug"
        },
        {
            key: "widgets"
        },
        {
            key: "cecDeviceNames"
        },
        {
            key: "keyBindings",
            noSave: true
        }
    ]

    // A stand-in for the SettingsStore item — values read via values[key].
    readonly property var store: ({
            "themeMode": "dark",
            "controllerDebug": true,
            "widgets": {
                "plex": {
                    "enabled": false,
                    "order": 2
                }
            },
            "cecDeviceNames": {
                "0": "Living Room TV"
            },
            "keyBindings": {
                "select": "BTN_SOUTH"
            }
        })

    function _keys(o) {
        var out = [];
        for (var k in o)
            out.push(k);
        return out.sort();
    }

    // 1. A single changed key yields a single-key payload with the right value.
    function test_single_key_is_minimal() {
        var p = SettingsPayload.buildSavePayload(tc.schema, ["themeMode"], tc.store);
        compare(_keys(p), ["themeMode"], "only the changed key is present");
        compare(p.themeMode, "dark", "value read from the store snapshot");
    }

    // 2. Daemon-owned (noSave) keys are never written back, even if 'changed'.
    function test_noSave_key_is_dropped() {
        var p = SettingsPayload.buildSavePayload(tc.schema, ["keyBindings"], tc.store);
        compare(_keys(p), [], "keyBindings (noSave) produces an empty payload");
        // Mixed: a writable + a noSave key → only the writable survives.
        var p2 = SettingsPayload.buildSavePayload(tc.schema, ["themeMode", "keyBindings"], tc.store);
        compare(_keys(p2), ["themeMode"], "noSave stripped from a mixed change set");
    }

    // 3. A key absent from the schema is skipped (never invents a write).
    function test_unknown_key_is_dropped() {
        var p = SettingsPayload.buildSavePayload(tc.schema, ["bogusKey"], tc.store);
        compare(_keys(p), [], "unknown key produces an empty payload");
    }

    // 4. The whole widgets subtree passes through as one key (per-widget writes).
    function test_widgets_subtree_passthrough() {
        var p = SettingsPayload.buildSavePayload(tc.schema, ["widgets"], tc.store);
        compare(_keys(p), ["widgets"], "only the widgets key is sent");
        compare(p.widgets.plex.enabled, false, "the full subtree is carried verbatim");
        compare(p.widgets.plex.order, 2);
    }

    // 5. Multiple writable keys all appear; nothing else leaks in.
    function test_multiple_writable_keys() {
        var p = SettingsPayload.buildSavePayload(tc.schema, ["themeMode", "controllerDebug", "cecDeviceNames"], tc.store);
        compare(_keys(p), ["cecDeviceNames", "controllerDebug", "themeMode"]);
        compare(p.controllerDebug, true);
        // The store carries other keys (widgets, keyBindings) that must NOT leak
        // into a payload that didn't name them — proves it's a diff, not a snapshot.
        verify(!("widgets" in p), "unchanged widgets not round-tripped");
        verify(!("keyBindings" in p), "unchanged keyBindings not round-tripped");
    }

    // 6. An empty change set is a no-op payload (caller then skips the socket).
    function test_empty_change_set() {
        compare(_keys(SettingsPayload.buildSavePayload(tc.schema, [], tc.store)), []);
    }
}
