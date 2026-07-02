.pragma library

// Shared vertical focus-chain traversal for the home-tile focus contract.
//
// Every focusable home region — the NavigableRow / NavigableGrid rails, the
// WakeCard stand-in, and the Widget base itself — exposes `previousRow` / `nextRow`
// neighbour links wired by the host (WidgetHost / HomeScreen / LibraryScreen). Up
// and Down walk that chain to the nearest FOCUSABLE neighbour, skipping hidden /
// !canFocus regions. This walk was historically copy-pasted (and, in WakeCard,
// subtly DIVERGED — it checked only `visible`, wrongly focusing a visible-but-
// !canFocus region others skip). It now lives here once so every consumer steps
// identically.
//
// The consumers are two different QML types (flat `components/` regions and the
// `widgets/lib/Widget` base) that can't share a QML supertype, so the shared
// implementation is a pure `.pragma library` helper taking the walking item as an
// argument (mirrors widgetConfig.js). Callers pass their root: `navigateUp(root)`.

// A neighbour is focusable when its contract `canFocus` is true. Targets that
// predate the contract (or non-region rows like the QuickActions strip) expose no
// `canFocus`, so fall back to plain `visible`.
function focusable(item) {
    if (!item)
        return false;
    return item.canFocus !== undefined ? item.canFocus : item.visible;
}

// Walk `startItem.previousRow` upward to the first focusable neighbour and focus
// it. Returns true if one was focused, false if the chain held none (a no-op).
function navigateUp(startItem) {
    var target = startItem ? startItem.previousRow : null;
    while (target) {
        if (focusable(target)) {
            target.forceActiveFocus();
            return true;
        }
        target = (target.previousRow !== undefined) ? target.previousRow : null;
    }
    return false;
}

// Walk `startItem.nextRow` downward to the first focusable neighbour and focus it.
// Returns true if one was focused, false if the chain held none (a no-op).
function navigateDown(startItem) {
    var target = startItem ? startItem.nextRow : null;
    while (target) {
        if (focusable(target)) {
            target.forceActiveFocus();
            return true;
        }
        target = (target.nextRow !== undefined) ? target.nextRow : null;
    }
    return false;
}
