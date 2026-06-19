import QtQuick
import QtQuick.Layouts
import "lib"

// Status-only Moonlight session indicator (icon + text). Renders whether a
// Moonlight stream is currently live on the host, driven by the `streaming`
// flag from the daemon's `steam-library` reply (the same reply that feeds the
// poster row). A live stream ⇒ "In session"; otherwise "No session" (muted).
//
// IMPORTANT: this is a glance affordance, not a control — it is NOT focusable,
// NOT selectable, has no key handler, and is skipped by the focus chain. It just
// reads state and paints it.
RowLayout {
    id: root

    // True when a Moonlight stream is currently live on the host. The host wires
    // this from `steam-library`'s `streaming` field — there is no probe here.
    property bool inSession: false

    spacing: Units.spacingSM

    Text {
        text: root.inSession ? "●" : "○"  // ● filled / ○ hollow
        font.pixelSize: Theme.fontSmall
        color: root.inSession ? Theme.online : Theme.textMuted
        Layout.alignment: Qt.AlignVCenter
    }

    Text {
        text: root.inSession ? "In session" : "No session"
        font.pixelSize: Theme.fontBody
        font.bold: root.inSession
        color: root.inSession ? Theme.textPrimary : Theme.textMuted
        Layout.alignment: Qt.AlignVCenter
    }
}
