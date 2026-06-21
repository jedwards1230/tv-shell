pragma Singleton
import QtQuick

// Test stub for the production `SettingsStore` singleton
// (shell/components/SettingsStore.qml), which does all QML-side settings I/O
// over the daemon socket (Quickshell.Io). Here it's a plain in-memory holder so
// the theme-toggle path (QuickActions index 2 -> setThemeMode cycle) can be
// asserted without a daemon. See tests/qml/README.md.
Item {
    property string themeMode: "auto"

    function setThemeMode(mode) {
        themeMode = mode;
    }
}
