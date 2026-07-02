pragma Singleton
import QtQuick

// Test stub for the Quickshell.Io-backed components/AppDiscoveryManager singleton.
// Returns an empty app list so AppsWidget's "All Apps" segment loads with no
// content. `applications` is a plain writable property, so a test can inject rows;
// its auto-generated onApplicationsChanged drives AppsWidget's Connections handler.
Item {
    id: manager

    property var applications: []
    property bool loading: false

    function refresh() {
    }
}
