import QtQuick
import QtQuick.Layouts
import "lib"
import "../components"
import "../components/lib"

// L0 — the widget list (level 0 of the Widgets app). One row per widget, in
// live `order`. Owns the order model, the reorder/toggle/configure key handling,
// and its own scroll. It is a leaf view: the parent WidgetsApp owns the back-stack
// (list ↔ config) and the B/Escape → close path; this component never consumes
// B/Escape (so it bubbles to the app root).
//
// Key map (A/X swapped vs the pre-refactor surface — #281):
//   A (Return/Enter) → open this widget's config (drill into L1)
//   X                → toggle the widget enabled/disabled in place
//   ←/→              → reorder (plasma-bigscreen style, persisted), focus follows
//   Up/Down          → move between rows
FocusScope {
    id: root

    // Emitted when the user drills into a widget's config (A / click).
    signal configureRequested(string id)
    // Any navigation — lets the app reset the auto-suspend idle timer.
    signal userActivity

    // Remember which row we drilled in from so returning focuses it.
    property string _lastListId: ""
    // Set on a reorder so the moved row regains focus once the list rebuilds.
    property string _pendingFocusId: ""

    // === Order model ===
    // Widget ids sorted by live order. Guarded so it reassigns ONLY when the
    // actual sequence changes — an enable/size toggle reassigns SettingsStore.widgets
    // but leaves order untouched, so the Repeater is NOT rebuilt (focus kept).
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
        if (root._pendingFocusId !== "") {
            var id = root._pendingFocusId;
            root._pendingFocusId = "";
            Qt.callLater(function () {
                root._focusRow(id);
            });
        }
    }

    Component.onCompleted: root._recomputeOrder()

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

    // === Public API (called by WidgetsApp) ===
    // Land focus on the list: the last-drilled row if still present, else the first.
    function focusEntry() {
        if (root._orderedIds.length === 0)
            return;
        var want = (root._lastListId !== "" && root._orderedIds.indexOf(root._lastListId) >= 0) ? root._lastListId : root._orderedIds[0];
        root._focusRow(want);
    }
    // Record a deep-link drill-in so B from that config lands back on the row.
    function noteDrillIn(id) {
        root._lastListId = id;
    }

    // === Row helpers ===
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
    function _configure(id) {
        root._lastListId = id;
        root.configureRequested(id);
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

    Flickable {
        id: listFlick
        anchors.fill: parent
        clip: true
        interactive: true
        contentWidth: width
        contentHeight: l0Col.implicitHeight
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
            var p = it.mapToItem(listFlick.contentItem, 0, 0);
            var maxY = Math.max(0, listFlick.contentHeight - listFlick.height);
            if (p.y < listFlick.contentY)
                listFlick.contentY = Math.max(0, p.y - 24);
            else if (p.y + it.height > listFlick.contentY + listFlick.height)
                listFlick.contentY = (p.y >= maxY) ? maxY : Math.min(p.y + it.height - listFlick.height + 24, maxY);
        }

        ColumnLayout {
            id: l0Col
            width: listFlick.width
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

                    // A (Return/Enter) → configure; X → toggle (swapped — #281).
                    Keys.onReturnPressed: root._configure(rowScope.rowId)
                    Keys.onEnterPressed: root._configure(rowScope.rowId)
                    Keys.onUpPressed: root._focusSibling(rowScope.index, -1)
                    Keys.onDownPressed: root._focusSibling(rowScope.index, 1)
                    Keys.onLeftPressed: root._reorder(rowScope.rowId, -1)
                    Keys.onRightPressed: root._reorder(rowScope.rowId, 1)
                    Keys.onPressed: event => {
                        if (event.key === Qt.Key_X && !event.modifiers) {
                            root._toggle(rowScope.rowId);
                            event.accepted = true;
                        }
                    }

                    onActiveFocusChanged: if (activeFocus)
                        listFlick.ensureVisible(rowScope)

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
                        // Primary activate = configure (matches A).
                        onClicked: {
                            rowScope.forceActiveFocus();
                            root._configure(rowScope.rowId);
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
                text: "A: Configure    X: Enable/Disable    ←→: Reorder    B: Back"
            }
        }
    }
}
