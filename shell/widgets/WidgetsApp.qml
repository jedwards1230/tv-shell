import QtQuick
import QtQuick.Layouts
import "../components"
import "../components/lib"

// The Widgets "app" — a top-level surface, peer of Home / Library / Settings,
// mirroring SettingsApp's "own app" abstraction. It owns the back-stack between
// its two leaf views (the L0 WidgetList and the L1 WidgetConfig) the way
// SettingsApp owns its sidebar↔content + B-stack, and exposes the SAME public
// API shape so ShellLayout / ScreenManager drive it without reaching inside:
//
//   open()        — show on the widget list
//   openPage(id)  — deep-link straight into a widget's config (e.g. "moonlight")
//   close()       — dismiss without emitting closed() (host reset paths)
//   signal closed — emitted when the user backs out past the list (→ return Home)
//   signal userActivity — forwarded from the leaves (idle-timer reset)
//
// Reached from the nav drawer, the QuickActions Widgets glyph, and the
// `widgets` / `moonlight` / `streaming` intents (all rerouted in ShellLayout to
// screens.push("widgets")). It is SCHEMA-DRIVEN from WidgetManifests — no
// per-widget page code lives here; the leaves render from the manifests.
FocusScope {
    id: root
    visible: false

    signal closed
    signal userActivity

    // Back-stack: "" = widget list (L0); a widget id = its config (L1). The app
    // is the single owner of this transition — the leaves request it via signals.
    property string _activeWidget: ""
    readonly property bool _atList: _activeWidget === ""

    // ── Public API ──────────────────────────────────────────────────────────
    // open() — show on the list, mirroring SettingsApp.open(): visible=true then
    // forceActiveFocus (the FocusScope forwards focus to its focused child), and
    // land focus on the list once it has realized.
    function open() {
        root._activeWidget = "";
        if (!visible)
            visible = true;
        forceActiveFocus();
        Qt.callLater(widgetList.focusEntry);
    }

    // openPage(id) — deep-link into a widget's config. Unknown id falls back to
    // the list (returns false so the caller can log it), matching SettingsApp.
    function openPage(id) {
        if (!visible)
            visible = true;
        forceActiveFocus();
        if (id && WidgetManifests.byId(id)) {
            widgetList.noteDrillIn(id);
            root._activeWidget = id;
            Qt.callLater(widgetConfig.focusFirstControl);
            return true;
        }
        root._activeWidget = "";
        Qt.callLater(widgetList.focusEntry);
        return false;
    }

    // close() — dismiss without emitting closed() (no user-intent echo); used by
    // the host's reset/popToHome paths.
    function close() {
        visible = false;
    }

    // Step back one internal level. Returns true if handled (consume B/Escape);
    // false at the list level so it bubbles to closed().
    function _back() {
        if (!root._atList) {
            root._activeWidget = "";
            Qt.callLater(widgetList.focusEntry);
            return true;
        }
        return false;
    }

    // B / Escape: pop our own stack; only bubble (→ closed) at the list level.
    Keys.onEscapePressed: event => {
        if (!root._back()) {
            root.userActivity();
            root.closed();
        }
    }
    Keys.onPressed: event => {
        if (event.key === Qt.Key_B && !event.modifiers) {
            if (root._back()) {
                event.accepted = true;
            } else {
                root.userActivity();
                root.closed();
                event.accepted = true;
            }
        }
    }

    // Opaque scrim so Home underneath doesn't bleed through.
    Rectangle {
        anchors.fill: parent
        color: Theme.background
    }

    // === Shared header chrome (stays put across both levels) ===
    RowLayout {
        id: header
        anchors.top: parent.top
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.margins: Theme.padding
        Layout.preferredHeight: Units.gridUnit * 4
        spacing: 16

        ColumnLayout {
            Layout.alignment: Qt.AlignVCenter
            spacing: 6
            Text {
                text: "Widgets"
                font.pixelSize: Theme.fontHero
                font.bold: true
                color: Theme.textPrimary
            }
            Text {
                text: "Arrange and configure your home screen widgets"
                font.pixelSize: Theme.fontBody
                color: Theme.textSecondary
            }
        }
        Item {
            Layout.fillWidth: true
        }
        Text {
            text: "B: Back"
            font.pixelSize: Theme.fontHint
            color: Theme.textMuted
            Layout.alignment: Qt.AlignVCenter
        }
    }

    // === Content area — the two leaf views, swapped by visibility. The app owns
    // the transition; each leaf owns its own scroll + focus chain. ===
    Item {
        id: content
        anchors.top: header.bottom
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.bottom: parent.bottom
        anchors.leftMargin: Theme.padding
        anchors.rightMargin: Theme.padding
        anchors.topMargin: Units.spacingMD

        WidgetList {
            id: widgetList
            anchors.fill: parent
            visible: root._atList
            focus: root._atList
            onConfigureRequested: id => {
                root._activeWidget = id;
                Qt.callLater(widgetConfig.focusFirstControl);
            }
            onUserActivity: root.userActivity()
        }

        WidgetConfig {
            id: widgetConfig
            anchors.fill: parent
            visible: !root._atList
            focus: !root._atList
            widgetId: root._activeWidget
            onUserActivity: root.userActivity()
        }
    }
}
