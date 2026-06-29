import QtQuick
import QtQuick.Layouts
import "../lib"
import "../../components"

// Home-screen Apps widget (#249) — the evolution of the old Recent widget. A
// SegmentedHeader flips between two segments fed into ONE horizontal rail:
//   "recent" → the merged running+recents model (HomeScreen-owned, passed in via
//              `model`), exactly what the Recent widget rendered.
//   "all"    → every installed app, read straight off the AppDiscoveryManager
//              singleton (the same source the Library surface uses).
// Both segments render in the SAME single-scroll horizontal rail (the "All Apps"
// pill stays bound to the rail, it does not expand inline) — the full vertical
// browse GRID lives in the fullscreen Library surface, one step away behind the
// "Open Library" action chip (and the standalone All Apps entry below the widget).
//
// The widget id stays "recent" (config namespace + registry key) so this is NOT a
// settings migration — only the DISPLAY name became "Apps". Both segments emit the
// same outward signals the Recent widget had (entryActivated / entryContextRequested
// / ensureVisibleRequested / escaped) so HomeScreen's launch/focus/PopoverMenu
// wiring is unchanged; the new openLibraryRequested is the one addition.
//
// Extends Widget (the home-screen base): a FocusScope hosting the header + grid,
// satisfying the duck-typed focus contract by delegating to them.
Widget {
    id: root

    // Merged recent/running model (HomeScreen-owned), and the small-size reflow
    // flag (small = icon-only square tiles; medium = full icon + label cards).
    property var model: []
    property bool recentSmall: false

    // Bubbled up so HomeScreen keeps the launch/focus + PopoverMenu logic. An
    // all-apps entry is just {name, exec, icon, comment, running:false}, which
    // HomeScreen's _recentActivate/_recentContext already treat as a launch.
    signal entryActivated(var entry)
    signal entryContextRequested(var entry, var card)
    signal ensureVisibleRequested(var item)
    // Trailing "Open Library" action chip → HomeScreen opens the Library surface.
    signal openLibraryRequested

    // === Segments ===
    property string _segment: "recent"

    // All installed apps, alphabetised, shaped exactly like AppCard expects with an
    // explicit running:false (so the shared _recentActivate path launches them).
    readonly property var _allApps: {
        var apps = (AppDiscoveryManager.applications || []).slice();
        apps.sort(function (a, b) {
            var an = (a.name || "").toLowerCase();
            var bn = (b.name || "").toLowerCase();
            return an < bn ? -1 : (an > bn ? 1 : 0);
        });
        var out = [];
        for (var i = 0; i < apps.length; i++) {
            var a = apps[i];
            out.push({
                "name": a.name || "",
                "exec": a.exec || "",
                "icon": a.icon || "",
                "comment": a.comment || "",
                "wmClass": a.wmClass || "",
                "running": false
            });
        }
        return out;
    }

    readonly property bool _hasRecent: root.model.length > 0
    readonly property bool _hasAll: root._allApps.length > 0

    // Present segments: Recent only when it has content; All Apps only when apps
    // exist (essentially always). Mirrors Plex/Steam's dynamic segment list.
    readonly property var _segmentOptions: {
        var o = [];
        if (_hasRecent)
            o.push({
                "label": "Recent",
                "value": "recent"
            });
        if (_hasAll)
            o.push({
                "label": "All Apps",
                "value": "all"
            });
        return o;
    }

    readonly property var _activeModel: root._segment === "all" ? root._allApps : root.model

    // Trailing "Open Library" ACTION chip sentinel (ignored by the segment handler).
    readonly property string _openValue: "__open_library__"

    // Surfaced for HomeScreen's hint bar (current rail selection).
    readonly property int currentIndex: appsRow.currentIndex

    // Apps essentially always exist, so this widget basically always shows — that's
    // intended (it is the home screen's app launcher).
    wantVisible: root.widgetEnabled && (root._hasRecent || root._hasAll)

    implicitWidth: col.implicitWidth
    implicitHeight: root.wantVisible ? col.implicitHeight : 0

    // === Home-tile focus contract ===
    firstRow: segmentHeader
    lastRow: appsRow
    canFocus: visible && (root._hasRecent || root._hasAll)

    function focusFirstChild() {
        if (!root.canFocus)
            return false;
        // Prefer the rail when the active segment has content; otherwise focus the
        // header (e.g. the active segment is empty but the other still has apps, so
        // the user can flip segments). Mirrors PlexWidget's firstRow-or-fallback.
        if (appsRow.canFocus)
            return appsRow.focusFirstChild();
        if (segmentHeader.visible)
            return segmentHeader.focusFirstChild();
        return false;
    }

    // Keep the active segment on something that has content (a flip of either input
    // can empty the current segment). Driven off the two data sources.
    function _coerceSegment() {
        if (root._segment === "recent" && !root._hasRecent && root._hasAll)
            root._segment = "all";
        else if (root._segment === "all" && !root._hasAll && root._hasRecent)
            root._segment = "recent";
    }
    onModelChanged: root._coerceSegment()
    Connections {
        target: AppDiscoveryManager
        function onApplicationsChanged() {
            root._coerceSegment();
        }
    }

    ColumnLayout {
        id: col
        width: root.width
        spacing: Units.spacingMD

        // === Header: Recent / All Apps segments + "Open Library" action chip ===
        SegmentedHeader {
            id: segmentHeader
            Layout.fillWidth: true
            visible: root._hasRecent || root._hasAll
            segments: root._segmentOptions
            currentValue: root._segment
            actions: [
                {
                    "label": "Open Library",
                    "value": root._openValue
                }
            ]
            previousRow: root.previousRow
            nextRow: appsRow
            onSegmentChanged: value => root._segment = value
            onActionTriggered: value => root.openLibraryRequested()
            onEscaped: root.escaped()
            onEnsureVisibleRequested: item => root.ensureVisibleRequested(item)
        }

        // === The one horizontal rail (shows the active segment) ===
        // Both the Recent and All Apps segments render here, in this single
        // horizontal single-scroll rail — exactly like the old Recent widget's row.
        // The vertical browse grid of every app lives in the Library surface, not
        // here; this stays a glance rail.
        NavigableRow {
            id: appsRow
            visible: root._activeModel.length > 0
            Layout.fillWidth: true
            // Extra breathing room between the chip strip and the rail (on top of
            // the ColumnLayout spacing) so the pills don't crowd the row below.
            Layout.topMargin: Units.spacingMD
            Layout.preferredHeight: Theme.rowHeight
            keyNavigationWraps: true
            previousRow: segmentHeader
            nextRow: root.nextRow
            model: root._activeModel
            onActiveFocusChanged: if (activeFocus)
                root.ensureVisibleRequested(appsRow)

            delegate: AppCard {
                required property int index
                required property var modelData
                iconOnly: root.recentSmall
                width: root.recentSmall ? Theme.cardHeight : Theme.cardWidth
                height: Theme.cardHeight
                app: modelData
                running: modelData.running === true
                focus: index === appsRow.currentIndex
                onActivated: {
                    // Sync the cursor to a clicked card (mouse mode) so a later
                    // controller move resumes from here, then bubble the launch up.
                    appsRow.currentIndex = index;
                    root.entryActivated(modelData);
                }
            }

            onContextRequested: {
                if (currentItem && currentIndex >= 0 && currentIndex < root._activeModel.length)
                    root.entryContextRequested(root._activeModel[currentIndex], currentItem);
            }
            onEscaped: root.escaped()
        }
    }
}
