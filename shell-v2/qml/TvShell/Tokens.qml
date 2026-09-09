// The design system, as one singleton. Every colour, size, weight and duration
// in the shell comes from here; a literal anywhere else is a bug.
//
// WHAT IS CARRIED FROM v1 AND WHAT IS NOT
//
// The DECISIONS in `config/palette.md` and v1's `Theme.qml` are real and were
// made against an OLED television at couch distance, so they are carried:
// crimson for focus and active states, ember for secondary interaction, gold as
// decoration and NEVER as text, and near-black rather than pure black for an
// OLED panel.
//
// The SIZING decisions are carried too, now — as v1's ratios of `gridUnit`
// rather than as numbers retyped by eye. They were not, at first, and the
// difference was visible on the television; see `gridUnit` below.
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

    // THE UNIT EVERY SIZE IS A MULTIPLE OF.
    //
    // 54 at 4K, which is v1's `Units.gridUnit` — `screenHeight / 40` — expressed
    // through the scale this file already had. Sizes below are v1's ratios of it.
    //
    // This is not decoration, it is the fix for a measured defect. The first
    // version of this file used fresh 4K constants, and they came out well under
    // v1's calibrated values: the clock 80%, card titles 85%, and card HEIGHT
    // 54% — a card of 40% the area. On the television the home screen occupied
    // roughly the top-left fifth of a 4K panel (observed 2026-09-08), and the
    // arithmetic matched: ~39% of the width and ~42% of the height.
    //
    // Nothing was scaled wrong and no window was mis-sized — the content simply
    // was that small. `CLAUDE.md` says "10-foot UI at 4K … Don't shrink them",
    // and it had been shrunk. v1's numbers are calibrated against the same panel
    // at the same viewing distance, so adopting its ratios restores a
    // calibration rather than inventing a new one.
    //
    // Keep new sizes as ratios of `gridUnit`. A raw pixel constant here is the
    // same mistake in a new place: it looks reasonable in a text editor and is
    // only wrong on a television.
    readonly property real gridUnit: Math.max(8, Math.round(54 * tokens.scale))

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
    // v1's borderThick ratio, floored the way v1 floors it: a focus ring that
    // rounds to a hairline is a focus ring you cannot see from the couch.
    readonly property real focusRingWidth: Math.max(3, Math.round(tokens.gridUnit * 0.11))

    // Every scrim in the shell. v1's palette rule is 0.7–0.85; one value, so two
    // overlays never look like different products.
    readonly property color scrim: Qt.rgba(0, 0, 0, 0.8)

    // ---- type -------------------------------------------------------------
    // Sized for three metres. These are the numbers not to shrink.

    // v1's ratios: fontHero, fontTitle, fontBody, fontCaption. 120/56/40/28 at 4K.
    readonly property int fontDisplay: Math.round(tokens.gridUnit * 2.22)  // the clock
    readonly property int fontTitle: Math.round(tokens.gridUnit * 1.04)    // rail headers
    readonly property int fontBody: Math.round(tokens.gridUnit * 0.74)     // card titles
    readonly property int fontCaption: Math.round(tokens.gridUnit * 0.52)  // subtitles, status

    // ---- space ------------------------------------------------------------

    // v1's spacingXS..XL. 8/16/24/32/48 at 4K.
    //
    // Note these went DOWN where the type and cards went up, and that is the
    // other half of the same defect: the gaps were larger than v1's while the
    // content was half its size, so the screen read as a small UI floating in
    // empty space rather than as a dense one.
    readonly property real spaceXS: Math.round(tokens.gridUnit * 0.15)
    readonly property real spaceS: Math.round(tokens.gridUnit * 0.30)
    readonly property real spaceM: Math.round(tokens.gridUnit * 0.44)
    readonly property real spaceL: Math.round(tokens.gridUnit * 0.59)
    readonly property real spaceXL: Math.round(tokens.gridUnit * 0.89)

    // v1's cardRadius. 24 at 4K.
    readonly property real radius: Math.round(tokens.gridUnit * 0.44)

    // ---- components -------------------------------------------------------

    // v1's cardWidth/cardHeight. 600x480 at 4K, against the 440x260 that shipped
    // — the single biggest contributor to the shrunken home screen.
    readonly property real cardWidth: Math.round(tokens.gridUnit * 11.11)
    readonly property real cardHeight: Math.round(tokens.gridUnit * 8.89)
    // Unchanged in size (720 at 4K), only re-expressed in gridUnit. The drawer
    // was never measured against a television, so this is not a place to invent
    // a correction.
    readonly property real drawerWidth: Math.round(tokens.gridUnit * 13.33)

    // One duration for every transition in the shell. Slower than a desktop's
    // because the eye is further away and there is no cursor to follow.
    readonly property int durationFast: 120
    readonly property int durationBase: 220
}
