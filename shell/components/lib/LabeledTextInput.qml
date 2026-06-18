import QtQuick
import QtQuick.Layouts
import "../"

// Label + text input field row. Used in MoonlightSettings server form
// (Name, Host, App fields — 3× duplication).
//
// Root is a FocusScope (per CLAUDE.md: interactive lib/ components must be
// FocusScopes so SettingsApp's outer Flickable scroll-follow keeps working).
// The inner RowLayout fills the scope; Layout.fillWidth lets the component
// stretch to its parent ColumnLayout's width the way the old RowLayout root did.
//
// The TextInput id must be accessible at the call site for KeyNavigation
// wiring. Expose it via the `inputField` alias.
//
// Usage:
//   LabeledTextInput {
//       label: "Name"
//       text: root.newName
//       onTextChanged: text => root.newName = text
//   }
//   // Wire nav to the next field via the exposed inner TextInput:
//   LabeledTextInput { id: hostRow; ... }
//   // hostRow.inputField is the inner TextInput
FocusScope {
    id: root

    property string label: ""
    property string text: ""
    property alias inputField: input

    // Named `textEdited` (not `textChanged`) to avoid colliding with the
    // auto-generated change signal of the `text` property above.
    signal textEdited(string text)

    // Forwarded from the inner TextInput's Keys.onEscapePressed. Call sites bind
    // this declaratively — Keys.* handlers are read-only and cannot be assigned
    // imperatively on the aliased inner input from the outside.
    signal escapePressed

    Layout.fillWidth: true
    implicitWidth: row.implicitWidth
    implicitHeight: row.implicitHeight

    RowLayout {
        id: row
        anchors.fill: parent
        spacing: 24

        Text {
            text: root.label
            font.pixelSize: Theme.fontSmall
            color: Theme.textSecondary
            Layout.preferredWidth: 160
        }

        Rectangle {
            Layout.fillWidth: true
            Layout.preferredHeight: 80
            radius: Units.radiusMD
            color: Theme.surfaceHover
            border.width: input.activeFocus ? 2 : 0
            border.color: Theme.focusBorder

            TextInput {
                id: input
                anchors.fill: parent
                anchors.leftMargin: 24
                anchors.rightMargin: 24
                anchors.topMargin: 20
                anchors.bottomMargin: 20
                text: root.text
                font.pixelSize: Theme.fontSmall
                color: Theme.textPrimary
                clip: true
                verticalAlignment: TextInput.AlignVCenter
                onTextChanged: root.textEdited(text)
                Keys.onEscapePressed: event => {
                    root.escapePressed();
                    event.accepted = true;
                }
            }
        }
    }
}
