pragma Singleton
import QtQuick

// Test stub for the production `Units` singleton (shell/components/Units.qml),
// which reads Quickshell.screens for its grid unit. Fixed values here keep
// layout assertions deterministic under offscreen. See tests/qml/README.md.
Item {
    property int spacingSM: 6
}
