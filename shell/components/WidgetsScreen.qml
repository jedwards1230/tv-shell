import QtQuick
import QtQuick.Layouts
import QtQuick.Window
import "lib"

// Top-level Widgets surface (#249 Phase 3). Promoted out of the Settings sidebar
// to a peer of Home / Library / Settings, reached from the nav drawer and the
// `widgets` / `moonlight` / `streaming` deep-links (rerouted in ShellLayout). It
// is SCHEMA-DRIVEN from WidgetManifests: no per-widget page code.
//
// Lifecycle mirrors LibraryScreen: toggled by `visible` from ShellLayout, B/Escape
// at the list level emits `closed`, and it owns its own internal B-stack
// (config L1 → list L0 → closed).
//
// Two levels:
//   L0 — widget list, iterated in live `order`. One row per widget = one focus
//        stop. A toggles enabled in place; X drills into config (L1); Left/Right
//        reorder (plasma-bigscreen style, persisted) keeping focus on the moved
//        widget; Up/Down move between rows.
//   L1 — per-widget config, generated from the manifest `config` schema (enabled
//        toggle + a generic control per entry). Moonlight additionally inlines the
//        full server-management surface (MoonlightSettings) below its Size control.
FocusScope {
    id: root

    // Back-out to Home (B/Escape at the list level).
    signal closed
    // Any navigation — lets ShellLayout reset the auto-suspend idle timer.
    signal userActivity

    // Internal nav state: "" = widget list (L0); a widget id = its config (L1).
    property string _activeWidget: ""
    readonly property bool _atList: _activeWidget === ""

    // Remember which row we drilled in from so B returns focus to it.
    property string _lastListId: ""
    // Set on a reorder so the moved row regains focus once the list rebuilds.
    property string _pendingFocusId: ""

    // === Order model ===
    // Widget ids sorted by live order. Guarded so it reassigns ONLY when the
    // actual sequence changes — an enable/size toggle reassigns SettingsStore.widgets
    // but leaves order untouched, so the L0 Repeater is NOT rebuilt (focus kept).
    property var _orderedIds: []
    function _recomputeOrder() {
        var ids = WidgetManifests.ids();
        var arr = [];
        for (var i = 0; i < ids.length; i++)
            arr.push({
                "id": ids[i],
                "order": SettingsStore.widget(ids[i]).order,
                "i": i
            });
        arr.sort(function (a, b) {
            if (a.order !== b.order)
                return a.order - b.order;
            return a.i - b.i;
        });
        var next = [];
        for (var k = 0; k < arr.length; k++)
            next.push(arr[k].id);
        if (next.join(",") !== root._orderedIds.join(","))
            root._orderedIds = next;
    }

    Connections {
        target: SettingsStore
        function onWidgetsChanged() {
            root._recomputeOrder();
        }
    }

    on_OrderedIdsChanged: {
        if (root._atList && root._pendingFocusId !== "") {
            var id = root._pendingFocusId;
            root._pendingFocusId = "";
            Qt.callLater(function () {
                root._focusRow(id);
            });
        }
    }

    // === Capability gate (Phase 5 stub) ===
    // The real capabilities query lands in Phase 5; for now everything is
    // available. Kept structured so the row can already render "Unavailable: <cap>".
    function _capabilityAvailable(cap) {
        return true;
    }
    function _missingCapability(id) {
        var m = WidgetManifests.byId(id);
        if (!m)
            return "";
        for (var i = 0; i < m.requires.length; i++) {
            if (!root._capabilityAvailable(m.requires[i]))
                return m.requires[i];
        }
        return "";
    }

    // === Public API (called by ScreenManager) ===
    function focusFirst() {
        root._activeWidget = "";
        Qt.callLater(root._focusListEntry);
    }
    function applyDeepTarget(t) {
        if (t === "moonlight") {
            root._openConfig("moonlight");
        } else {
            focusFirst();
        }
    }

    // === Internal nav ===
    function _focusListEntry() {
        if (root._orderedIds.length === 0)
            return;
        var want = (root._lastListId !== "" && root._orderedIds.indexOf(root._lastListId) >= 0) ? root._lastListId : root._orderedIds[0];
        root._focusRow(want);
    }
    function _openConfig(id) {
        root._lastListId = id;
        root._activeWidget = id;
        Qt.callLater(root._focusL1First);
    }
    function _focusL1First() {
        if (!root._atList && l1Repeater.count > 0) {
            var it = l1Repeater.itemAt(0);
            if (it)
                it.forceActiveFocus();
        }
    }
    // Step back one internal level. Returns true if handled (consume B/Escape);
    // false at the list level so it bubbles to closed().
    function _back() {
        if (!root._atList) {
            root._activeWidget = "";
            Qt.callLater(root._focusListEntry);
            return true;
        }
        return false;
    }

    // === L0 row helpers ===
    function _rowItemById(id) {
        for (var i = 0; i < l0Repeater.count; i++) {
            var it = l0Repeater.itemAt(i);
            if (it && it.rowId === id)
                return it;
        }
        return null;
    }
    function _focusRow(id) {
        var it = root._rowItemById(id);
        if (it)
            it.forceActiveFocus();
    }
    function _focusSibling(index, delta) {
        var ni = index + delta;
        if (ni < 0 || ni >= root._orderedIds.length)
            return;
        root._focusRow(root._orderedIds[ni]);
    }
    function _toggle(id) {
        SettingsStore.setWidget(id, "enabled", !SettingsStore.widget(id).enabled);
    }
    function _reorder(id, delta) {
        var ids = root._orderedIds.slice();
        var idx = ids.indexOf(id);
        var ni = idx + delta;
        if (idx < 0 || ni < 0 || ni >= ids.length)
            return;
        var tmp = ids[ni];
        ids[ni] = ids[idx];
        ids[idx] = tmp;
        root._pendingFocusId = id;
        SettingsStore.setWidgetOrder(ids);
    }

    // === L1 focus chain (generic over the manifest control list) ===
    function _l1Up(index) {
        if (index > 0) {
            var it = l1Repeater.itemAt(index - 1);
            if (it)
                it.forceActiveFocus();
        }
    }
    function _l1Down(index) {
        if (index < l1Repeater.count - 1) {
            var it = l1Repeater.itemAt(index + 1);
            if (it)
                it.forceActiveFocus();
            return;
        }
        // Last control: drop into the Moonlight server surface when present.
        if (root._activeWidget === "moonlight" && moonlightServersLoader.item)
            moonlightServersLoader.item.forceActiveFocus();
    }

    // Build the L1 control list for a widget: a synthetic "enabled" entry first,
    // then the manifest's config entries (size + any prefs) in order.
    function _l1Schema(id) {
        var arr = [
            {
                "key": "__enabled",
                "type": "__enabled",
                "label": "Enabled"
            }
        ];
        var m = WidgetManifests.byId(id);
        if (m) {
            for (var i = 0; i < m.config.length; i++)
                arr.push(m.config[i]);
        }
        return arr;
    }

    function _cap(s) {
        return s.length > 0 ? s.charAt(0).toUpperCase() + s.slice(1) : s;
    }
    function _optionsFor(entry) {
        var out = [];
        var vals = entry.values || [];
        for (var i = 0; i < vals.length; i++)
            out.push({
                "label": root._cap(vals[i]),
                "value": vals[i]
            });
        return out;
    }

    Component.onCompleted: root._recomputeOrder()

    // B / Escape: consume to pop our own stack; only bubble (→ closed) at the list.
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

    // === Header ===
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

    // === Scrollable content (follows focus so the Moonlight server list is
    // reachable inside Widgets ▸ Moonlight). ===
    Flickable {
        id: scrollView
        anchors.top: header.bottom
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.bottom: parent.bottom
        anchors.leftMargin: Theme.padding
        anchors.rightMargin: Theme.padding
        anchors.topMargin: Units.spacingMD
        clip: true
        interactive: true
        contentWidth: width
        contentHeight: root._atList ? l0Col.implicitHeight : l1Col.implicitHeight
        boundsBehavior: Flickable.StopAtBounds

        Behavior on contentY {
            NumberAnimation {
                duration: 150
                easing.type: Easing.OutCubic
            }
        }

        function ensureVisible(it) {
            if (!it)
                return;
            var p = it.mapToItem(scrollView.contentItem, 0, 0);
            var maxY = Math.max(0, scrollView.contentHeight - scrollView.height);
            if (p.y < scrollView.contentY)
                scrollView.contentY = Math.max(0, p.y - 24);
            else if (p.y + it.height > scrollView.contentY + scrollView.height)
                scrollView.contentY = (p.y >= maxY) ? maxY : Math.min(p.y + it.height - scrollView.height + 24, maxY);
        }

        // ===================== L0 — widget list =====================
        ColumnLayout {
            id: l0Col
            visible: root._atList
            width: scrollView.width
            spacing: Units.spacingMD

            Repeater {
                id: l0Repeater
                model: root._orderedIds

                delegate: FocusScope {
                    id: rowScope
                    required property int index
                    required property var modelData
                    readonly property string rowId: modelData
                    readonly property string _missing: root._missingCapability(modelData)

                    Layout.fillWidth: true
                    implicitHeight: 96

                    Keys.onReturnPressed: root._toggle(rowScope.rowId)
                    Keys.onEnterPressed: root._toggle(rowScope.rowId)
                    Keys.onUpPressed: root._focusSibling(rowScope.index, -1)
                    Keys.onDownPressed: root._focusSibling(rowScope.index, 1)
                    Keys.onLeftPressed: root._reorder(rowScope.rowId, -1)
                    Keys.onRightPressed: root._reorder(rowScope.rowId, 1)
                    Keys.onPressed: event => {
                        if (event.key === Qt.Key_X && !event.modifiers) {
                            root._openConfig(rowScope.rowId);
                            event.accepted = true;
                        }
                    }

                    onActiveFocusChanged: if (activeFocus)
                        scrollView.ensureVisible(rowScope)

                    SettingsListRow {
                        anchors.fill: parent
                        selected: rowScope.activeFocus

                        RowLayout {
                            anchors.fill: parent
                            anchors.leftMargin: Units.spacingLG
                            anchors.rightMargin: Units.spacingLG
                            spacing: Units.spacingMD

                            ColumnLayout {
                                Layout.alignment: Qt.AlignVCenter
                                spacing: 2
                                Text {
                                    text: {
                                        var m = WidgetManifests.byId(rowScope.rowId);
                                        return m ? m.name : rowScope.rowId;
                                    }
                                    font.pixelSize: Theme.fontTitle
                                    font.bold: true
                                    color: Theme.textPrimary
                                }
                                Text {
                                    visible: rowScope._missing !== ""
                                    text: "Unavailable: " + rowScope._missing
                                    font.pixelSize: Theme.fontCaption
                                    color: Theme.offline
                                }
                            }
                            Item {
                                Layout.fillWidth: true
                            }
                            Text {
                                text: SettingsStore.widget(rowScope.rowId).enabled ? "Enabled" : "Disabled"
                                font.pixelSize: Theme.fontBody
                                color: SettingsStore.widget(rowScope.rowId).enabled ? Theme.sidebarActive : Theme.textMuted
                                Layout.alignment: Qt.AlignVCenter
                            }
                            Text {
                                text: "⇅"
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
                            InputMode.pointerMoved(p.x, p.y);
                        }
                        onClicked: {
                            rowScope.forceActiveFocus();
                            root._toggle(rowScope.rowId);
                        }
                    }
                }
            }

            Item {
                Layout.fillHeight: true
                Layout.minimumHeight: Units.spacingLG
            }

            HintBar {
                Layout.fillWidth: true
                text: "A: Enable/Disable    X: Configure    ←→: Reorder    B: Back"
            }
        }

        // ===================== L1 — per-widget config =====================
        ColumnLayout {
            id: l1Col
            visible: !root._atList
            width: scrollView.width
            spacing: Units.spacingLG

            Text {
                text: {
                    var m = WidgetManifests.byId(root._activeWidget);
                    return m ? m.name : root._activeWidget;
                }
                font.pixelSize: Theme.fontTitle
                font.bold: true
                color: Theme.textPrimary
            }

            // Generic controls generated from the manifest schema (+ a leading
            // synthetic "enabled" toggle).
            Repeater {
                id: l1Repeater
                model: root._atList ? [] : root._l1Schema(root._activeWidget)

                delegate: FocusScope {
                    id: ctl
                    required property int index
                    required property var modelData
                    readonly property var entry: modelData
                    readonly property string wid: root._activeWidget

                    Layout.fillWidth: true
                    implicitHeight: ctlRow.implicitHeight

                    Keys.onUpPressed: root._l1Up(ctl.index)
                    Keys.onDownPressed: root._l1Down(ctl.index)
                    onActiveFocusChanged: if (activeFocus)
                        scrollView.ensureVisible(ctl)

                    RowLayout {
                        id: ctlRow
                        anchors.left: parent.left
                        anchors.right: parent.right
                        spacing: Units.spacingLG

                        Text {
                            text: ctl.entry.label || ctl.entry.key
                            font.pixelSize: Theme.fontBody
                            color: Theme.textSecondary
                            Layout.alignment: Qt.AlignVCenter
                            Layout.preferredWidth: Units.gridUnit * 8
                        }

                        // --- enabled toggle (synthetic) ---
                        FocusButton {
                            visible: ctl.entry.type === "__enabled"
                            focus: ctl.entry.type === "__enabled"
                            Layout.alignment: Qt.AlignVCenter
                            text: SettingsStore.widget(ctl.wid).enabled ? "Enabled" : "Disabled"
                            fillActive: SettingsStore.widget(ctl.wid).enabled
                            fillColor: Theme.sidebarActive
                            onActivated: SettingsStore.setWidget(ctl.wid, "enabled", !SettingsStore.widget(ctl.wid).enabled)
                        }

                        // --- enum (size or pref) → chip selector ---
                        SettingsButtonGroup {
                            visible: ctl.entry.type === "enum"
                            focus: ctl.entry.type === "enum"
                            Layout.alignment: Qt.AlignVCenter
                            options: root._optionsFor(ctl.entry)
                            isCurrentOption: opt => {
                                if (ctl.entry.key === "size")
                                    return opt.value === SettingsStore.widget(ctl.wid).size;
                                return opt.value === SettingsStore.widget(ctl.wid).prefs[ctl.entry.key];
                            }
                            onValueSelected: opt => {
                                if (ctl.entry.key === "size")
                                    SettingsStore.setWidget(ctl.wid, "size", opt.value);
                                else
                                    SettingsStore.setWidgetPref(ctl.wid, ctl.entry.key, opt.value);
                            }
                        }

                        // --- bool pref → on/off toggle ---
                        FocusButton {
                            visible: ctl.entry.type === "bool"
                            focus: ctl.entry.type === "bool"
                            Layout.alignment: Qt.AlignVCenter
                            text: SettingsStore.widget(ctl.wid).prefs[ctl.entry.key] ? "On" : "Off"
                            fillActive: SettingsStore.widget(ctl.wid).prefs[ctl.entry.key] === true
                            fillColor: Theme.sidebarActive
                            onActivated: SettingsStore.setWidgetPref(ctl.wid, ctl.entry.key, !SettingsStore.widget(ctl.wid).prefs[ctl.entry.key])
                        }

                        // --- int pref → minimal −/+ stepper ---
                        RowLayout {
                            visible: ctl.entry.type === "int"
                            Layout.alignment: Qt.AlignVCenter
                            spacing: Units.spacingMD
                            FocusButton {
                                focus: ctl.entry.type === "int"
                                text: "−"
                                onActivated: SettingsStore.setWidgetPref(ctl.wid, ctl.entry.key, (SettingsStore.widget(ctl.wid).prefs[ctl.entry.key] || 0) - 1)
                            }
                            Text {
                                text: String(SettingsStore.widget(ctl.wid).prefs[ctl.entry.key] || 0)
                                font.pixelSize: Theme.fontBody
                                color: Theme.textPrimary
                                Layout.alignment: Qt.AlignVCenter
                            }
                            FocusButton {
                                text: "+"
                                onActivated: SettingsStore.setWidgetPref(ctl.wid, ctl.entry.key, (SettingsStore.widget(ctl.wid).prefs[ctl.entry.key] || 0) + 1)
                            }
                        }

                        // --- string pref / unknown → read-only value (won't crash) ---
                        Text {
                            visible: ctl.entry.type === "string" || (ctl.entry.type !== "__enabled" && ctl.entry.type !== "enum" && ctl.entry.type !== "bool" && ctl.entry.type !== "int")
                            text: String(SettingsStore.widget(ctl.wid).prefs[ctl.entry.key] || "")
                            font.pixelSize: Theme.fontBody
                            color: Theme.textMuted
                            Layout.alignment: Qt.AlignVCenter
                        }

                        Item {
                            Layout.fillWidth: true
                        }
                    }
                }
            }

            // Moonlight special-case: inline the full server-management surface
            // below the Size control (preserves the Moonlight server deep-link).
            // Loaded only for Moonlight's config page; its server actions are
            // self-contained (MoonlightSettings exposes no signals to wire).
            Loader {
                id: moonlightServersLoader
                Layout.fillWidth: true
                active: !root._atList && root._activeWidget === "moonlight"
                visible: active
                sourceComponent: moonlightServersComp
                onLoaded: {
                    if (item)
                        item.upTarget = Qt.binding(function () {
                            return l1Repeater.count > 0 ? l1Repeater.itemAt(l1Repeater.count - 1) : null;
                        });
                }
            }

            Item {
                Layout.fillHeight: true
                Layout.minimumHeight: Units.spacingLG
            }

            HintBar {
                Layout.fillWidth: true
                text: "B: Back to Widgets"
            }
        }
    }

    Component {
        id: moonlightServersComp
        MoonlightSettings {
            embedded: true
        }
    }

    // Follow keyboard/controller focus inside the embedded Moonlight surface so
    // its server rows scroll into view (mirrors SettingsApp's scroll-follow).
    property Item _afItem: Window.activeFocusItem
    on_AfItemChanged: if (root._afItem && !root._atList)
        scrollView.ensureVisible(root._afItem)
}
