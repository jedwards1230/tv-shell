// The design system, as one singleton. Every colour, size, weight and duration
// in the shell comes from here; a literal anywhere else is a bug.
//
// WHAT IS CARRIED FROM v1 AND WHAT IS NOT
//
// The DECISIONS in `config/palette.md` and v1's `Theme.qml` are real and were
// made against an OLED television at couch distance, so they are carried:
// crimson for focus and active states, ember for secondary interaction, gold as
// decoration and NEVER as text, near-black rather than pure black for an OLED
// panel, and type sized so it reads from three metres.
//
// The CODE is not carried. v1's Theme.qml is an `Item` hosting Processes and
// Timers because it also owns theme-mode polling; this is a plain QtObject that
// holds values and computes nothing but sizes.
//
// SCALE
//
// Every size below is written for the 4K panel the couch actually runs, and
// multiplied by `scale`. `scale` has exactly ONE writer — Main.qml, from the
// base surface's height, via the pure `viewport.scaleFor` — so a size is never a
// function of which window happens to be asking.
//
// DARK ONLY, FOR NOW, AND WHY
//
// v1 has a light palette selected by `themeMode` in `settings.json`. v2 has no
// settings store: the core owns state and publishes none of that kind, so a
// theme toggle here would have nowhere to persist to and would reset on every
// restart. The palette is therefore a single object rather than a ternary at
// each site — adding light mode is a second object and a selector, not an edit
// to every colour reference.
pragma Singleton

import QtQuick
import "viewport.js" as Viewport

QtObject {
    id: tokens

    // ---- scale ------------------------------------------------------------

    // Set once by Main.qml. 1.0 is the 4K reference.
    property real scale: 1.0

    // Convenience for the one caller: keeps `viewport.js` the only place the
    // clamping rule lives, rather than duplicating it at the assignment site.
    function scaleForHeight(height: real): real {
        return Viewport.scaleFor(height);
    }

    // ---- palette ----------------------------------------------------------

    readonly property color background: "#111215"   // near-black, not #000 (OLED)
    readonly property color surface: "#1c1e24"
    readonly property color surfaceRaised: "#2e3139"
    readonly property color surfaceBorder: "#3a3e48"

    readonly property color textPrimary: "#e6e4e0"
    readonly property color textSecondary: "#c2bfba"
    readonly property color textMuted: "#928e88"

    readonly property color crimson: "#c72138"      // focus and active states
    readonly property color ember: "#e06236"        // secondary interaction, warnings
    readonly property color online: "#2d8a4e"

    // Focus is a crimson ring, everywhere, at one width. A component that draws
    // its own focus treatment is a component that will disagree with the one
    // next to it.
    readonly property color focusRing: crimson
    readonly property real focusRingWidth: 4 * tokens.scale

    // Every scrim in the shell. v1's palette rule is 0.7–0.85; one value, so two
    // overlays never look like different products.
    readonly property color scrim: Qt.rgba(0, 0, 0, 0.8)

    // ---- type -------------------------------------------------------------
    // Sized for three metres. These are the numbers not to shrink.

    readonly property int fontDisplay: Math.round(96 * tokens.scale)   // the clock
    readonly property int fontTitle: Math.round(52 * tokens.scale)     // rail headers
    readonly property int fontBody: Math.round(34 * tokens.scale)      // card titles
    readonly property int fontCaption: Math.round(26 * tokens.scale)   // subtitles, status

    // ---- space ------------------------------------------------------------

    readonly property real spaceXS: Math.round(8 * tokens.scale)
    readonly property real spaceS: Math.round(16 * tokens.scale)
    readonly property real spaceM: Math.round(32 * tokens.scale)
    readonly property real spaceL: Math.round(56 * tokens.scale)
    readonly property real spaceXL: Math.round(96 * tokens.scale)

    readonly property real radius: Math.round(14 * tokens.scale)

    // ---- components -------------------------------------------------------

    readonly property real cardWidth: Math.round(440 * tokens.scale)
    readonly property real cardHeight: Math.round(260 * tokens.scale)
    readonly property real drawerWidth: Math.round(720 * tokens.scale)

    // One duration for every transition in the shell. Slower than a desktop's
    // because the eye is further away and there is no cursor to follow.
    readonly property int durationFast: 120
    readonly property int durationBase: 220
}
