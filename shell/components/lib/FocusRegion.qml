import QtQuick

// Typed base for a home-screen focusable region.
//
// HomeScreen drives every focusable region from one ordered _contentRegions()
// list (see docs/INPUT_AND_STATE.md §3 "Home-tile focus contract"). Historically
// the contract was duck-typed — each region (NavigableRow / MoonlightWidget /
// PlexWidget) hand-rolled the same members, and a region that forgot one became a
// silent focus dead-end (e.g. the degraded-Plex dead stick, #248). This base
// makes the contract a *typed* surface: a region that extends FocusRegion
// inherits safe defaults for every member, so a missing override is a quiet,
// inspectable no-op (focusFirstChild()===false → the region is skipped) instead
// of a `... is not a function` load error.
//
// The 8 home-contract members:
//   1. visible          — inherited from Item (a region is skipped when hidden)
//   2. previousRow      — neighbour above in the vertical chain
//   3. nextRow          — neighbour below in the vertical chain
//   4. escaped()        — emitted on B/Escape (the host re-anchors / steps back)
//   5. regionFocused    — does this region currently hold focus?
//   6. canFocus         — does this region have something focusable RIGHT NOW?
//   7. focusFirstChild()— focus the first selectable child; false if it could not
//   8. _navigateUp/_navigateDown — walk the chain to the nearest focusable
//      neighbour (shared so every region steps identically)
//
// Subclasses override regionFocused / canFocus / focusFirstChild() (and usually
// emit escaped() from their own Keys handlers); previousRow/nextRow and the
// chain-walk helpers come for free.
FocusScope {
    id: region

    // Vertical-chain neighbours, wired by the host (HomeScreen/LibraryScreen).
    property Item previousRow: null
    property Item nextRow: null

    // Emitted on B/Escape from within the region.
    signal escaped

    // Does this region currently hold focus? Default tracks plain activeFocus;
    // multi-sub-row regions (Plex) override to OR their inner rows together.
    readonly property bool regionFocused: region.activeFocus

    // Does this region have something focusable right now? A visible-but-empty
    // region (no cards, degraded service) must report false so the chain walk
    // skips it. Default is conservative: false until a subclass declares it can.
    readonly property bool canFocus: false

    // Focus the first selectable child; return false if it could not (hidden or
    // empty). The base default is a no-op that returns false, so a region that
    // forgets to override is skipped rather than crashing the focus walk.
    function focusFirstChild() {
        return false;
    }

    // A neighbour is focusable when its contract `canFocus` is true. Targets that
    // predate the contract (or non-region rows like the QuickActions strip)
    // expose no canFocus, so fall back to plain `visible`.
    function _focusable(item) {
        return item.canFocus !== undefined ? item.canFocus : item.visible;
    }

    function _navigateUp() {
        var target = region.previousRow;
        while (target) {
            if (region._focusable(target)) {
                target.forceActiveFocus();
                return;
            }
            target = (target.previousRow !== undefined) ? target.previousRow : null;
        }
    }

    function _navigateDown() {
        var target = region.nextRow;
        while (target) {
            if (region._focusable(target)) {
                target.forceActiveFocus();
                return;
            }
            target = (target.nextRow !== undefined) ? target.nextRow : null;
        }
    }
}
