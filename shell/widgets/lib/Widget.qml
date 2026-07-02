import QtQuick
import "../../components/lib/focusChain.js" as FocusChain

// Base type for home-screen widgets (Now Playing, Moonlight, Plex, Recent, …).
// It bakes in the duck-typed focus/visibility contract that HomeScreen and
// NavigableRow query, so any widget extending it satisfies the contract with
// sensible defaults and overrides only what it needs.
//
// Two tiers (see docs/INPUT_AND_STATE.md):
//   Tier 1 (all regions): focusFirstChild() + regionFocused — HomeScreen's
//     focus walk and re-anchor net read these.
//   Tier 2 (vertical chain walking, NavigableRow): previousRow / nextRow /
//     canFocus — multi-row widgets additionally set firstRow / lastRow so the
//     chain targets the correct internal sub-row.
// widgetEnabled, size, and the escaped signal are widget-level concerns the
// host wires in (Settings ▸ Widgets toggle, sized widgets, B/Escape bubbling).
//
// Vertical Up/Down traversal (_navigateUp/_navigateDown) is inherited from the
// shared focusChain.js helper, so a single-stop widget steps its previousRow/
// nextRow chain correctly with zero widget-local nav code. ensureVisibleRequested
// is declared here ONCE (not re-declared per widget) and auto-wired to the host by
// WidgetHost; for single-stop widgets the base also auto-emits it on focus entry
// (see onActiveFocusChanged) so a leaf never has to remember either the signal or
// the emit.
FocusScope {
    id: root

    // Vertical focus chain neighbours (set by the host). Either may be null.
    property Item previousRow: null
    property Item nextRow: null

    // Multi-row widgets expose these so the chain can target the correct
    // internal sub-row; single-stop widgets leave them null.
    property Item firstRow: null
    property Item lastRow: null

    // Home-screen widget toggle (Settings ▸ Widgets). When false the widget is
    // typically hidden and collapses to zero height.
    property bool widgetEnabled: true

    // Optional render size (e.g. "small"/"medium"); only sized widgets use it.
    property string size: ""

    // The widget's INTENDED visibility — a plain bool computed from its own state
    // (widgetEnabled + content), overridden by sized/health-aware widgets.
    // `visible` is bound to it so a subclass sets `wantVisible:` rather than
    // `visible:`. WidgetHost reads `wantVisible` (not `visible`) to collapse a
    // hidden widget's layout slot: reading the effective `visible` from the host's
    // Loader would feed back through the parent chain and latch every widget off.
    property bool wantVisible: true
    visible: wantVisible

    // regionFocused lets HomeScreen's re-anchor net recognise this region;
    // canFocus lets the shared vertical-chain walk (focusChain.js) skip the widget
    // when it can't take focus (the walk falls back to `visible` when absent).
    readonly property bool regionFocused: activeFocus
    property bool canFocus: visible

    // Bubbles B/Escape up to HomeScreen's focus reset.
    signal escaped

    // Asks the host's scroll view to bring `item` into view. Declared on the base
    // so every widget inherits it (a leaf that forgets it would be silently
    // unscrollable-to); WidgetHost forwards it to HomeScreen. Multi-row widgets
    // emit it from their internal rows on focus; single-stop widgets get the base
    // auto-emit below.
    signal ensureVisibleRequested(var item)

    // Single-stop widgets (firstRow unset) have no internal row to emit
    // ensureVisibleRequested on focus entry, so the base emits for them. Multi-row
    // widgets set firstRow to an internal region that emits on ITS own focus, so
    // the base stays silent here to avoid a double-scroll on entry.
    onActiveFocusChanged: {
        if (activeFocus && (root.firstRow === null || root.firstRow === root))
            root.ensureVisibleRequested(root);
    }

    // Vertical chain walk, inherited by every widget (shared focusChain.js). Up/
    // Down step previousRow/nextRow to the nearest focusable neighbour, skipping
    // hidden / !canFocus regions. Return true when one was focused.
    function _navigateUp() {
        return FocusChain.navigateUp(root);
    }
    function _navigateDown() {
        return FocusChain.navigateDown(root);
    }

    // Default single-stop focus entry; multi-row widgets override to target a
    // specific internal sub-row.
    function focusFirstChild() {
        if (!visible || !canFocus)
            return false;
        forceActiveFocus();
        return true;
    }
}
