// The design tokens, against v1's calibrated reference.
//
// WHY THIS LANE EXISTS
//
// The first version of Tokens.qml used fresh 4K constants written by eye. They
// came out well under v1's — the clock 80%, card titles 85%, and card HEIGHT
// 54%, a card of 40% the area — and on the television the home screen occupied
// roughly the top-left fifth of a 4K panel. Nothing was scaled wrong and no
// window was mis-sized; the content was simply that small, against a repo rule
// that says in as many words "10-foot UI at 4K … Don't shrink them".
//
// A number typed by eye reads as reasonable in an editor and is only wrong on a
// television, which is the worst possible place to find out. So the numbers are
// pinned here, at the one resolution the couch actually runs, against the values
// v1 produces from the same `gridUnit` ratios for the same panel.
//
// If a future change wants different sizes that is entirely legitimate — but it
// has to change this file too, which makes it a decision rather than a drift.
import QtQuick
import QtTest
import TvShell
import "qrc:/qt/qml/TvShell/viewport.js" as Viewport

Item {
    id: harness

    TestCase {
        name: "Tokens"

        // v1: Units.gridUnit = max(8, round(screenHeight / 40)) -> 54 at 2160.
        readonly property int v1GridUnit: 54

        function init() {
            // The couch resolution. Main.qml is the only writer in the shell; a
            // test is the other legitimate one.
            Tokens.scale = 1.0;
        }

        function test_gridUnit_matches_v1_at_4k() {
            compare(Tokens.gridUnit, v1GridUnit, "gridUnit must equal v1's Units.gridUnit at 4K, or every ratio below drifts");
        }

        // The type scale, as v1's ratios resolve at 4K.
        function test_type_matches_v1() {
            compare(Tokens.fontDisplay, 120); // v1 fontHero,    gridUnit * 2.22
            compare(Tokens.fontTitle, 56);    // v1 fontTitle,   gridUnit * 1.04
            compare(Tokens.fontBody, 40);     // v1 fontBody,    gridUnit * 0.74
            compare(Tokens.fontCaption, 28);  // v1 fontCaption, gridUnit * 0.52
        }

        // The card, which was the single biggest contributor: 440x260 shipped
        // against v1's 600x480.
        function test_card_matches_v1() {
            compare(Tokens.cardWidth, 600);
            compare(Tokens.cardHeight, 480);
            verify(Tokens.cardWidth * Tokens.cardHeight >= 280000);
        }

        function test_spacing_matches_v1() {
            compare(Tokens.spaceXS, 8);
            compare(Tokens.spaceS, 16);
            compare(Tokens.spaceM, 24);
            compare(Tokens.spaceL, 32);
            compare(Tokens.spaceXL, 48);
        }

        // The other half of the defect, and the one easiest to reintroduce: the
        // gaps were LARGER than v1's while the content was half the size, so the
        // screen read as a small UI floating in empty space. Stated as a
        // relationship rather than a number, because it is the relationship that
        // was wrong.
        function test_content_is_large_relative_to_the_gaps() {
            verify(Tokens.cardHeight > Tokens.spaceXL * 8, "cards have shrunk toward the size of the gaps around them");
            verify(Tokens.fontDisplay > Tokens.spaceXL * 2, "the clock has shrunk toward the size of the page margin");
        }

        // The focus ring is how the couch knows where it is. v1 floors its
        // thickest border at 3px for exactly this reason.
        function test_focus_ring_is_visible_from_the_couch() {
            verify(Tokens.focusRingWidth >= 3);
            compare(Tokens.focusRingWidth, 6); // gridUnit * 0.11 at 4K
        }

        // Everything is a ratio, so a smaller panel scales rather than clipping —
        // and the clamp in viewport.js keeps it from collapsing.
        function test_tokens_scale_with_the_panel() {
            Tokens.scale = 0.5; // 1080p
            compare(Tokens.gridUnit, 27);
            compare(Tokens.cardWidth, 300);
            compare(Tokens.fontDisplay, 60);
            Tokens.scale = 1.0;
        }

        // A degenerate screen height must still yield a usable unit — and this
        // goes through `Viewport.scaleFor`, the path the shell actually uses,
        // rather than poking `scale` directly.
        //
        // The distinction is the point. `gridUnit` carries v1's `max(8, …)`
        // floor, but `scaleFor` already clamps to [0.5, 2.0], so through the
        // real path the unit never falls below 27 and that floor is never
        // reached. A test that sets `scale = 0.01` proves the floor works while
        // proving nothing about any state the shell can produce; this one pins
        // the property that matters, which is that the clamp and the floor
        // agree about an unknown height instead of contradicting each other.
        function test_a_degenerate_height_still_yields_a_usable_unit() {
            const degenerate = [0, -1, NaN, Infinity, undefined, 1];
            for (let i = 0; i < degenerate.length; ++i) {
                Tokens.scale = Viewport.scaleFor(degenerate[i]);
                verify(Tokens.gridUnit >= 27, "height " + JSON.stringify(degenerate[i]) + " gave gridUnit " + Tokens.gridUnit);
                verify(Tokens.cardHeight > Tokens.fontCaption);
            }
            Tokens.scale = 1.0;
        }

        // The floor itself, for a caller that bypasses scaleFor and sets scale
        // directly. Unreachable through the shell — kept as defence, and
        // labelled so nobody mistakes it for a reachable state.
        function test_gridUnit_has_a_floor_for_direct_callers() {
            Tokens.scale = 0.01;
            verify(Tokens.gridUnit >= 8);
            Tokens.scale = 1.0;
        }
    }
}
