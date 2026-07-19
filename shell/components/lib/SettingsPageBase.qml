import QtQuick
import QtQuick.Layouts
import "../"

// Standard scaffold for a settings page. Encapsulates the boilerplate every
// page repeats: a FocusScope sized to its content plus the page padding, a
// padded content column at the standard section spacing, a trailing flex
// spacer, and a bottom HintBar driven by `hintText`.
//
// Page content is declared as direct children — they land in the content column
// via the default property. The spacer and HintBar are appended after content
// (they live in an outer column, so child ordering stays correct). Pages still
// own their `focusFirst()` and the KeyNavigation chain between their controls.
//
//   SettingsPageBase {
//       id: root
//       hintText: "A: toggle"
//       function focusFirst() { firstControl.forceActiveFocus(); }
//       SectionHeader { text: "..." }
//       FocusButton { id: firstControl; ... }
//
//       // Anchor-filling overlay (e.g. a confirm dialog) — NOT column-flow:
//       overlay: ConfirmDialog { opened: root.confirmAction !== "" }
//   }
//
// Non-visual children (Process / SocketClient / Timer) can be declared in the
// `overlay` slot too — they don't belong in the content column's layout flow.
FocusScope {
    id: page

    // Bottom-of-page hint text. Empty string hides the HintBar.
    property string hintText: ""

    // Overlay / non-flow slot: children here anchor over the whole page (above
    // the content column) instead of joining the content ColumnLayout. Use for
    // anchor-filling overlays (ConfirmDialog) and non-visual helpers
    // (Process/SocketClient/Timer) that must NOT take a column slot.
    property alias overlay: overlayHost.data

    // === Settings-page contract ===
    // SettingsApp drives every loaded page through these two entry points
    // (contentLoader.item.focusFirst() on Return-to-enter; applyDeepTarget() when
    // a deep-link slug is pending). Declaring them on the base means a page that
    // extends SettingsPageBase always satisfies the contract — a page that forgets
    // to override focusFirst() inherits a safe no-op instead of SettingsApp's
    // guarded `&& item.focusFirst` silently skipping focus-entry.
    //
    // Override in the page:
    //   function focusFirst() { firstControl.forceActiveFocus(); }
    //   function applyDeepTarget(t) { ... }   // only pages with deep-links
    function focusFirst() {
    }

    // Apply a pending deep-link target (e.g. "moonlight"). Default no-op — only
    // pages that host a deep-linkable sub-surface (Widgets ▸ Moonlight) override.
    function applyDeepTarget(target) {
    }

    // Page → app back request. A page whose only focusable control SWALLOWS
    // B/Escape (e.g. NavigableGrid accepts the event and merely emits `escaped`,
    // doing no back-nav itself) must re-emit this so SettingsApp can route it
    // through the unified _back() handler — otherwise focus is stranded on the
    // page. SettingsApp wires this via a Connections on the loaded page; pages
    // that let B/Escape bubble naturally never need to emit it.
    signal backRequested

    // Page content is appended into the content column.
    default property alias content: contentColumn.data

    implicitHeight: outerColumn.implicitHeight + 2 * Theme.padding

    ColumnLayout {
        id: outerColumn
        anchors.fill: parent
        anchors.margins: Theme.padding
        spacing: Units.spacingLG

        ColumnLayout {
            id: contentColumn
            Layout.fillWidth: true
            spacing: Units.spacingLG
            // Page content lands here via the default alias above.
        }

        // Flex spacer pushes the hint to the bottom of the page.
        Item {
            Layout.fillWidth: true
            Layout.fillHeight: true
        }

        HintBar {
            visible: page.hintText !== ""
            text: page.hintText
        }
    }

    // Overlay / non-flow host: anchor-fills the page above the content column.
    // Children assigned via the `overlay` alias land here (ConfirmDialog,
    // Process/SocketClient/Timer) instead of joining the content layout.
    Item {
        id: overlayHost
        anchors.fill: parent
    }
}
