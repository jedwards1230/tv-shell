import QtQuick
import QtQuick.Layouts
import "../../components"

// Shared "segment-pill header" for the home widgets (#249) — the RowLayout that
// Plex, Moonlight (Steam) and Apps all need: a FilterChips strip (present filter
// segments + a trailing `action:true` chip such as "Open Plex" / "Open Steam" /
// "Open Library"), a fillWidth spacer, and an OPTIONAL right-justified status slot
// (Moonlight's non-focusable SessionIndicator). It was hand-rolled identically in
// PlexWidget and SteamLibraryView; this extracts the one implementation.
//
// It is a FocusScope wrapping the RowLayout so it slots into the host focus chain
// EXACTLY where the bare FilterChips used to: it exposes the same duck-typed
// contract (visible / regionFocused / canFocus / focusFirstChild + previousRow/
// nextRow) and delegates all of it to the inner chips. forceActiveFocus() on the
// header lands on the chips (they carry `focus: true` inside this scope), matching
// how a sibling NavigableRow's up/down nav used to focus the chips directly.
//
// NOTE: the trailing slot is a `Component` (a Loader instantiates it), NOT a
// default-property alias — a default alias on this FocusScope would also swallow
// the header RowLayout declared below. A Component also lets the caller's bindings
// (e.g. `inSession: root.streaming`) resolve in the caller's scope.
FocusScope {
    id: root

    // Present filter segments, [{label, value}] — drive the crimson selected fill.
    property var segments: []
    // The active segment's value (matched WITHIN `segments`, never the actions).
    property string currentValue: ""
    // Trailing action chips, [{label, value}] — rendered as `action:true` ember
    // chips (a button, never the selected segment).
    property var actions: []

    // Forwarded to the inner FilterChips for the vertical focus chain.
    property Item previousRow: null
    property Item nextRow: null

    // Optional right-justified status slot (Moonlight's SessionIndicator). Plex/Apps
    // leave it null. A Component so the caller's bindings resolve in ITS scope and
    // the Loader owns the item's lifecycle + Layout positioning.
    property Component trailing: null

    // Chips committed a filter / fired an action; bubble both up to the widget.
    signal segmentChanged(var value)
    signal actionTriggered(var value)
    signal escaped
    // Emitted (deferred) when the chips gain focus, passing the HEADER root so the
    // host scrolls the whole header into view — matches the per-widget pattern.
    signal ensureVisibleRequested(var item)

    // Combined chip model = present segments + the action chips flagged action:true.
    // Mirrors the inline `_chipOptions` the widgets used to build by hand.
    readonly property var _chipOptions: {
        let o = root.segments.slice();
        for (var i = 0; i < root.actions.length; i++) {
            let a = root.actions[i];
            o.push({
                "label": a.label,
                "value": a.value,
                "action": true
            });
        }
        return o;
    }

    // === Home-tile focus contract (delegates to the inner chips) ===
    // The chips carry the focus; the header is focused iff they are.
    readonly property bool regionFocused: chips.activeFocus
    // FilterChips exposes no canFocus (the chain falls back to `visible` for it),
    // so the header is focusable exactly when it is shown — the parent owns the
    // `visible:` binding (segments-present), same as the old RowLayout.
    readonly property bool canFocus: visible

    function focusFirstChild() {
        if (!visible)
            return false;
        return chips.focusFirstChild();
    }

    implicitWidth: headerRow.implicitWidth
    implicitHeight: headerRow.implicitHeight

    RowLayout {
        id: headerRow
        anchors.fill: parent
        spacing: Units.spacingXL

        FilterChips {
            id: chips
            // Carry focus within this FocusScope so forceActiveFocus() on the
            // header (sibling-row up/down nav) lands here, like the bare chips did.
            focus: true
            Layout.alignment: Qt.AlignVCenter
            options: root._chipOptions
            // Selected FILTER index = currentValue's position within `segments`
            // ONLY — NOT the combined list, so an action chip never reads as the
            // selected segment. Mirrors the for-loop the widgets had inline.
            currentIndex: {
                for (var i = 0; i < root.segments.length; i++) {
                    if (root.segments[i].value === root.currentValue)
                        return i;
                }
                return 0;
            }
            previousRow: root.previousRow
            nextRow: root.nextRow
            onFilterChanged: value => root.segmentChanged(value)
            onActionTriggered: value => root.actionTriggered(value)
            onEscaped: root.escaped()
            // Defer so the Flickable geometry is settled (the widget may have been
            // hidden — service down — and just re-revealed) before we scroll to it.
            onActiveFocusChanged: if (activeFocus)
                Qt.callLater(() => root.ensureVisibleRequested(root))
        }

        Item {
            Layout.fillWidth: true
        }

        // Right-justified status slot — only instantiated when a caller supplies a
        // trailing Component (Moonlight's SessionIndicator). Non-focusable.
        Loader {
            Layout.alignment: Qt.AlignVCenter
            active: root.trailing !== null
            sourceComponent: root.trailing
        }
    }
}
