import QtQuick
import QtQuick.Layouts
import QtQuick.Window
import "lib"
import "../components"
import "../components/lib"

// L1 — per-widget config (level 1 of the Widgets app). SCHEMA-DRIVEN from the
// widget's manifest: a synthetic "Enabled" toggle first, then one generic control
// per manifest `config` entry (size + any prefs). Moonlight additionally inlines
// the full server-management surface (MoonlightSettings) below its Size control.
//
// Leaf view: the parent WidgetsApp owns the back-stack and the B/Escape → list
// path; this component never consumes B/Escape (it bubbles to the app root). Owns
// its own scroll + focus-follow so the embedded Moonlight server rows reach view.
FocusScope {
    id: root

    // The widget whose config is shown; "" renders nothing (parent hides it).
    property string widgetId: ""
    readonly property bool _isMoonlight: root.widgetId === "moonlight"

    // Any navigation — lets the app reset the auto-suspend idle timer.
    signal userActivity

    // === Public API (called by WidgetsApp) ===
    function focusFirstControl() {
        if (l1Repeater.count > 0) {
            var it = l1Repeater.itemAt(0);
            if (it)
                it.forceActiveFocus();
        }
    }

    // Build the control list: a synthetic "enabled" entry first, then the
    // manifest's config entries (size + any prefs) in order.
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

    // === Focus chain (generic over the manifest control list) ===
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
        if (root._isMoonlight && moonlightServersLoader.item)
            moonlightServersLoader.item.forceActiveFocus();
    }

    Flickable {
        id: configFlick
        anchors.fill: parent
        clip: true
        interactive: true
        contentWidth: width
        contentHeight: l1Col.implicitHeight
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
            var p = it.mapToItem(configFlick.contentItem, 0, 0);
            var maxY = Math.max(0, configFlick.contentHeight - configFlick.height);
            if (p.y < configFlick.contentY)
                configFlick.contentY = Math.max(0, p.y - 24);
            else if (p.y + it.height > configFlick.contentY + configFlick.height)
                configFlick.contentY = (p.y >= maxY) ? maxY : Math.min(p.y + it.height - configFlick.height + 24, maxY);
        }

        ColumnLayout {
            id: l1Col
            width: configFlick.width
            spacing: Units.spacingLG

            Text {
                text: {
                    var m = WidgetManifests.byId(root.widgetId);
                    return m ? m.name : root.widgetId;
                }
                font.pixelSize: Theme.fontTitle
                font.bold: true
                color: Theme.textPrimary
            }

            // Generic controls generated from the manifest schema (+ a leading
            // synthetic "enabled" toggle).
            Repeater {
                id: l1Repeater
                model: root.widgetId === "" ? [] : root._l1Schema(root.widgetId)

                delegate: FocusScope {
                    id: ctl
                    required property int index
                    required property var modelData
                    readonly property var entry: modelData
                    readonly property string wid: root.widgetId

                    Layout.fillWidth: true
                    implicitHeight: ctlRow.implicitHeight

                    Keys.onUpPressed: root._l1Up(ctl.index)
                    Keys.onDownPressed: root._l1Down(ctl.index)
                    onActiveFocusChanged: if (activeFocus)
                        configFlick.ensureVisible(ctl)

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
            // Loaded only for Moonlight's config; its server actions are
            // self-contained (MoonlightSettings exposes no signals to wire).
            Loader {
                id: moonlightServersLoader
                Layout.fillWidth: true
                active: root._isMoonlight
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
    on_AfItemChanged: if (root._afItem && root.visible)
        configFlick.ensureVisible(root._afItem)
}
