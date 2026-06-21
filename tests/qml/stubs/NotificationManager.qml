pragma Singleton
import QtQuick

// Test stub for the production `NotificationManager` singleton. Only the
// `unreadCount` the QuickActions notification glyph + CountBadge read is
// reproduced. See tests/qml/README.md.
Item {
    property int unreadCount: 0
}
