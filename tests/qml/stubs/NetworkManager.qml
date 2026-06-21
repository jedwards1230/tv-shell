pragma Singleton
import QtQuick

// Test stub for the production `NetworkManager` singleton. Only the `connected`
// flag the QuickActions network glyph reads is reproduced. See
// tests/qml/README.md.
Item {
    property bool connected: true
}
