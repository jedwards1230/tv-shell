import QtQuick
import QtQuick.Layouts
import "../"

// Label + text input field row. Used in MoonlightSettings server form
// (Name, Host, App fields — 3× duplication).
//
// The TextInput id must be accessible at the call site for KeyNavigation
// wiring. Expose it via the `inputField` alias.
//
// Usage:
//   LabeledTextInput {
//       label: "Name"
//       text: root.newName
//       onTextChanged: text => root.newName = text
//       KeyNavigation.up: above
//       KeyNavigation.down: below
//       Keys.onEscapePressed: { ... }
//   }
//   // Wire nav to the next field:
//   LabeledTextInput { id: hostRow; ... }
//   // hostRow.inputField is the inner TextInput
RowLayout {
    id: root

    property string label: ""
    property string text: ""
    property alias inputField: input

    signal textChanged(string text)

    spacing: 24

    Text {
        text: root.label
        font.pixelSize: Theme.fontSmall
        color: Theme.textSecondary
        Layout.preferredWidth: 160
    }

    Rectangle {
        Layout.fillWidth: true
        height: 80
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
            onTextChanged: root.textChanged(text)
        }
    }
}
