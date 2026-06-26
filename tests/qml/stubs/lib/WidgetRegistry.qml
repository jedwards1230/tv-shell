pragma Singleton
import QtQuick

// Test-only stand-in for the production WidgetRegistry (which pulls in the real
// widgets → Quickshell). Provides a 3-widget set — single-stop A, multi-row B,
// single-stop C — with WRITABLE `enabled` flags so tst_widgethost can disable a
// widget and assert the WidgetHost focus chain reroutes around it. Same shape as
// the real registry (widgetId / component / enabled / size).
Item {
    id: registry

    readonly property var widgets: [entryA, entryB, entryC]

    function entryById(widgetId) {
        for (var i = 0; i < registry.widgets.length; i++) {
            if (registry.widgets[i].widgetId === widgetId)
                return registry.widgets[i];
        }
        return null;
    }

    QtObject {
        id: entryA
        readonly property string widgetId: "a"
        readonly property Component component: Component {
            StubWidget {}
        }
        property bool enabled: true
        property string size: ""
    }

    QtObject {
        id: entryB
        readonly property string widgetId: "b"
        readonly property Component component: Component {
            StubMultiRowWidget {}
        }
        property bool enabled: true
        property string size: ""
    }

    QtObject {
        id: entryC
        readonly property string widgetId: "c"
        readonly property Component component: Component {
            StubWidget {}
        }
        property bool enabled: true
        property string size: ""
    }
}
