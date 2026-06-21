pragma Singleton
import QtQuick

// Test stub for the production `Theme` singleton (shell/components/Theme.qml).
// QtQuickTest runs headless under QT_QPA_PLATFORM=offscreen with NO Quickshell
// runtime, so the real Theme — which reads SettingsStore (Quickshell.Io) and
// InputMode — can't load. This stub reproduces ONLY the surface consumed by the
// components under test (QuickActions / QuickActionButton / CountBadge). Keep it
// in sync with the real API as the tested component set grows; see
// tests/qml/README.md.
Item {
    // Mirror the real delegation so the theme-toggle path is exercised end to
    // end: Theme.themeMode reflects SettingsStore, and toggling SettingsStore
    // flips it back here.
    readonly property string themeMode: SettingsStore.themeMode
    readonly property bool mouseMode: InputMode.mouseMode

    property int fontHint: 18

    property color textPrimary: "#ffffff"
    property color textMuted: "#9aa0a6"
    property color warning: "#e0a030"
    property color surfaceHover: "#202020"
    property color focusBorder: "#dd3333"
    property color crimson: "#dd3333"
    property color textOnDark: "#ffffff"
}
