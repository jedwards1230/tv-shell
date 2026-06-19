import QtQuick
import QtQuick.Layouts
import "../components"
import "../components/lib"

// Widgets settings — list-first IA (controller-friendly). The page is a flat
// list of widget rows (L0): one focus stop each, A toggles enable in place,
// X drills into that widget's config sub-page (L1: Size, Hide-from-Recent, and
// — Moonlight only — the full server-management surface inlined below the Size
// control). Directional keys never drill — they move the highlight only.
// Internal B steps back one level; only at the list does B bubble to SettingsApp
// (→ sidebar → Home). Two levels only (list → config); the former L2 servers
// page was folded into the Moonlight config page. This keeps the frequent on/off
// task at one focus stop instead of scrolling past every widget's config.
FocusScope {
    id: root

    // Internal nav state: "" = widget list (L0); a widget id = its config (L1).
    property string _activeWidget: ""
    readonly property bool _atList: _activeWidget === ""

    implicitHeight: (_atList ? listCol.implicitHeight : configLoader.implicitHeight) + 2 * Theme.padding

    readonly property var _sizeOptions: [
        {
            "label": "Small",
            "value": "small"
        },
        {
            "label": "Medium",
            "value": "medium"
        }
    ]

    // Moonlight has a third size: small = server cards, medium/large = Steam
    // library posters at two scales (two views of the one widget).
    readonly property var _moonlightSizeOptions: [
        {
            "label": "Small",
            "value": "small"
        },
        {
            "label": "Medium",
            "value": "medium"
        },
        {
            "label": "Large",
            "value": "large"
        }
    ]

    // SettingsApp calls this on section entry (Right from sidebar). Always
    // reset to the list level so re-entry is predictable.
    function focusFirst() {
        _activeWidget = "";
        Qt.callLater(_focusCurrentLevel);
    }

    // Entry point for a deep-link routed by SettingsApp (the demoted
    // "moonlight"/"streaming" slug → the Moonlight config page, which now hosts
    // the server-management surface inline). Opens that page directly.
    function applyDeepTarget(t) {
        if (t === "moonlight") {
            _lastListId = "moonlight";
            _activeWidget = "moonlight";
            Qt.callLater(_focusCurrentLevel);
        }
    }

    function _openWidget(id) {
        _activeWidget = id;
        Qt.callLater(_focusCurrentLevel);
    }

    // Step back one internal level. Returns true if handled (so B/Escape is
    // consumed); false at the list level (so it bubbles to SettingsApp).
    function _back() {
        if (!_atList) {
            _activeWidget = "";
            Qt.callLater(_focusCurrentLevel);
            return true;
        }
        return false;
    }

    // Focus the current level's entry control. Driven by both Qt.callLater (after
    // a nav state change) and the config Loader's onLoaded (covers the case where
    // the item is created a tick after the state flips) — double-calling is
    // harmless.
    function _focusCurrentLevel() {
        if (!_atList) {
            if (configLoader.item && configLoader.item.focusFirst)
                configLoader.item.focusFirst();
        } else {
            _focusListRow(_lastListId);
        }
    }

    // Remember which row we drilled in from, so B returns focus to it.
    property string _lastListId: "moonlight"
    function _focusListRow(id) {
        if (id === "nowplaying")
            nowPlayingRow.forceActiveFocus();
        else if (id === "plex")
            plexRow.forceActiveFocus();
        else if (id === "recent")
            recentRow.forceActiveFocus();
        else
            moonlightRow.forceActiveFocus();
    }

    // B / Escape: consume to pop our own stack; only bubble at the list level.
    Keys.onEscapePressed: event => {
        if (!root._back())
            event.accepted = false;
    }
    Keys.onPressed: event => {
        if (event.key === Qt.Key_B && !event.modifiers) {
            if (root._back())
                event.accepted = true;
            else
                event.accepted = false;
        }
    }

    // ===================== L0 — widget list =====================
    component WidgetRow: FocusScope {
        id: rowScope
        property string label: ""
        property bool isEnabled: false
        signal toggled
        signal drilled

        Layout.fillWidth: true
        implicitHeight: 80

        // A/Return quick-toggles enable; X opens the config sub-page. Right is
        // deliberately NOT a drill — directional keys move the highlight only, so
        // the cursor can't chain into a sub-view by accident (B backs out a level).
        Keys.onReturnPressed: rowScope.toggled()
        Keys.onEnterPressed: rowScope.toggled()
        Keys.onPressed: event => {
            if (event.key === Qt.Key_X && !event.modifiers) {
                rowScope.drilled();
                event.accepted = true;
            }
        }

        SettingsListRow {
            anchors.fill: parent
            selected: rowScope.activeFocus

            RowLayout {
                anchors.fill: parent
                anchors.leftMargin: Units.spacingLG
                anchors.rightMargin: Units.spacingLG
                spacing: Units.spacingMD

                Text {
                    text: rowScope.label
                    font.pixelSize: Theme.fontTitle
                    font.bold: true
                    color: Theme.textPrimary
                    Layout.alignment: Qt.AlignVCenter
                }
                Item {
                    Layout.fillWidth: true
                }
                Text {
                    text: rowScope.isEnabled ? "Enabled" : "Disabled"
                    font.pixelSize: Theme.fontBody
                    color: rowScope.isEnabled ? Theme.sidebarActive : Theme.textMuted
                    Layout.alignment: Qt.AlignVCenter
                }
                Text {
                    text: "›"  // ›
                    font.pixelSize: Theme.fontTitle
                    color: Theme.textMuted
                    Layout.alignment: Qt.AlignVCenter
                }
            }
        }

        MouseArea {
            anchors.fill: parent
            hoverEnabled: true
            cursorShape: Qt.PointingHandCursor
            onPositionChanged: mouse => {
                let p = mapToItem(null, mouse.x, mouse.y);
                Theme.pointerMoved(p.x, p.y);
            }
            onClicked: {
                rowScope.forceActiveFocus();
                rowScope.toggled();
            }
        }
    }

    ColumnLayout {
        id: listCol
        visible: root._atList
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.top: parent.top
        anchors.margins: Theme.padding
        spacing: Units.spacingMD

        WidgetRow {
            id: moonlightRow
            label: "Moonlight"
            isEnabled: Theme.widgetMoonlightEnabled
            onToggled: SettingsStore.setWidgetMoonlightEnabled(!Theme.widgetMoonlightEnabled)
            onDrilled: {
                root._lastListId = "moonlight";
                root._openWidget("moonlight");
            }
            KeyNavigation.down: nowPlayingRow
        }
        WidgetRow {
            id: nowPlayingRow
            label: "Now Playing"
            isEnabled: Theme.widgetSpotifyEnabled
            onToggled: SettingsStore.setWidgetSpotifyEnabled(!Theme.widgetSpotifyEnabled)
            onDrilled: {
                root._lastListId = "nowplaying";
                root._openWidget("nowplaying");
            }
            KeyNavigation.up: moonlightRow
            KeyNavigation.down: plexRow
        }
        WidgetRow {
            id: plexRow
            label: "Plex"
            isEnabled: Theme.widgetPlexEnabled
            onToggled: SettingsStore.setWidgetPlexEnabled(!Theme.widgetPlexEnabled)
            onDrilled: {
                root._lastListId = "plex";
                root._openWidget("plex");
            }
            KeyNavigation.up: nowPlayingRow
            KeyNavigation.down: recentRow
        }
        WidgetRow {
            id: recentRow
            label: "Recent"
            isEnabled: Theme.widgetRecentEnabled
            onToggled: SettingsStore.setWidgetRecentEnabled(!Theme.widgetRecentEnabled)
            onDrilled: {
                root._lastListId = "recent";
                root._openWidget("recent");
            }
            KeyNavigation.up: plexRow
        }

        Item {
            Layout.fillHeight: true
        }

        HintBar {
            text: "A: Enable/Disable    X: Configure    B: Back"
        }
    }

    // ===================== L1 — per-widget config =====================
    Loader {
        id: configLoader
        visible: !root._atList
        active: visible
        onLoaded: root._focusCurrentLevel()
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.top: parent.top
        sourceComponent: {
            switch (root._activeWidget) {
            case "moonlight":
                return moonlightConfigComp;
            case "nowplaying":
                return nowPlayingConfigComp;
            case "plex":
                return plexConfigComp;
            case "recent":
                return recentConfigComp;
            default:
                return null;
            }
        }
    }

    // Reusable config-page scaffold: title + description + a column for controls.
    component ConfigPage: FocusScope {
        id: cfg
        property string title: ""
        property string blurb: ""
        default property alias controls: controlsCol.data
        implicitHeight: cfgCol.implicitHeight + 2 * Theme.padding

        ColumnLayout {
            id: cfgCol
            anchors.left: parent.left
            anchors.right: parent.right
            anchors.top: parent.top
            anchors.margins: Theme.padding
            spacing: Units.spacingLG

            Text {
                text: cfg.title
                font.pixelSize: Theme.fontTitle
                font.bold: true
                color: Theme.textPrimary
            }
            Text {
                Layout.fillWidth: true
                wrapMode: Text.WordWrap
                text: cfg.blurb
                font.pixelSize: Theme.fontCaption
                color: Theme.textMuted
            }
            ColumnLayout {
                id: controlsCol
                Layout.fillWidth: true
                spacing: Units.spacingLG
            }
            Item {
                Layout.fillHeight: true
            }
            HintBar {
                text: "B: Back to Widgets"
            }
        }
    }

    Component {
        id: moonlightConfigComp
        ConfigPage {
            id: mc
            title: "Moonlight"
            blurb: "Jump into game streaming. Small = your streaming-server cards; Medium and Large = your Steam library as posters (smaller / full size)."
            function focusFirst() {
                mEnabled.forceActiveFocus();
            }

            FocusButton {
                id: mEnabled
                text: Theme.widgetMoonlightEnabled ? "Enabled" : "Disabled"
                fillActive: Theme.widgetMoonlightEnabled
                fillColor: Theme.sidebarActive
                onActivated: SettingsStore.setWidgetMoonlightEnabled(!Theme.widgetMoonlightEnabled)
                KeyNavigation.down: mSize
            }
            RowLayout {
                Layout.fillWidth: true
                spacing: Units.spacingLG
                Text {
                    text: "Size"
                    font.pixelSize: Theme.fontBody
                    color: Theme.textSecondary
                    Layout.alignment: Qt.AlignVCenter
                }
                SettingsButtonGroup {
                    id: mSize
                    Layout.alignment: Qt.AlignVCenter
                    options: root._moonlightSizeOptions
                    isCurrentOption: opt => opt.value === Theme.widgetMoonlightSize
                    onValueSelected: opt => SettingsStore.setWidgetMoonlightSize(opt.value)
                    KeyNavigation.up: mEnabled
                    KeyNavigation.down: mServers
                }
                Item {
                    Layout.fillWidth: true
                }
            }

            // Server management inlined directly on this page (no separate L2
            // servers page). Down off the Size control enters its server list;
            // Up off the first server row returns here (upTarget). Stays a
            // FocusScope so SettingsApp's outer Flickable scroll-follow keeps
            // tracking the focused control — not wrapped in a self-scrolling list.
            MoonlightSettings {
                id: mServers
                Layout.fillWidth: true
                upTarget: mSize
            }
        }
    }

    Component {
        id: nowPlayingConfigComp
        ConfigPage {
            id: npc
            title: "Now Playing"
            blurb: "Cover art, track info, and transport controls for the active player. When off, the player appears in the Recent row instead."
            function focusFirst() {
                npEnabled.forceActiveFocus();
            }

            FocusButton {
                id: npEnabled
                text: Theme.widgetSpotifyEnabled ? "Enabled" : "Disabled"
                fillActive: Theme.widgetSpotifyEnabled
                fillColor: Theme.sidebarActive
                onActivated: SettingsStore.setWidgetSpotifyEnabled(!Theme.widgetSpotifyEnabled)
                KeyNavigation.down: npSize
            }
            RowLayout {
                Layout.fillWidth: true
                spacing: Units.spacingLG
                Text {
                    text: "Size"
                    font.pixelSize: Theme.fontBody
                    color: Theme.textSecondary
                    Layout.alignment: Qt.AlignVCenter
                }
                SettingsButtonGroup {
                    id: npSize
                    Layout.alignment: Qt.AlignVCenter
                    options: root._sizeOptions
                    isCurrentOption: opt => opt.value === Theme.widgetSpotifySize
                    onValueSelected: opt => SettingsStore.setWidgetSpotifySize(opt.value)
                    KeyNavigation.up: npEnabled
                    KeyNavigation.down: npHide
                }
                Item {
                    Layout.fillWidth: true
                }
            }
            RowLayout {
                Layout.fillWidth: true
                spacing: Units.spacingLG
                Text {
                    text: "Hide from Recent"
                    font.pixelSize: Theme.fontBody
                    color: Theme.textSecondary
                    Layout.alignment: Qt.AlignVCenter
                }
                FocusButton {
                    id: npHide
                    Layout.alignment: Qt.AlignVCenter
                    text: Theme.widgetSpotifyHideFromRecent ? "Hidden" : "Shown"
                    fillActive: Theme.widgetSpotifyHideFromRecent
                    fillColor: Theme.sidebarActive
                    onActivated: SettingsStore.setWidgetSpotifyHideFromRecent(!Theme.widgetSpotifyHideFromRecent)
                    KeyNavigation.up: npSize
                }
                Item {
                    Layout.fillWidth: true
                }
            }
        }
    }

    Component {
        id: plexConfigComp
        ConfigPage {
            id: pc
            title: "Plex"
            blurb: "Up Next and Recently Added in one row. Small = a poster-only rail; Medium = posters with titles and resume bars."
            function focusFirst() {
                pEnabled.forceActiveFocus();
            }

            FocusButton {
                id: pEnabled
                text: Theme.widgetPlexEnabled ? "Enabled" : "Disabled"
                fillActive: Theme.widgetPlexEnabled
                fillColor: Theme.sidebarActive
                onActivated: SettingsStore.setWidgetPlexEnabled(!Theme.widgetPlexEnabled)
                KeyNavigation.down: pSize
            }
            RowLayout {
                Layout.fillWidth: true
                spacing: Units.spacingLG
                Text {
                    text: "Size"
                    font.pixelSize: Theme.fontBody
                    color: Theme.textSecondary
                    Layout.alignment: Qt.AlignVCenter
                }
                SettingsButtonGroup {
                    id: pSize
                    Layout.alignment: Qt.AlignVCenter
                    options: root._sizeOptions
                    isCurrentOption: opt => opt.value === Theme.widgetPlexSize
                    onValueSelected: opt => SettingsStore.setWidgetPlexSize(opt.value)
                    KeyNavigation.up: pEnabled
                    KeyNavigation.down: pHide
                }
                Item {
                    Layout.fillWidth: true
                }
            }
            RowLayout {
                Layout.fillWidth: true
                spacing: Units.spacingLG
                Text {
                    text: "Hide from Recent"
                    font.pixelSize: Theme.fontBody
                    color: Theme.textSecondary
                    Layout.alignment: Qt.AlignVCenter
                }
                FocusButton {
                    id: pHide
                    Layout.alignment: Qt.AlignVCenter
                    text: Theme.widgetPlexHideFromRecent ? "Hidden" : "Shown"
                    fillActive: Theme.widgetPlexHideFromRecent
                    fillColor: Theme.sidebarActive
                    onActivated: SettingsStore.setWidgetPlexHideFromRecent(!Theme.widgetPlexHideFromRecent)
                    KeyNavigation.up: pSize
                }
                Item {
                    Layout.fillWidth: true
                }
            }
        }
    }

    Component {
        id: recentConfigComp
        ConfigPage {
            id: rc
            title: "Recent"
            blurb: "Running and recently-launched apps. Small = icon-only tiles; Medium = icon + name cards."
            function focusFirst() {
                rEnabled.forceActiveFocus();
            }

            FocusButton {
                id: rEnabled
                text: Theme.widgetRecentEnabled ? "Enabled" : "Disabled"
                fillActive: Theme.widgetRecentEnabled
                fillColor: Theme.sidebarActive
                onActivated: SettingsStore.setWidgetRecentEnabled(!Theme.widgetRecentEnabled)
                KeyNavigation.down: rSize
            }
            RowLayout {
                Layout.fillWidth: true
                spacing: Units.spacingLG
                Text {
                    text: "Size"
                    font.pixelSize: Theme.fontBody
                    color: Theme.textSecondary
                    Layout.alignment: Qt.AlignVCenter
                }
                SettingsButtonGroup {
                    id: rSize
                    Layout.alignment: Qt.AlignVCenter
                    options: root._sizeOptions
                    isCurrentOption: opt => opt.value === Theme.widgetRecentSize
                    onValueSelected: opt => SettingsStore.setWidgetRecentSize(opt.value)
                    KeyNavigation.up: rEnabled
                }
                Item {
                    Layout.fillWidth: true
                }
            }
        }
    }
}
