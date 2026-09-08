// viewport.js, headless. Pins V1..V8.
//
// Scrolling and scale are arithmetic, so they are answerable with no window —
// which is the whole reason they are not bindings. docs/V2_SHELL.md records
// which mutation of viewport.js each assertion caught.
import QtQuick
import QtTest
import "qrc:/qt/qml/TvShell/viewport.js" as Viewport

Item {
    id: harness

    // A rail 1000 wide over 3000 of content; cards 400 wide, margin 100.
    readonly property real viewportWidth: 1000
    readonly property real contentWidth: 3000
    readonly property real cardWidth: 400
    readonly property real edgeMargin: 100

    function offset(current, itemX) {
        return Viewport.scrollOffset(current, itemX, harness.cardWidth, harness.viewportWidth, harness.contentWidth, harness.edgeMargin);
    }

    TestCase {
        name: "Viewport"

        // --- V1: a visible item does not move the view ----------------------

        function test_a_fully_visible_item_does_not_scroll() {
            // At offset 0 the viewport shows [0,1000); an item at 200..600 sits
            // inside it with both margins.
            compare(harness.offset(0, 200), 0);
            // And the same partway down the rail.
            compare(harness.offset(1000, 1200), 1000);
        }

        function test_an_item_exactly_at_the_margin_does_not_scroll() {
            // Leading edge exactly at offset+margin, trailing exactly at
            // offset+viewport-margin. Both count as already visible, so a
            // >=/> mix-up shows up as a rail that twitches on every move.
            compare(harness.offset(0, 100), 0);
            compare(harness.offset(0, 500), 0);
        }

        // --- V2 / V3: minimum movement, not centring -------------------------

        function test_an_item_past_the_trailing_edge_scrolls_the_minimum() {
            // Item at 800..1200, margin 100 -> trailing 1300 must be visible, so
            // the offset becomes 1300-1000 = 300. Centring would give 700.
            compare(harness.offset(0, 800), 300);
        }

        function test_an_item_before_the_leading_edge_scrolls_the_minimum() {
            // Viewport at 1000..2000; item at 900..1300 with margin -> leading
            // edge 800, so the offset becomes 800.
            compare(harness.offset(1000, 900), 800);
        }

        // --- V4: clamped to the content --------------------------------------

        function test_the_offset_never_leaves_the_content() {
            // The last card: its trailing edge plus margin is past the content,
            // so the unclamped answer would scroll into empty space.
            compare(harness.offset(0, 2600), 2000);
            // And never negative, at either end.
            compare(harness.offset(0, 0), 0);
            compare(harness.offset(500, 0), 0);
        }

        function test_content_narrower_than_the_viewport_never_scrolls() {
            // Two cards in a rail wider than they are: there is nowhere to go,
            // so every answer is 0 regardless of which card is focused.
            compare(Viewport.scrollOffset(0, 0, 400, 1000, 800, 100), 0);
            compare(Viewport.scrollOffset(0, 400, 400, 1000, 800, 100), 0);
        }

        // --- V5: an oversized item aligns to its leading edge -----------------

        function test_an_item_wider_than_the_viewport_aligns_leading() {
            // A 1200-wide item in a 1000-wide viewport satisfies neither edge
            // rule; applying both in turn oscillates. The leading edge wins.
            compare(Viewport.scrollOffset(0, 1500, 1200, 1000, 3000, 100), 1400);
        }

        // --- V6..V8: scale ----------------------------------------------------

        function test_scale_is_one_at_the_4k_reference() {
            compare(Viewport.scaleFor(2160), 1);
        }

        function test_scale_tracks_height() {
            compare(Viewport.scaleFor(1080), 0.5);
            compare(Viewport.scaleFor(1440), 1440 / 2160);
        }

        // The failure this exists to prevent: a scale of 0 makes every size 0,
        // which is a black screen with no error message.
        function test_a_degenerate_height_is_one_and_never_zero() {
            const bad = [0, -1080, NaN, Infinity, null, undefined, "1080"];
            for (let i = 0; i < bad.length; ++i)
                compare(Viewport.scaleFor(bad[i]), 1, "height " + JSON.stringify(bad[i]));
        }

        function test_scale_is_clamped_at_both_ends() {
            compare(Viewport.scaleFor(1), 0.5);
            compare(Viewport.scaleFor(100000), 2);
        }
    }
}
