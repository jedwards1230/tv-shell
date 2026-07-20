import QtQuick
import QtTest
import "../../shell/components/launchTrace.js" as LaunchTrace

// Headless tests for the launch-trace formatter. launchTrace.js is a pure
// `.pragma library` imported by its real source path (zero drift) — no
// Quickshell, no stubs.
//
// These pin the WIRE FORMAT, not prose: the whole point of the instrumentation
// is that a boot journal can be grepped and parsed months from now, so the
// stable prefix, the `origin` tag, and the `rule` field are a contract. A
// reworded comment is free; a renamed field silently breaks every saved grep.
TestCase {
    id: testCase
    name: "LaunchTrace"

    function test_prefix_is_stable() {
        compare(LaunchTrace.PREFIX, "[tv-shell:launch]", "the greppable prefix is a contract — docs and saved greps depend on it");
    }

    function test_exec_line_carries_every_correlation_field() {
        var line = LaunchTrace.formatExec("launch", "[fullscreen]", "Plex", "tv.plex.Plex", "plex", "/usr/bin/Plex");
        verify(line.indexOf("[tv-shell:launch]") === 0, "prefix leads the line so a grep anchors on it");
        verify(line.indexOf("origin=launch") >= 0, "the origin tag names the call path — the whole point");
        verify(line.indexOf("rule=[fullscreen]") >= 0);
        verify(line.indexOf("app=Plex") >= 0);
        verify(line.indexOf("class=tv.plex.Plex") >= 0);
        verify(line.indexOf("comm=plex") >= 0, "comm is the field that correlates the line with `ps -eo comm=`");
        verify(line.indexOf("exec=/usr/bin/Plex") >= 0);
    }

    // A rule-less dispatch must be visibly distinct from a ruled one — it is
    // exactly the case where a window maps unplaced, so it can never render as
    // an empty/absent field.
    function test_missing_rule_renders_as_none() {
        var line = LaunchTrace.formatExec("redeliver", "", "Plex", "tv.plex.Plex", "plex", "/usr/bin/Plex");
        verify(line.indexOf("rule=none") >= 0, "no exec-rule prefix logs as an explicit 'none'");
        verify(line.indexOf("origin=redeliver") >= 0);
    }

    function test_absent_fields_never_collapse_the_line() {
        var line = LaunchTrace.formatExec("stream", "", "", null, "", undefined);
        verify(line.indexOf("app=-") >= 0, "an empty field renders as a placeholder, keeping the shape parseable");
        verify(line.indexOf("class=-") >= 0);
        verify(line.indexOf("exec=-") >= 0);
    }

    // The trace must stay ONE greppable line even if a desktop entry's Exec
    // carries an embedded newline or tab.
    function test_line_stays_single_line() {
        var line = LaunchTrace.formatExec("launch", "[silent]", "Weird\nName", "", "x", "a\tb\nc");
        compare(line.indexOf("\n"), -1, "no embedded newline survives into the journal line");
        compare(line.indexOf("\t"), -1);
    }

    function test_decision_line_reports_what_it_saw_and_chose() {
        var line = LaunchTrace.formatDecision(1, 3, 210, ["plex"], [
            {
                key: "steam",
                reason: "process-alive"
            }
        ]);
        verify(line.indexOf("origin=prewarm-decision") >= 0);
        verify(line.indexOf("configured=1") >= 0);
        verify(line.indexOf("windows=3") >= 0);
        verify(line.indexOf("procs=210") >= 0);
        verify(line.indexOf("launch=plex") >= 0);
        verify(line.indexOf("skip=steam:process-alive") >= 0, "the skip reason is what distinguishes 'prewarm chose not to' from 'prewarm never ran'");
    }

    function test_decision_line_with_nothing_launched() {
        var line = LaunchTrace.formatDecision(1, 0, 0, [], []);
        verify(line.indexOf("launch=-") >= 0, "a pass that launched nothing says so explicitly");
        verify(line.indexOf("skip=-") >= 0);
    }

    // --- Resume tracing (#347) ---
    //
    // These lines exist because the resume path had NO output at all: a resume
    // that focused nothing was indistinguishable from one that worked. They
    // share the launch prefix on purpose — "why is this app not on screen?" is
    // one question, answerable by one grep, whether the cause was a launch that
    // never happened or a resume that landed nowhere.

    function test_resume_line_shares_the_launch_prefix() {
        var line = LaunchTrace.formatResume("none", "0xstale", "", "unknown-address");
        verify(line.indexOf(LaunchTrace.PREFIX) === 0, "one prefix answers 'why is this app not on screen'");
        verify(line.indexOf("origin=resume") >= 0, "origin distinguishes a resume from a launch on the shared prefix");
        verify(line.indexOf("mode=none") >= 0);
        verify(line.indexOf("address=0xstale") >= 0);
        verify(line.indexOf("reason=unknown-address") >= 0, "the reason is the whole diagnostic value of the line");
    }

    function test_resume_fallback_line_names_the_class_it_used() {
        var line = LaunchTrace.formatResume("class", "0xstale", "tv.plex.Plex", "unknown-address");
        verify(line.indexOf("mode=class") >= 0);
        verify(line.indexOf("class=tv.plex.Plex") >= 0);
    }

    // A resume with nothing to act on must still render every field — an absent
    // field would make the line unparseable exactly when it matters most.
    function test_resume_line_never_collapses() {
        var line = LaunchTrace.formatResume("none", "", null, undefined);
        verify(line.indexOf("address=-") >= 0);
        verify(line.indexOf("class=-") >= 0);
        verify(line.indexOf("reason=-") >= 0);
        compare(line.indexOf("\n"), -1);
    }

    // The verify line is the ONLY evidence that a focus dispatch landed —
    // `hyprctl dispatch` exits 0 even when it matched no window.
    function test_focus_verify_line_reports_wanted_vs_actual() {
        var line = LaunchTrace.formatFocusVerify("address", "0xaaa", "steam", "active-address-mismatch");
        verify(line.indexOf("origin=resume-verify") >= 0);
        verify(line.indexOf("wanted=0xaaa") >= 0, "what we aimed at");
        verify(line.indexOf("active=steam") >= 0, "what actually ended up focused — the pair is the diagnosis");
        verify(line.indexOf("reason=active-address-mismatch") >= 0);
    }

    function test_focus_verify_line_with_nothing_active() {
        var line = LaunchTrace.formatFocusVerify("class", "tv.plex.Plex", "", "no-active-window");
        verify(line.indexOf("active=-") >= 0, "'nothing is focused' must be explicit, not an empty field");
    }
}
