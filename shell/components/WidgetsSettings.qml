import QtQuick
import QtQuick.Layouts
import "lib"

// Widgets settings — list-first IA (controller-friendly). The page is a flat
// list of widget rows (L0): one focus stop each, A toggles enable in place,
// Right/X drills into that widget's config sub-page (L1: Size, Hide-from-Recent,
// and — Moonlight only — Manage servers, which drills into the embedded server
// management surface at L2). Internal B steps back one level; only at the list
// does B bubble to SettingsPanel (→ sidebar → Home). This keeps the frequent
// on/off task at one focus stop instead of scrolling past every widget's config.
FocusScope {
    id: root

    // Internal nav state: "" = widget list (L0); a widget id = its config (L1);
    // _showServers = Moonlight server management (L2, moonlight only).
    property string _activeWidget: ""
    property bool _showServers: false
    readonly property bool _atList: _activeWidget === ""

    implicitHeight: (_atList ? listCol.implicitHeight : (_showServers ? serversLoader.implicitHeight : configLoader.implicitHeight)) + 2 * Theme.padding

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

    // SettingsPanel calls this on section entry (Right from sidebar). Always
    // reset to the list level so re-entry is predictable.
    function focusFirst() {
        _activeWidget = "";
        _showServers = false;
        Qt.callLater(_focusCurrentLevel);
    }

    function _openWidget(id) {
        _showServers = false;
        _activeWidget = id;
        Qt.callLater(_focusCurrentLevel);
    }

    function _openServers() {
        _showServers = true;
        Qt.callLater(_focusCurrentLevel);
    }

    // Step back one internal level. Returns true if handled (so B/Escape is
    // consumed); false at the list level (so it bubbles to SettingsPanel).
    function _back() {
        if (_showServers) {
            _showServers = false;
            Qt.callLater(_focusCurrentLevel);
            return true;
        }
        if (!_atList) {
            _activeWidget = "";
            Qt.callLater(_focusCurrentLevel);
            return true;
        }
        return false;
    }

    function _focusCurrentLevel() {
        if (_showServers) {
            if (serversLoader.item && serversLoader.item.focusFirst)
                serversLoader.item.focusFirst();
        } else if (!_atList) {
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

        Keys.onReturnPressed: rowScope.toggled()
        Keys.onEnterPressed: rowScope.toggled()
        Keys.onRightPressed: rowScope.drilled()
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
        visible: !root._atList && !root._showServers
        active: visible
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

    // A labeled row: caption on the left, a control supplied by the caller.
    component LabeledRow: RowLayout {
        id: lr
        property string caption: ""
        default property alias body: bodyHolder.data
        Layout.fillWidth: true
        spacing: Units.spacingLG
        Text {
            text: lr.caption
            font.pixelSize: Theme.fontBody
            color: Theme.textSecondary
            Layout.alignment: Qt.AlignVCenter
        }
        Item {
            id: bodyHolder
            Layout.alignment: Qt.AlignVCenter
            implicitWidth: childrenRect.width
            implicitHeight: childrenRect.height
        }
        Item {
            Layout.fillWidth: true
        }
    }

    Component {
        id: moonlightConfigComp
        ConfigPage {
            id: mc
            title: "Moonlight"
            blurb: "Your game-streaming servers. Small = an icon-only online rail; Medium = cards with the server name."
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
            LabeledRow {
                caption: "Size"
                SettingsButtonGroup {
                    id: mSize
                    options: root._sizeOptions
                    isCurrentOption: opt => opt.value === Theme.widgetMoonlightSize
                    onValueSelected: opt => SettingsStore.setWidgetMoonlightSize(opt.value)
                    KeyNavigation.up: mEnabled
                    KeyNavigation.down: mManage
                }
            }
            FocusButton {
                id: mManage
                text: "Manage servers  ›"
                onActivated: root._openServers()
                Keys.onRightPressed: root._openServers()
                KeyNavigation.up: mSize
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
            LabeledRow {
                caption: "Size"
                SettingsButtonGroup {
                    id: npSize
                    options: root._sizeOptions
                    isCurrentOption: opt => opt.value === Theme.widgetSpotifySize
                    onValueSelected: opt => SettingsStore.setWidgetSpotifySize(opt.value)
                    KeyNavigation.up: npEnabled
                    KeyNavigation.down: npHide
                }
            }
            LabeledRow {
                caption: "Hide from Recent"
                FocusButton {
                    id: npHide
                    text: Theme.widgetSpotifyHideFromRecent ? "Hidden" : "Shown"
                    fillActive: Theme.widgetSpotifyHideFromRecent
                    fillColor: Theme.sidebarActive
                    onActivated: SettingsStore.setWidgetSpotifyHideFromRecent(!Theme.widgetSpotifyHideFromRecent)
                    KeyNavigation.up: npSize
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
            LabeledRow {
                caption: "Size"
                SettingsButtonGroup {
                    id: pSize
                    options: root._sizeOptions
                    isCurrentOption: opt => opt.value === Theme.widgetPlexSize
                    onValueSelected: opt => SettingsStore.setWidgetPlexSize(opt.value)
                    KeyNavigation.up: pEnabled
                    KeyNavigation.down: pHide
                }
            }
            LabeledRow {
                caption: "Hide from Recent"
                FocusButton {
                    id: pHide
                    text: Theme.widgetPlexHideFromRecent ? "Hidden" : "Shown"
                    fillActive: Theme.widgetPlexHideFromRecent
                    fillColor: Theme.sidebarActive
                    onActivated: SettingsStore.setWidgetPlexHideFromRecent(!Theme.widgetPlexHideFromRecent)
                    KeyNavigation.up: pSize
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
            LabeledRow {
                caption: "Size"
                SettingsButtonGroup {
                    id: rSize
                    options: root._sizeOptions
                    isCurrentOption: opt => opt.value === Theme.widgetRecentSize
                    onValueSelected: opt => SettingsStore.setWidgetRecentSize(opt.value)
                    KeyNavigation.up: rEnabled
                }
            }
        }
    }

    // ===================== L2 — Moonlight server management =====================
    // Wired in increment 2 — embeds MoonlightSettings here. Placeholder for now
    // so the list → config → servers nav + B-back ladder can be verified first.
    Loader {
        id: serversLoader
        visible: root._showServers
        active: visible
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.top: parent.top
        sourceComponent: serversPlaceholderComp
    }

    Component {
        id: serversPlaceholderComp
        FocusScope {
            id: sp
            implicitHeight: spCol.implicitHeight + 2 * Theme.padding
            function focusFirst() {
                spBack.forceActiveFocus();
            }
            ColumnLayout {
                id: spCol
                anchors.left: parent.left
                anchors.right: parent.right
                anchors.top: parent.top
                anchors.margins: Theme.padding
                spacing: Units.spacingLG
                Text {
                    text: "Manage servers"
                    font.pixelSize: Theme.fontTitle
                    font.bold: true
                    color: Theme.textPrimary
                }
                Text {
                    Layout.fillWidth: true
                    wrapMode: Text.WordWrap
                    text: "Server management moves here (increment 2)."
                    font.pixelSize: Theme.fontCaption
                    color: Theme.textMuted
                }
                FocusButton {
                    id: spBack
                    text: "Back"
                    onActivated: root._back()
                }
                Item {
                    Layout.fillHeight: true
                }
                HintBar {
                    text: "B: Back to Moonlight"
                }
            }
        }
    }
}
