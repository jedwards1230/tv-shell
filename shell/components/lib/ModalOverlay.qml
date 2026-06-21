import QtQuick
import "../"

// Base for a full-screen modal overlay — the pattern PowerOverlay /
// NotificationCenter / ErrorLogViewer / SessionDialog each hand-rolled: a
// FocusScope filling the parent, gated visible/focus on `opened`, a
// DimmedBackdrop scrim (click-to-dismiss), and B/Escape → close. It promotes the
// shared half of Drawer.qml's modal handling without the slide-in geometry (these
// overlays appear in place / centered, they don't slide from an edge).
//
//   ModalOverlay {
//       id: root
//       opened: someBool
//       onClosed: someBool = false
//       // content (a centered card, a list, …) as direct children
//   }
//
// Caller owns the `opened` binding: the base only EMITS closed(); it never writes
// `opened` itself, so a host binding (`opened: shell.powerOpen`) is never broken.
// `dimLevel` defaults to the canonical Theme.scrimOpacity; an intentionally
// heavier surface (the power menu) overrides it.
FocusScope {
    id: overlay

    // Host-owned open state. The base reads it (visible/focus); it never writes it.
    property bool opened: false

    // Scrim dim. Defaults to the canonical modal value; override for a heavier
    // (near-opaque) surface like the power chooser.
    property real dimLevel: Theme.scrimOpacity

    // Click-on-scrim and B/Escape both emit this. Host sets `opened = false`.
    signal closed

    // Overlay content lands above the scrim, via the default property.
    default property alias content: contentHost.data

    anchors.fill: parent
    visible: opened
    focus: opened

    DimmedBackdrop {
        dimLevel: overlay.dimLevel
        onClicked: overlay.closed()
    }

    Item {
        id: contentHost
        anchors.fill: parent
    }

    // B / Escape close. The controller B button arrives as Escape; the literal
    // keyboard 'B' is handled too. Emit only — the host owns `opened`.
    Keys.onEscapePressed: overlay.closed()
    Keys.onPressed: event => {
        if (event.key === Qt.Key_B && !event.modifiers) {
            overlay.closed();
            event.accepted = true;
        }
    }
}
