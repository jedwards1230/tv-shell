import QtQuick

// Test stub for components/SessionIndicator.qml (a status-only, non-focusable
// RowLayout of Theme-styled text/icon). SteamLibraryView's SegmentedHeader
// instantiates it through the trailing Loader, so it just needs `inSession`.
Item {
    id: root

    property bool inSession: false
}
