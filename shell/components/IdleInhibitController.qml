import QtQuick
import Quickshell.Services.Mpris

// Pure, headless-testable policy for the shell-side Wayland idle-inhibitor (#195).
//
// The shell asserts a Wayland idle-inhibitor ONLY in the two cases it *knows*
// video is playing:
//   1. its own Moonlight `streaming` state, and
//   2. an `appRunning` app while an MPRIS player reports Playing (Plex/mpv,
//      which may or may not assert their own inhibitor).
//
// It deliberately does NOT inhibit on a static app screen or the idle home
// screen, so a compositor idle daemon (hypridle/DPMS — a system concern outside
// this repo) can still blank those for OLED burn-in protection while honoring
// these inhibitors. This is the selective counterpart to the blanket
// "idleinhibit fullscreen" windowrule that docs/KIOSK_WINDOW_MODEL.md rejected.
//
// Kept dependency-light on purpose (QtQuick + Mpris only, no Theme/SettingsStore/
// Units) so it loads and runs under the headless qmltestrunner harness.
Item {
    id: controller

    // Injected by shell.qml — mirrors root.state ("idle" | "launching" |
    // "streaming" | "reconnecting" | "appRunning").
    property string shellState: "idle"

    // True iff any MPRIS player on the bus reports Playing. Null-safe scan
    // mirroring MprisPlayerBase's access pattern (Mpris.players may be null).
    readonly property bool mediaPlaying: {
        let list = Mpris.players ? Mpris.players.values : [];
        if (!list || list.length === 0)
            return false;
        for (let i = 0; i < list.length; i++) {
            if (list[i] && list[i].isPlaying)
                return true;
        }
        return false;
    }

    // Whether the shell should assert a Wayland idle-inhibitor.
    readonly property bool shouldInhibit: shellState === "streaming" || (shellState === "appRunning" && mediaPlaying)
}
