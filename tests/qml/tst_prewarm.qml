import QtQuick
import QtTest
import "../../shell/components/prewarm.js" as Prewarm

// Headless tests for the login-prewarm decision engine (#238 follow-up).
// prewarm.js is a pure `.pragma library` module imported by its real source path
// (zero drift) — no Quickshell, no stubs. These pin the invariants the fix exists
// for: a candidate is suppressed when its PROCESS is alive even though its WINDOW
// has not mapped yet (the double-launch regression), a genuinely-absent candidate
// launches on the very FIRST poll (no delay), process matching is exact enough
// that `steamwebhelper` cannot suppress `steam`, and an unusable snapshot decides
// nothing rather than launching blind.
TestCase {
    id: testCase
    name: "Prewarm"

    // Minimal matcher stub mirroring the WindowMatcher singleton's two QML-free
    // helpers prewarm.js relies on (extends tst_resumemodel.qml's stub with
    // matchesApp).
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

    readonly property var plex: ({
            name: "Plex",
            exec: "/usr/bin/Plex",
            wmClass: "plex"
        })
    readonly property var steam: ({
            name: "Steam",
            exec: "/home/user/.local/share/Steam/ubuntu12_32/steam %U",
            wmClass: ""
        })

    function emptyState() {
        return {
            issued: {}
        };
    }

    // --- 1. THE regression: process alive, window not mapped yet → NO launch ---
    // An out-of-band Plex launch ~1s earlier has a live process but no window for
    // 10-15s. The window snapshot is honestly empty; the process table is not.
    // Suppression must happen on the FIRST poll, with no settle delay.
    function test_live_process_without_window_suppresses_launch() {
        var procs = ["systemd", "Plex", "Plex", "QtWebEngineProc"];
        var r = Prewarm.evaluate(["plex"], [testCase.plex], [], procs, emptyState(), testCase.matcher);
        verify(r.decided, "a usable snapshot decides");
        compare(r.launch.length, 0, "never launches a second copy while the process is alive");
        compare(r.state.issued["plex"], undefined, "and does not mark it issued");
    }

    // --- 2. THE requirement: genuinely absent → launches on the FIRST poll ------
    // No window, no process. This is the "as early as possible" guarantee.
    function test_launches_immediately_when_genuinely_absent() {
        var procs = ["systemd", "bash", "quickshell"];
        var r = Prewarm.evaluate(["plex"], [testCase.plex], [], procs, emptyState(), testCase.matcher);
        verify(r.decided);
        compare(r.launch.length, 1, "launched on the very first poll — no settle window");
        compare(r.launch[0].name, "Plex");
        compare(r.state.issued["plex"], true, "marked issued the instant it is dispatched");
    }

    // --- 3. `issued` is sticky across polls (the in-flight dedup) -------------
    // Our own launch's window/process may not appear for 10-15s; `issued` is what
    // stops the next poll launching a second copy in that gap.
    function test_issued_is_sticky() {
        var st = emptyState();
        var procs = [];
        var r = Prewarm.evaluate(["plex"], [testCase.plex], [], procs, st, testCase.matcher);
        compare(r.launch.length, 1, "first poll launches");
        st = r.state;
        // Second poll: still no window AND still no process (cold start in flight).
        r = Prewarm.evaluate(["plex"], [testCase.plex], [], procs, st, testCase.matcher);
        compare(r.launch.length, 0, "issued suppresses the second launch");
        compare(r.state.issued["plex"], true, "issued survives the round-trip");
    }

    // --- 4. Precision guard: steamwebhelper must NOT suppress steam -----------
    // The exact-comm rule exists for this. A substring match over "steam" would
    // hit every one of these helpers and silently break Steam prewarm forever.
    function test_steam_helpers_do_not_suppress_steam() {
        var procs = ["steamwebhelper", "steam-runtime-l", "srt-logger", "pv-adverb", "steamerrorrepor"];
        var r = Prewarm.evaluate(["steam"], [testCase.steam], [], procs, emptyState(), testCase.matcher);
        compare(r.launch.length, 1, "helper processes must not suppress the real prewarm");
        compare(r.launch[0].name, "Steam");
        // ...but the real `steam` process does suppress it.
        procs.push("steam");
        r = Prewarm.evaluate(["steam"], [testCase.steam], [], procs, emptyState(), testCase.matcher);
        compare(r.launch.length, 0, "the real steam process suppresses it");
    }

    // --- 5. comm truncation: a >15-char exec basename still matches -----------
    function test_long_exec_basename_matches_truncated_comm() {
        var longApp = {
            name: "Web Engine Thing",
            exec: "/usr/lib/QtWebEngineProcess",
            wmClass: ""
        };
        // Linux reports this as the 15-char `QtWebEngineProc`.
        var r = Prewarm.evaluate(["qtwebengineprocess"], [longApp], [], ["QtWebEngineProc"], emptyState(), testCase.matcher);
        compare(r.launch.length, 0, "matches the truncated comm, so does not double-launch");
        // Absent from the process table → still launches normally.
        r = Prewarm.evaluate(["qtwebengineprocess"], [longApp], [], ["systemd"], emptyState(), testCase.matcher);
        compare(r.launch.length, 1, "a long-named app is still prewarmable when genuinely absent");
    }

    // --- 6. A mapped window also suppresses (the original signal, unchanged) ---
    function test_mapped_window_suppresses_launch() {
        var clients = [
            {
                "class": "plex"
            }
        ];
        var r = Prewarm.evaluate(["plex"], [testCase.plex], clients, ["systemd"], emptyState(), testCase.matcher);
        compare(r.launch.length, 0, "an already-mapped window is already instant-resume");
        verify(r.decided);
    }

    // --- 7. Unusable snapshot decides nothing --------------------------------
    function test_bad_snapshot_is_a_noop() {
        var st = {
            issued: {
                "other": true
            }
        };
        // No process list (the `ps` call failed): must NOT launch blind.
        var r = Prewarm.evaluate(["plex"], [testCase.plex], [], null, st, testCase.matcher);
        verify(!r.decided, "a missing process list is not a decision");
        compare(r.launch.length, 0);
        compare(r.state.issued["other"], true, "the caller's state is handed straight back");
        // No window list either.
        r = Prewarm.evaluate(["plex"], [testCase.plex], null, ["systemd"], st, testCase.matcher);
        verify(!r.decided, "a missing window list is not a decision");
        compare(r.launch.length, 0);
    }

    // --- 8. Blank + duplicate keys are skipped (behaviour moved from QML) -----
    function test_blank_and_duplicate_keys_skipped() {
        var r = Prewarm.evaluate(["", "plex", "plex", "nosuchapp"], [testCase.plex], [], ["systemd"], emptyState(), testCase.matcher);
        compare(r.launch.length, 1, "one launch despite the duplicate entry");
        compare(r.launch[0].name, "Plex");
    }

    // --- 9. keyFor falls back to the exec basename (Steam, item 3b) ----------
    function test_keyFor_falls_back_to_exec_basename() {
        compare(Prewarm.keyFor(testCase.steam, testCase.matcher), "steam");
        compare(Prewarm.keyFor(testCase.plex, testCase.matcher), "plex", "wmClass still wins when present");
        compare(Prewarm.keyFor(null, testCase.matcher), "");
    }

    // --- 10. resolveApp prefers an exact wmClass over the basename fallback ---
    // Proves existing configs are unaffected by the fallback tier.
    function test_resolveApp_prefers_exact_wmClass() {
        var realSteam = {
            name: "Steam (declared)",
            exec: "/usr/bin/steam",
            wmClass: "steam"
        };
        var apps = [testCase.steam, realSteam];
        compare(Prewarm.resolveApp("steam", apps, testCase.matcher).name, "Steam (declared)");
        // With only the wmClass-less entry present, the fallback tier resolves it.
        compare(Prewarm.resolveApp("steam", [testCase.steam], testCase.matcher).name, "Steam");
        compare(Prewarm.resolveApp("nope", apps, testCase.matcher), null);
    }
}
