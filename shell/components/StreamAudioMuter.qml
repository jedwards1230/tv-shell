import QtQuick
import Quickshell.Io

// StreamAudioMuter — mute a backgrounded native stream app's PipeWire audio so
// it doesn't keep playing behind the shell, and restore it on refocus/exit.
//
// Scope is intentionally NARROW: only apps whose window class matches the
// `streamClasses` allowlist (default ["steam"], for Steam Remote Play) are ever
// touched. All other audio — the default sink, other apps — is never affected.
//
// How shell.qml drives it (both properties are injected bindings):
//   - shellState       ← root.state          ("idle" | "appRunning" | …)
//   - runningAppClass  ← appLifecycle.runningAppClass (window class, "" when none)
//
// Transitions (see shell.qml AppLifecycleManager + returnToShell):
//   - background: appRunning → idle while runningAppClass still holds a stream
//     class → MUTE that app's stream nodes.
//   - refocus:   idle → appRunning (same app) → shellState leaves "idle" →
//     UNMUTE.
//   - close:     onAppClosed clears runningAppClass (→ "") then returnToShell();
//     runningAppClass going empty while a mute is outstanding → UNMUTE (cleanup,
//     never strand a muted node).
//
// The desired state is a single pure predicate (`_desiredToken`) recomputed on
// every state/class change; a serialized pump reconciles the actual applied
// state (`_appliedToken`) toward it, so even rapid background→refocus flips
// can't strand a muted node.
Item {
    id: muter

    // Injected by shell.qml.
    property string shellState: "idle"
    property string runningAppClass: ""

    // The ONLY knob that decides what gets auto-muted. Window classes here are
    // matched case-insensitively (exact OR substring) against runningAppClass.
    // Keep this a tiny, deliberate allowlist — do NOT try to cover arbitrary
    // apps. Extend by adding a class token; the same token is used to identify
    // the app's PipeWire nodes (see the mute pipeline below).
    readonly property var streamClasses: ["steam"]

    // Token we WANT muted right now ("" = nothing should be muted).
    property string _desiredToken: ""
    // Token the last completed wpctl run actually left muted ("" = none). This
    // is the source of truth for what's really muted on the graph.
    property string _appliedToken: ""

    // Return the matched (lowercased) stream token for a window class, or "" if
    // the class isn't in the allowlist.
    function _matchStreamClass(cls) {
        if (!cls)
            return "";
        var lc = cls.toLowerCase();
        for (var i = 0; i < streamClasses.length; i++) {
            var s = streamClasses[i].toLowerCase();
            if (lc === s || lc.indexOf(s) >= 0)
                return s;
        }
        return "";
    }

    // Recompute the desired mute state and pump toward it. Mute only when a
    // stream app is BACKGROUNDED (shell idle, its class still set); anything
    // else (foreground/gone/non-stream) means nothing should be muted.
    function _evaluate() {
        muter._desiredToken = (shellState === "idle") ? _matchStreamClass(runningAppClass) : "";
        _pump();
    }

    // Serialize wpctl runs and drive _appliedToken toward _desiredToken. A no-op
    // when already in sync or while a run is in flight (onExited re-pumps).
    function _pump() {
        if (muteProc.running)
            return;
        if (muter._desiredToken === muter._appliedToken)
            return;
        if (muter._desiredToken !== "") {
            // Mute the desired stream app.
            muteProc.token = muter._desiredToken;
            muteProc.action = "1";
            muteProc.targetToken = muter._desiredToken;
            NotificationManager.info("stream", "Stream muted");
        } else {
            // Unmute whatever we currently hold muted.
            muteProc.token = muter._appliedToken;
            muteProc.action = "0";
            muteProc.targetToken = "";
            NotificationManager.info("stream", "Stream unmuted");
        }
        muteProc.running = true;
    }

    onShellStateChanged: _evaluate()
    onRunningAppClassChanged: _evaluate()

    // Enumerate the app's PipeWire playback-stream nodes and set their mute in a
    // SINGLE invocation (resolve-then-apply — no ids round-tripped back to QML,
    // so a node appearing/vanishing between enumerate and apply can't race).
    //
    // The app token is passed via env (GS_STREAM_TOKEN) so its content can never
    // be shell-injected. Matching is case-insensitive across several props —
    // application.name / application.process.binary / node.name / media.name —
    // because Steam Remote Play's audio often lives in a CHILD process/node, not
    // one whose name matches the window class. We scope to playback streams
    // (media.class "Stream/Output/Audio") so only the app's sink-inputs are
    // touched, never a device/sink.
    //
    // jq is preferred (robust prop matching) but NOT guaranteed on the device —
    // fall back to scoping wpctl status' Streams block and grepping the token
    // (mirrors AudioSettings' jq-optional pattern). No matching node → clean
    // no-op, never an error.
    Process {
        id: muteProc

        property string token: ""
        property string action: "1" // "1" = mute, "0" = unmute
        // Value _appliedToken becomes once this run completes successfully.
        property string targetToken: ""

        environment: ({
                "GS_STREAM_TOKEN": token,
                "GS_MUTE": action
            })
        command: ["bash", "-c", "tok=\"$GS_STREAM_TOKEN\"; act=\"$GS_MUTE\"; " + "[ -z \"$tok\" ] && exit 0; " + "if command -v jq >/dev/null 2>&1; then " + "  ids=$(pw-dump 2>/dev/null | jq -r --arg t \"$tok\" '" + ".[] | select(.info.props != null) " + "| select((.info.props[\"media.class\"] // \"\") | test(\"Stream/Output/Audio\")) " + "| select( ((.info.props[\"application.name\"] // \"\") | ascii_downcase | contains($t)) " + "  or ((.info.props[\"application.process.binary\"] // \"\") | ascii_downcase | contains($t)) " + "  or ((.info.props[\"node.name\"] // \"\") | ascii_downcase | contains($t)) " + "  or ((.info.props[\"media.name\"] // \"\") | ascii_downcase | contains($t)) ) " + "| .id'); " + "else " + "  ids=$(wpctl status 2>/dev/null | awk '/Streams:/{s=1;next} /Sinks:|Sources:|Filters:|Devices:|Clients:|^Video/{s=0} s' | grep -iF \"$tok\" | grep -oE '[0-9]+\\.' | tr -d '.'); " + "fi; " + "for id in $ids; do wpctl set-mute \"$id\" \"$act\" 2>/dev/null || true; done; " + "exit 0"]

        onExited: {
            muter._appliedToken = muteProc.targetToken;
            // Reconcile any transition that arrived while this run was in flight.
            muter._pump();
        }
    }
}
