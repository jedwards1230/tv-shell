import QtQuick

// Test-only single-stop home widget — a minimal Widget whose focusability tracks
// `widgetEnabled` (via visible), so a test can flip a registry entry's `enabled`
// and assert the WidgetHost chain reroutes around it. Single-stop = no
// firstRow/lastRow, so the chain targets the widget itself.
Widget {
    wantVisible: widgetEnabled
    implicitWidth: 100
    implicitHeight: 40
}
