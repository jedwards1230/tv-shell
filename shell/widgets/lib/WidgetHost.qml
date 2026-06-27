import QtQuick
import QtQuick.Layouts

// Home-widget host (#249 Phase 2). Instantiates the WidgetRegistry set in order
// and builds the generic vertical focus chain that replaces HomeScreen's former
// hand-wired previousRow/nextRow web. Drop-in for the column of standardized home
// widgets: HomeScreen places one WidgetHost between the hero row and the All Apps
// entry, wires the two terminal anchors (the QuickActions row above, the All Apps
// entry below), and attaches each widget's behaviour via `widgetById(id)`.
//
// Focus chain (replicating the former static wiring, generically): walk the
// instantiated widgets in registry order; for each, its UP neighbour is the
// nearest preceding FOCUSABLE widget's exit point (its `lastRow`, or the widget
// itself if single-stop), falling back to `topAnchor`; its DOWN neighbour is the
// nearest following focusable widget's entry point (its `firstRow`, or itself),
// falling back to `bottomAnchor`. The neighbours are reactive bindings, so they
// re-resolve when a widget's `canFocus` / `firstRow` / `lastRow` changes (e.g.
// Plex health flips, Moonlight size flips) — exactly what the old per-site
// `canFocus ? …firstRow : …` ternaries did, but for any widget set.
ColumnLayout {
    id: host

    // Inter-widget spacing matches HomeScreen's contentColumn so the gaps between
    // hero ▸ widgets ▸ All Apps stay identical to the pre-refactor layout.
    spacing: 24

    // Terminal focus anchors — HomeScreen-owned items OUTSIDE the widget set.
    // topAnchor = the QuickActions row (UP from the first widget lands here);
    // bottomAnchor = the All Apps entry (DOWN from the last widget lands here).
    property Item topAnchor: null
    property Item bottomAnchor: null

    // Loaded widget items in registry order (null entries until each Loader
    // resolves — Loaders are synchronous, so this fills during construction).
    property var regions: []
    // id → loaded item map (reassigned on (un)load so reads stay reactive).
    property var itemsById: ({})

    // True while any hosted widget region holds focus — HomeScreen's re-anchor
    // net reads this (alongside the QuickActions row / popover) to decide whether
    // focus has fallen off the home content and needs re-seating.
    readonly property bool regionFocused: {
        for (var i = 0; i < host.regions.length; i++) {
            if (host.regions[i] && host.regions[i].regionFocused)
                return true;
        }
        return false;
    }

    function widgetById(id) {
        return host.itemsById[id] || null;
    }

    // Focus the first focusable widget in order; returns false if none could take
    // focus (HomeScreen then falls back to its own All Apps entry so focus never
    // strands). Mirrors the old _focusFirstVisibleRow walk over the widget set.
    function focusFirstVisible() {
        for (var i = 0; i < host.regions.length; i++) {
            var w = host.regions[i];
            if (w && w.focusFirstChild())
                return true;
        }
        return false;
    }

    // === Chain resolvers (read live region state; used inside Qt.bindings) ===
    function _isFocusable(w) {
        return w && w.canFocus === true;
    }
    // Entry point arriving going DOWN: the widget's firstRow if it exposes one,
    // else the widget itself (single-stop widgets are targeted directly).
    function _entry(w) {
        if (w.firstRow !== undefined && w.firstRow)
            return w.firstRow;
        return w;
    }
    // Exit point arriving going UP: the widget's lastRow if present, else itself.
    function _exit(w) {
        if (w.lastRow !== undefined && w.lastRow)
            return w.lastRow;
        return w;
    }
    function _prevTargetFor(index) {
        for (var j = index - 1; j >= 0; j--) {
            var w = host.regions[j];
            if (host._isFocusable(w))
                return host._exit(w);
        }
        return host.topAnchor;
    }
    function _nextTargetFor(index) {
        var n = host.regions.length;
        for (var j = index + 1; j < n; j++) {
            var w = host.regions[j];
            if (host._isFocusable(w))
                return host._entry(w);
        }
        return host.bottomAnchor;
    }
    // Exit of the last focusable widget (or topAnchor if none) — the All Apps
    // entry's UP neighbour, so B/Up from the bottom lands on the lowest content.
    function _lastFocusableExit() {
        for (var j = host.regions.length - 1; j >= 0; j--) {
            var w = host.regions[j];
            if (host._isFocusable(w))
                return host._exit(w);
        }
        return host.topAnchor;
    }

    // Rebuild the ordered region list + id map from the Repeater's loaded items.
    function _refresh() {
        var arr = [];
        var map = {};
        for (var i = 0; i < rep.count; i++) {
            var ld = rep.itemAt(i);
            var it = (ld && ld.item) ? ld.item : null;
            arr.push(it);
            if (it && WidgetRegistry.widgets[i])
                map[WidgetRegistry.widgets[i].widgetId] = it;
        }
        host.regions = arr;
        host.itemsById = map;
    }

    // Keep the All Apps entry's UP neighbour bound to the last focusable widget.
    onBottomAnchorChanged: {
        if (host.bottomAnchor)
            host.bottomAnchor.previousRow = Qt.binding(() => host._lastFocusableExit());
    }

    Repeater {
        id: rep
        model: WidgetRegistry.widgets

        delegate: Loader {
            id: wLoader
            required property int index
            required property var modelData

            Layout.fillWidth: true
            sourceComponent: modelData.component
            // Collapse with the widget — an invisible Loader is skipped by the
            // ColumnLayout (no phantom spacing around a hidden widget). Read the
            // widget's INTENDED visibility (`wantVisible`), not its effective
            // `visible`: the latter includes this Loader's own visibility, so
            // binding to it would feed back and latch every widget off.
            visible: item ? item.wantVisible : false

            onLoaded: {
                var idx = wLoader.index;
                item.widgetEnabled = Qt.binding(() => wLoader.modelData.enabled);
                item.size = Qt.binding(() => wLoader.modelData.size);
                item.previousRow = Qt.binding(() => host._prevTargetFor(idx));
                item.nextRow = Qt.binding(() => host._nextTargetFor(idx));
                host._refresh();
            }
        }
    }

    Component.onCompleted: host._refresh()
}
