pragma Singleton
import QtQuick

// Holds the available streaming providers and exposes the active one.
//
// `active` is what the rest of the shell consumes (HomeScreen rows,
// StreamManager launch args, SettingsApp section). Flipping
// `streamingEnabled` to false swaps in the empty base TargetProvider, which
// collapses all streaming UI — the no-streaming mode — without any call-site
// changes.
//
// IMPORTANT: root is Item (not QtObject) so it can host the provider child
// objects (which themselves host Process/Timer children).
Item {
    id: providers

    property bool streamingEnabled: true

    readonly property var active: streamingEnabled ? moonlightProvider : noneProvider

    MoonlightProvider {
        id: moonlightProvider
    }

    // Empty base provider = no-streaming mode.
    TargetProvider {
        id: noneProvider
    }
}
