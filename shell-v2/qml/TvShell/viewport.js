// Two viewport decisions, pure: how far to scroll a rail, and how big
// everything is.
//
// v1 put both of these in bindings. The scroll one became `ensureVisibleRequested`,
// a member of the eight-part duck-typed focus protocol that every widget had to
// remember to emit; the size one became `screenScale.js` after a zero-height
// screen collapsed the whole UI. Both are arithmetic, so both belong here where
// a test can reach them without a window.
//
// THE RULES — scrolling
//
//   V1  An item already fully visible, margin included, does not move the view.
//       Scrolling on every focus move is the single most common way a 10-foot UI
//       feels unsteady.
//   V2  An item past the trailing edge scrolls the MINIMUM that brings its
//       trailing edge plus the margin into view — it is not centred. Centring
//       moves the whole rail on every step and loses the sense of place.
//   V3  Symmetrically for the leading edge.
//   V4  The result is clamped to [0, max(0, contentWidth - viewportWidth)].
//       Content smaller than the viewport therefore always yields 0: a rail with
//       two cards cannot be scrolled into empty space.
//   V5  An item WIDER than the viewport aligns to its leading edge. Applying V2
//       and V3 to it would satisfy neither and oscillate.
//
// THE RULES — scale
//
//   V6  scale = height / 2160. Every size in Tokens is written for 4K, which is
//       what the couch runs, so the reference is 1.0 there and nothing needs a
//       second set of numbers.
//   V7  A non-finite or non-positive height yields 1, not 0. A scale of 0 makes
//       every element zero-sized — a black screen with no error, which is the
//       worst possible failure and so the one pinned by a test.
//   V8  Clamped to [0.5, 2]. Beyond that the 10-foot type sizes stop being
//       10-foot type sizes in either direction.
//
// tests/qml/tst_viewport.qml asserts each of these.
.pragma library

var REFERENCE_HEIGHT = 2160;
var MIN_SCALE = 0.5;
var MAX_SCALE = 2;

// The content offset that brings [itemX, itemX+itemWidth] into view, given the
// offset the view currently has. Returns `offset` unchanged when nothing needs
// to move (V1).
function scrollOffset(offset, itemX, itemWidth, viewportWidth, contentWidth, margin) {
    var maxOffset = Math.max(0, contentWidth - viewportWidth);
    var clamp = function (v) {
        return Math.max(0, Math.min(maxOffset, v)); // V4
    };
    if (!isFinite(offset) || !isFinite(itemX) || !isFinite(itemWidth) || viewportWidth <= 0)
        return clamp(isFinite(offset) ? offset : 0);

    var m = isFinite(margin) ? Math.max(0, margin) : 0;

    // V5 first: for an oversized item the two edge rules contradict each other,
    // so neither is applied and the leading edge wins.
    if (itemWidth + 2 * m > viewportWidth)
        return clamp(itemX - m);

    var leading = itemX - m;
    var trailing = itemX + itemWidth + m;

    if (leading < offset)
        return clamp(leading); // V3
    if (trailing > offset + viewportWidth)
        return clamp(trailing - viewportWidth); // V2
    return clamp(offset); // V1
}

// The UI scale for a viewport of `height` pixels.
function scaleFor(height) {
    if (typeof height !== "number" || !isFinite(height) || height <= 0)
        return 1; // V7
    var s = height / REFERENCE_HEIGHT; // V6
    return Math.max(MIN_SCALE, Math.min(MAX_SCALE, s)); // V8
}
