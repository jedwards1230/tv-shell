pragma Singleton
import QtQuick

// Per-widget manifest metadata — the pure-data SSOT for the widget framework
// (#249 Phase 3). Keyed by id, in framework order (moonlight → nowplaying →
// plex → recent → steam). This singleton intentionally has NO Quickshell imports and NO
// widget imports: it is plain data so it can be read from another `components`
// singleton (SettingsStore) and from the pure-JS migrator (widgetConfig.js)
// without a dependency cycle.
//
// Each manifest entry:
//   id          — stable kebab id (config namespace + registry key)
//   name        — human label for the Widgets page
//   version     — semver (drives future widget-<id>-v* releases)
//   requires    — capability strings the host gates against (Phase 5); a missing
//                 capability greys the widget + explains why, never crashes
//   defaultOrder— seeds widgets.<id>.order on first run only; the reorder UI then
//                 owns it
//   defaultEnabled — OPTIONAL. Seeds widgets.<id>.enabled on first run only.
//                 Absent ⇒ true (every existing widget ships enabled). Set false
//                 to ship a widget disabled-by-default (the user opts in from the
//                 Widgets app); once toggled, the persisted value owns it.
//   config      — ORDERED typed schema the Widgets page renders against. Entries
//                 are {key, type, default, label, values?} with
//                 type ∈ "enum" | "bool" | "int" | "string". The key "size" is the
//                 framework-owned size enum; every other key is a widget pref
//                 (stored under widgets.<id>.prefs). `enabled` and `order` are
//                 implicit framework keys and are NOT in `config`.
//
// Defaults match the pre-Phase-3 flat SettingsStore defaults (all sizes "medium",
// hideFromRecent true) so a fresh install is byte-for-byte unchanged; migration
// preserves any on-disk value over these.
QtObject {
    id: root

    readonly property var manifests: [
        {
            "id": "moonlight",
            "name": "Moonlight",
            "version": "1.0.0",
            "requires": ["moonlight-backend"],
            "defaultOrder": 0,
            "config": [
                {
                    "key": "size",
                    "type": "enum",
                    "values": ["small", "medium", "large"],
                    "default": "medium",
                    "label": "Size"
                }
            ]
        },
        {
            "id": "nowplaying",
            "name": "Now Playing",
            "version": "1.0.0",
            "requires": ["mpris"],
            "defaultOrder": 1,
            "config": [
                {
                    "key": "size",
                    "type": "enum",
                    "values": ["small", "medium"],
                    "default": "medium",
                    "label": "Size"
                },
                {
                    "key": "hideFromRecent",
                    "type": "bool",
                    "default": true,
                    "label": "Hide from Recent"
                }
            ]
        },
        {
            "id": "plex",
            "name": "Plex",
            "version": "1.0.0",
            "requires": ["plex-backend"],
            "defaultOrder": 2,
            "config": [
                {
                    "key": "size",
                    "type": "enum",
                    "values": ["small", "medium"],
                    "default": "medium",
                    "label": "Size"
                },
                {
                    "key": "hideFromRecent",
                    "type": "bool",
                    "default": true,
                    "label": "Hide from Recent"
                }
            ]
        },
        {
            "id": "recent",
            "name": "Apps",
            "version": "1.1.0",
            "requires": [],
            "defaultOrder": 3,
            "config": [
                {
                    "key": "size",
                    "type": "enum",
                    "values": ["small", "medium"],
                    "default": "medium",
                    "label": "Size"
                }
            ]
        },
        {
            "id": "steam",
            "name": "Steam",
            "version": "1.0.0",
            "requires": [],
            "defaultOrder": 4,
            "defaultEnabled": false,
            "config": [
                {
                    "key": "size",
                    "type": "enum",
                    "values": ["medium", "large"],
                    "default": "medium",
                    "label": "Size"
                }
            ]
        }
    ]

    // Manifest for an id, or null when unknown.
    function byId(id) {
        for (var i = 0; i < root.manifests.length; i++) {
            if (root.manifests[i].id === id)
                return root.manifests[i];
        }
        return null;
    }

    // Ids in framework order.
    function ids() {
        var out = [];
        for (var i = 0; i < root.manifests.length; i++)
            out.push(root.manifests[i].id);
        return out;
    }
}
