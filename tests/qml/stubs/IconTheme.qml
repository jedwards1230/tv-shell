pragma Singleton
import QtQuick

// Test stub for the production `IconTheme` singleton. An empty `base` makes
// QuickActions build no `file://` icon path, so each glyph renders its Unicode
// fallback — no on-disk Freedesktop icon theme is needed under offscreen. See
// tests/qml/README.md.
Item {
    property string base: ""
}
