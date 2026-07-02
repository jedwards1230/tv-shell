pragma Singleton
import QtQuick

// Headless stub for the Quickshell.Services.Mpris `Mpris` singleton. MprisPlayerBase
// reads `Mpris.players` (and `.values` when non-null). Default null → the widget
// sees no player and collapses to zero height; the widget-contract test injects
// `{ values: [fakePlayer] }` to exercise the focused/transport path.
QtObject {
    property var players: null
}
