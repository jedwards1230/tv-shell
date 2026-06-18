import QtQuick
import QtQuick.Layouts
import "lib"

// Accessibility settings page (#109, #110).
// Controller-first: A/Return toggles Reduce Motion; A/Return on the Text Size
// selector opens the dropdown (same open-on-A rule as audio output dropdown).
// Left/B from here returns focus to the SettingsPanel sidebar.
//
// Reference migration to SettingsPageBase (the page scaffold lives there now —
// padded content column, trailing spacer, and the bottom HintBar via hintText).
SettingsPageBase {
    id: root
    hintText: "Text Scale: " + (Theme.textScale === 1.0 ? "Default" : Theme.textScale === 1.15 ? "Large" : "Larger")

    readonly property var textSizeOptions: [
        {
            id: 1.0,
            label: "Default",
            desc: "Standard couch-readable size"
        },
        {
            id: 1.15,
            label: "Large",
            desc: "~15% larger text"
        },
        {
            id: 1.3,
            label: "Larger",
            desc: "~30% larger text"
        }
    ]

    function focusFirst() {
        reduceMotionScope.forceActiveFocus();
    }

    // === Reduce Motion ===
    SectionHeader {
        text: "Reduce Motion"
    }

    PreferenceRow {
        label: "Suppress animations and scrolling text"
        description: "Focus ring, glow, and fill remain active — only scale/transition animations stop."

        FocusButton {
            id: reduceMotionScope
            KeyNavigation.down: textSizeScope
            text: Theme.reduceMotion ? "Enabled" : "Disabled"
            fillActive: Theme.reduceMotion
            fillColor: Theme.sidebarActive
            onActivated: SettingsStore.setReduceMotion(!SettingsStore.reduceMotion)
        }
    }

    // Divider
    Rectangle {
        Layout.fillWidth: true
        height: 1
        color: Theme.surfaceBorder
    }

    // === Text Size ===
    SectionHeader {
        text: "Text Size"
    }

    SettingsDropdown {
        id: textSizeScope
        model: root.textSizeOptions
        displayText: {
            for (var i = 0; i < root.textSizeOptions.length; i++) {
                if (Math.abs(root.textSizeOptions[i].id - Theme.textScale) < 0.01)
                    return root.textSizeOptions[i].label;
            }
            return "Default";
        }
        headerHeight: 88
        rowHeight: 76
        isCurrentItem: function (item) {
            return Math.abs(item.id - Theme.textScale) < 0.01;
        }
        itemLabel: function (item) {
            return item.label;
        }
        onItemSelected: function (item) {
            SettingsStore.setTextScale(item.id);
        }

        KeyNavigation.up: reduceMotionScope
    }
}
