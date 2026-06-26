import QtQuick

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
    // canFocus lets the vertical-chain walk skip the widget when it can't take
    // focus (NavigableRow._focusable falls back to `visible` when absent).
    readonly property bool regionFocused: activeFocus
    property bool canFocus: visible

    // Bubbles B/Escape up to HomeScreen's focus reset.
    signal escaped

    // Default single-stop focus entry; multi-row widgets override to target a
    // specific internal sub-row.
    function focusFirstChild() {
        if (!visible || !canFocus)
            return false;
        forceActiveFocus();
        return true;
    }
}
