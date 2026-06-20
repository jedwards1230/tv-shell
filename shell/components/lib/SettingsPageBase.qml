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
//   }
FocusScope {
    id: page

    // Bottom-of-page hint text. Empty string hides the HintBar.
    property string hintText: ""

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
}
