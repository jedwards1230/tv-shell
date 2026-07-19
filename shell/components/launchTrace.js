.pragma library

// Structured launch tracing — the single formatter for every app-launch
// shell-out the shell performs.
//
// WHY: the shell launches apps from several independent call paths (a foreground
// launch, a silent login prewarm, a single-instance exec redelivery, a stream
// client spawn). When two copies of an app appear, or a window maps with the
// wrong placement, the journal alone could not say WHICH path issued it — every
// one of them lands in the compositor as an identical `hyprctl dispatch exec`
// child. These lines name the path, so one boot's journal answers "who launched
// what, when, with which window rule" without a re-run.
//
// The shell's output is tee'd to BOTH the journal (tagged
// `tv-shell-quickshell`) and /tmp/qs-log.txt — see config/hyprland.conf's
// exec-once. Grep either with the stable prefix below:
//
//   journalctl --user -b -t tv-shell-quickshell | grep 'tv-shell:launch'
//
// LEVEL: console.log, deliberately — it is the level every other diagnostic in
// this shell already uses and is therefore known to survive whatever log
// filtering the runtime applies. A launch is normal operation, not a warning,
// so it must not be escalated to console.warn just to be seen. Both emitters
// below funnel through this one choice, so it is a one-line change if a future
// runtime ever filters it.
//
// Volume is deliberately low — these are launches and one prewarm decision, not
// a hot path. Only an app's desktop-entry Exec line is logged; never the
// environment, never a credential.

// Stable, greppable prefix. Do not change it without updating the docs/grep
// recipes that depend on it.
var PREFIX = "[tv-shell:launch]";

// Rule tag used when a dispatch carries NO Hyprland exec-rule prefix. Logged
// explicitly (rather than omitted) because "no rule" is itself a finding — a
// rule-less exec is not placed fullscreen at map time.
var RULE_NONE = "none";

// Collapse anything that would break the one-line, greppable shape.
function _clean(v) {
    if (v === undefined || v === null)
        return "";
    return String(v).replace(/[\r\n\t]+/g, " ").trim();
}

function _field(key, value) {
    var v = _clean(value);
    return key + "=" + (v === "" ? "-" : v);
}

// Format one launch dispatch.
//   origin  — the CALL PATH that issued it (launch / prewarm / redeliver /
//             stream). This is the field that identifies the culprit.
//   rule    — the Hyprland exec-rule prefix actually used ("[fullscreen]",
//             "[silent]") or "" for a rule-less dispatch.
//   name    — the app's display name.
//   wmClass — its StartupWMClass, when the desktop entry declares one.
//   comm    — the process name `ps -eo comm=` will report for it
//             (WindowMatcher.execBasename of the exec line). This is the field
//             to correlate a journal line with a live pid.
//   exec    — the exec line handed to the compositor.
function formatExec(origin, rule, name, wmClass, comm, exec) {
    return PREFIX + " " + [_field("origin", origin), _field("rule", rule || RULE_NONE), _field("app", name), _field("class", wmClass), _field("comm", comm), _field("exec", exec)].join(" ");
}

function logExec(origin, rule, name, wmClass, comm, exec) {
    console.log(formatExec(origin, rule, name, wmClass, comm, exec));
}

// Render prewarm.js's skip list ([{key, reason}, …]) as one compact field.
function _skips(skipped) {
    var parts = [];
    for (var i = 0; i < (skipped || []).length; i++) {
        var s = skipped[i] || {};
        parts.push(_clean(s.key) + ":" + _clean(s.reason));
    }
    return parts.join(",");
}

// Format the login prewarm pass's ONE decided evaluation: what it saw (how many
// configured keys, mapped windows, and live processes) and what it chose. This
// is what distinguishes "prewarm launched it" from "something else did" when two
// copies of an app show up seconds apart.
function formatDecision(configured, windowCount, procCount, launchKeys, skipped) {
    return PREFIX + " " + [_field("origin", "prewarm-decision"), _field("configured", configured), _field("windows", windowCount), _field("procs", procCount), _field("launch", (launchKeys || []).join(",")), _field("skip", _skips(skipped))].join(" ");
}

function logDecision(configured, windowCount, procCount, launchKeys, skipped) {
    console.log(formatDecision(configured, windowCount, procCount, launchKeys, skipped));
}

// The prewarm pass ran but the snapshot was unusable, so it decided NOTHING and
// will retry. Logged once per shell process by the caller — a repeating `ps`
// failure is a real fault worth seeing, but not worth a line every poll.
function logUndecided(reason) {
    console.warn(PREFIX + " " + [_field("origin", "prewarm-decision"), _field("decided", "false"), _field("reason", reason)].join(" "));
}
