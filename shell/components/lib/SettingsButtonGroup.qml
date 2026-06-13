import QtQuick
import QtQuick.Layouts
import "../"

// Horizontal row of chip buttons where Left/Right moves focus and Return
// commits the highlighted choice.  Replaces the scale, overscan, and
// auto-dim-delay rows which all share the exact same FocusScope + Repeater
// + focusedIndex pattern.
//
// Usage:
//   SettingsButtonGroup {
//       options: [{label: "1x", value: 1.0}, ...]
//       currentValue: someValue
//       isCurrentOption: function(opt) { return opt.value === someValue }
//       onValueSelected: function(opt) { doSomething(opt.value) }
//       enabled: true   // set false to dim the whole row
//       KeyNavigation.up: above
//       KeyNavigation.down: below
//   }
//
// `options` items must have at least a `label` field; any extra fields are
// passed through to onValueSelected for the caller to use.

FocusScope {
    id: root

    // --- Public API ---
    property var options: []

    // Called with each option: return true if it matches the current value.
    property var isCurrentOption: function (opt) {
        return false;
    }

    // Emitted when the user confirms (Return or mouse click).
    signal valueSelected(var option)

    // When false, the row is visually dimmed and key-presses are no-ops.
    property bool enabled: true

    // --- Internal ---
    property int focusedIndex: {
        for (var i = 0; i < options.length; i++) {
            if (root.isCurrentOption(options[i]))
                return i;
        }
        return 0;
    }

    Layout.fillWidth: true
    Layout.preferredHeight: 96
    opacity: root.enabled ? 1.0 : 0.4

    Keys.onLeftPressed: {
        if (root.enabled && focusedIndex > 0)
            focusedIndex--;
    }
    Keys.onRightPressed: {
        if (root.enabled && focusedIndex < options.length - 1)
            focusedIndex++;
    }
    Keys.onReturnPressed: {
        if (root.enabled)
            root.valueSelected(options[focusedIndex]);
    }

    RowLayout {
        anchors.fill: parent
        spacing: 16

        Repeater {
            model: root.options

            FocusScope {
                id: optScope
                required property var modelData
                required property int index
                width: optBtn.implicitWidth
                height: optBtn.implicitHeight

                SettingsButton {
                    id: optBtn
                    text: optScope.modelData.label
                    anchors.fill: parent

                    property bool isCurrent: root.isCurrentOption(optScope.modelData)
                    property bool isFocused: root.activeFocus && root.focusedIndex === optScope.index

                    color: isCurrent ? Theme.sidebarActive : isFocused ? Theme.surfaceHover : Theme.surface
                    border.width: isFocused ? 2 : 1
                    border.color: isFocused ? Theme.focusBorder : Theme.surfaceBorder

                    onActivated: {
                        if (root.enabled)
                            root.valueSelected(optScope.modelData);
                    }

                    MouseArea {
                        anchors.fill: parent
                        hoverEnabled: true
                        cursorShape: Qt.PointingHandCursor
                        onClicked: {
                            root.forceActiveFocus();
                            root.focusedIndex = optScope.index;
                            optBtn.activated();
                        }
                    }
                }
            }
        }
    }
}
