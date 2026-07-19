import QtQuick
import QtTest
import "../../shell/components/prewarm.js" as Prewarm

// Headless tests for the login-prewarm decision engine (#238 follow-up).
// prewarm.js is a pure `.pragma library` module imported by its real source path
// (zero drift) — no Quickshell, no stubs. These pin the two invariants the fix
// exists for: a candidate must be ABSENT for `settlePolls` consecutive polls
// before it is launched (so a slow-mapping out-of-band launch is not
// double-launched), and a key resolves by exec basename when the desktop entry
// declares no StartupWMClass (Steam).
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
            exec: "/usr/bin/plex-htpc",
            wmClass: "plex"
        })
    readonly property var steam: ({
            name: "Steam",
            exec: "/usr/bin/steam %U",
            wmClass: ""
        })

    function emptyState() {
        return {
            absent: {},
            issued: {}
        };
    }

    // --- 1. THE regression: a slow-mapping out-of-band launch is not doubled --
    // Polls 1-2 see no Plex window (the external launch is still cold-starting),
    // poll 3 the window finally maps. With settlePolls = 4 the candidate never
    // reaches the threshold, so prewarm must NEVER launch a second copy.
    function test_settle_window_suppresses_slow_mapper() {
        var st = emptyState();
        var apps = [testCase.plex];
        var list = ["plex"];
        var r;
        // Polls 1-2: nothing mapped yet.
        for (var p = 0; p < 2; p++) {
            r = Prewarm.evaluate(list, apps, [], st, testCase.matcher, 4);
            compare(r.launch.length, 0, "no launch while still settling (poll " + (p + 1) + ")");
            verify(r.pending > 0, "candidate still pending");
            st = r.state;
        }
        compare(st.absent["plex"], 2, "absent counter accumulated");
        // Poll 3+: the out-of-band window mapped — counter resets, forever.
        var clients = [
            {
                "class": "plex"
            }
        ];
        for (var q = 0; q < 6; q++) {
            r = Prewarm.evaluate(list, apps, clients, st, testCase.matcher, 4);
            compare(r.launch.length, 0, "never launches once the window is seen");
            compare(r.state.absent["plex"], 0, "absent counter reset to 0");
            st = r.state;
        }
    }

    // --- 2. Genuinely-absent app IS launched, exactly once, after the settle ---
    function test_launches_after_settle() {
        var st = emptyState();
        var apps = [testCase.plex];
        var list = ["plex"];
        var r;
        for (var p = 1; p <= 3; p++) {
            r = Prewarm.evaluate(list, apps, [], st, testCase.matcher, 4);
            compare(r.launch.length, 0, "no launch on poll " + p);
            st = r.state;
        }
        r = Prewarm.evaluate(list, apps, [], st, testCase.matcher, 4);
        compare(r.launch.length, 1, "launched on the 4th consecutive absent poll");
        compare(r.launch[0].name, "Plex");
        compare(r.pending, 0, "nothing left to decide");
        st = r.state;
        // Polls 5-8: still no window (the launch is cold-starting) — no re-launch.
        for (var q = 5; q <= 8; q++) {
            r = Prewarm.evaluate(list, apps, [], st, testCase.matcher, 4);
            compare(r.launch.length, 0, "no second launch on poll " + q);
            compare(r.pending, 0);
            st = r.state;
        }
    }

    // --- 3. `issued` is sticky across polls (the in-flight dedup) -------------
    function test_issued_is_sticky() {
        var st = {
            absent: {},
            issued: {
                "plex": true
            }
        };
        var r = Prewarm.evaluate(["plex"], [testCase.plex], [], st, testCase.matcher, 1);
        compare(r.launch.length, 0, "a pre-issued key never launches again");
        compare(r.pending, 0);
        compare(r.state.issued["plex"], true, "issued survives the round-trip");
    }

    // --- 4. Today's happy path: running from the first poll → never launches ---
    function test_running_from_first_poll_never_launches() {
        var st = emptyState();
        var clients = [
            {
                "class": "plex"
            }
        ];
        var r = Prewarm.evaluate(["plex"], [testCase.plex], clients, st, testCase.matcher, 4);
        compare(r.launch.length, 0);
        compare(r.pending, 0, "an already-running candidate is decided immediately");
    }

    // --- 5. Blank + duplicate keys are skipped (behaviour moved from QML) -----
    function test_blank_and_duplicate_keys_skipped() {
        var st = emptyState();
        var r = Prewarm.evaluate(["", "plex", "plex", "nosuchapp"], [testCase.plex], [], st, testCase.matcher, 1);
        compare(r.launch.length, 1, "one launch despite the duplicate entry");
        compare(r.launch[0].name, "Plex");
        compare(r.pending, 0, "the blank and the unresolvable key add no pending work");
    }

    // --- 6. keyFor falls back to the exec basename (Steam, item 3b) ----------
    function test_keyFor_falls_back_to_exec_basename() {
        compare(Prewarm.keyFor(testCase.steam, testCase.matcher), "steam");
        compare(Prewarm.keyFor(testCase.plex, testCase.matcher), "plex", "wmClass still wins when present");
        compare(Prewarm.keyFor(null, testCase.matcher), "");
    }

    // --- 7. resolveApp prefers an exact wmClass over the basename fallback ----
    // Proves existing configs are unaffected by the new fallback tier.
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

    // --- 8. A bad snapshot decides nothing and leaves state untouched --------
    function test_bad_snapshot_is_a_noop() {
        var st = {
            absent: {
                "plex": 2
            },
            issued: {}
        };
        var r = Prewarm.evaluate(["plex"], [testCase.plex], null, st, testCase.matcher, 4);
        compare(r.pending, -1, "signals 'no decision made'");
        compare(r.launch.length, 0);
        compare(r.state.absent["plex"], 2, "the caller's state is handed straight back");
    }
}
