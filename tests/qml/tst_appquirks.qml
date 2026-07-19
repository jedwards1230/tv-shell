import QtQuick
import QtTest
import "../../shell/components/appQuirks.js" as AppQuirks

// Headless tests for the per-app quirk table (appQuirks.js). Like prewarm.js it is
// a pure `.pragma library` module imported by its real source path (zero drift) —
// no Quickshell, no stubs.
//
// What these pin:
//  - the table is keyed by prewarm.keyFor(), NOT by a second identity scheme, so
//    an app with an empty StartupWMClass (Steam) still resolves via its exec
//    basename and a declared wmClass still wins over the exec;
//  - a miss returns null, which the close path reads as "just close the window" —
//    the pre-existing behaviour for every app without an override;
//  - the window-driven lookup the close path actually uses resolves a live window
//    back to its app through WindowMatcher before keying the table;
//  - null / malformed inputs return null instead of throwing (the close path runs
//    on a user keypress; a throw there would wedge the popover).
TestCase {
    id: testCase
    name: "AppQuirks"

    // Matcher stub mirroring the WindowMatcher singleton's QML-free helpers,
    // identical in contract to tst_prewarm.qml's.
    readonly property var matcher: ({
            execBasename: function (exec) {
                if (!exec)
                    return "";
                var cmd = exec.split(/\s/)[0];
                return cmd.split("/").pop().toLowerCase();
            },
            normalize: function (s) {
                return (s || "").toLowerCase().replace(/[-_.]/g, "");
            },
            matchesApp: function (app, client) {
                var cls = (client["class"] || "").toLowerCase();
                if (cls === "")
                    return false;
                var wm = (app.wmClass || "").toLowerCase();
                if (wm !== "" && cls === wm)
                    return true;
                var base = this.execBasename(app.exec || "");
                return base !== "" && cls === base;
            }
        })

    // Steam's real desktop entry declares NO StartupWMClass — the exec-basename
    // fallback in keyFor() is the only thing that resolves it to "steam".
    readonly property var steam: ({
            name: "Steam",
            exec: "/home/user/.local/share/Steam/ubuntu12_32/steam %U",
            wmClass: ""
        })
    readonly property var plex: ({
            name: "Plex",
            exec: "/usr/bin/Plex",
            wmClass: "plex"
        })

    // --- strategy hit ---

    function test_quitCommand_hit_via_execBasename() {
        var cmd = AppQuirks.quitCommandFor(testCase.steam, testCase.matcher);
        verify(cmd !== null, "Steam must resolve to a quit command");
        compare(cmd.length, 2);
        compare(cmd[0], "steam");
        compare(cmd[1], "-shutdown");
    }

    function test_quitCommand_hit_via_wmClass() {
        // Same key reached the other way round: a declared StartupWMClass. Proves
        // the table is keyed by keyFor()'s output, not by the exec specifically.
        var app = {
            name: "Steam",
            exec: "/opt/steam-launcher-wrapper",
            wmClass: "steam"
        };
        var cmd = AppQuirks.quitCommandFor(app, testCase.matcher);
        verify(cmd !== null);
        compare(cmd[0], "steam");
    }

    function test_quirksFor_returns_record_not_just_command() {
        // The table holds records so future quirks can be added without
        // restructuring callers.
        var q = AppQuirks.quirksFor(testCase.steam, testCase.matcher);
        verify(q !== null);
        verify(q.quitCommand !== undefined);
    }

    // --- strategy miss => window close ---

    function test_quitCommand_miss_returns_null() {
        compare(AppQuirks.quitCommandFor(testCase.plex, testCase.matcher), null);
    }

    function test_wmClass_wins_over_exec_so_no_false_hit() {
        // An app whose exec basename is "steam" but which declares a DIFFERENT
        // StartupWMClass keys as that wmClass and must NOT inherit Steam's quirk.
        var app = {
            name: "Steam Wrapper",
            exec: "/usr/bin/steam",
            wmClass: "steamwrapper"
        };
        compare(AppQuirks.quitCommandFor(app, testCase.matcher), null);
    }

    // --- null / malformed safety ---

    function test_null_app_is_safe() {
        compare(AppQuirks.quitCommandFor(null, testCase.matcher), null);
        compare(AppQuirks.quitCommandFor(undefined, testCase.matcher), null);
        compare(AppQuirks.quirksFor(null, testCase.matcher), null);
    }

    function test_missing_matcher_is_safe() {
        compare(AppQuirks.quitCommandFor(testCase.steam, null), null);
    }

    function test_app_with_no_identity_is_safe() {
        compare(AppQuirks.quitCommandFor({
            name: "Mystery"
        }, testCase.matcher), null);
    }

    // --- window-driven lookup (the path the close action actually takes) ---

    function test_window_lookup_resolves_app_then_quirk() {
        var apps = [testCase.plex, testCase.steam];
        var cmd = AppQuirks.quitCommandForWindow("steam", apps, testCase.matcher);
        verify(cmd !== null, "a live 'steam' window must resolve to Steam's quit command");
        compare(cmd[0], "steam");
        compare(cmd[1], "-shutdown");
    }

    function test_window_lookup_miss_returns_null() {
        var apps = [testCase.plex, testCase.steam];
        compare(AppQuirks.quitCommandForWindow("plex", apps, testCase.matcher), null);
        compare(AppQuirks.quitCommandForWindow("unknownapp", apps, testCase.matcher), null);
    }

    function test_window_lookup_skips_quirkless_loose_match() {
        // WindowMatcher's last resort is a substring test, so a class can match more
        // than one entry. A quirkless earlier match must not mask the real one.
        var decoy = {
            name: "Steam Runtime Helper",
            exec: "/usr/bin/steam",
            wmClass: "steamhelper"
        };
        var cmd = AppQuirks.quitCommandForWindow("steam", [decoy, testCase.steam], testCase.matcher);
        verify(cmd !== null);
        compare(cmd[1], "-shutdown");
    }

    function test_window_lookup_null_inputs_are_safe() {
        compare(AppQuirks.quitCommandForWindow("", [testCase.steam], testCase.matcher), null);
        compare(AppQuirks.quitCommandForWindow(null, [testCase.steam], testCase.matcher), null);
        compare(AppQuirks.quitCommandForWindow("steam", null, testCase.matcher), null);
        compare(AppQuirks.quitCommandForWindow("steam", [], testCase.matcher), null);
        compare(AppQuirks.quitCommandForWindow("steam", [testCase.steam], null), null);
    }
}
