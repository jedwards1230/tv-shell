import QtQuick
import QtQuick.Layouts
import "lib"

// Accessibility settings page (#109, #110).
// Controller-first: A/Return toggles Reduce Motion; A/Return on the Text Size
// selector opens the dropdown (same open-on-A rule as audio output dropdown).
// Left/B from here returns focus to the SettingsPanel sidebar.
FocusScope {
    id: root
    implicitHeight: a11yMainCol.implicitHeight + 2 * Theme.padding

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

    ColumnLayout {
        id: a11yMainCol
        anchors.fill: parent
        anchors.margins: Theme.padding
        spacing: 48

        // === Reduce Motion ===
        Text {
            text: "Reduce Motion"
            font.pixelSize: Theme.fontBody
            font.bold: true
            color: Theme.textPrimary
        }

        RowLayout {
            Layout.fillWidth: true
            spacing: 24

            ColumnLayout {
                spacing: 4
                Layout.fillWidth: true

                Text {
                    text: "Suppress animations and scrolling text"
                    font.pixelSize: Theme.fontBody
                    color: Theme.textPrimary
                    wrapMode: Text.WordWrap
                    Layout.fillWidth: true
                }

                Text {
                    text: "Focus ring, glow, and fill remain active — only scale/transition animations stop."
                    font.pixelSize: Theme.fontSmall
                    color: Theme.textSecondary
                    wrapMode: Text.WordWrap
                    Layout.fillWidth: true
                }
            }

            FocusScope {
                id: reduceMotionScope
                width: reduceMotionBtn.width
                height: reduceMotionBtn.height
                activeFocusOnTab: true

                KeyNavigation.down: textSizeScope

                SettingsButton {
                    id: reduceMotionBtn
                    text: Theme.reduceMotion ? "Enabled" : "Disabled"
                    focus: parent.activeFocus
                    anchors.fill: parent
                    color: Theme.reduceMotion ? Theme.sidebarActive : (parent.activeFocus ? Theme.surfaceHover : Theme.surface)

                    onActivated: Theme.setReduceMotion(!Theme.reduceMotion)

                    MouseArea {
                        anchors.fill: parent
                        hoverEnabled: true
                        cursorShape: Qt.PointingHandCursor
                        onClicked: {
                            reduceMotionScope.forceActiveFocus();
                            reduceMotionBtn.activated();
                        }
                    }
                }
            }
        }

        // Divider
        Rectangle {
            Layout.fillWidth: true
            height: 1
            color: Theme.surfaceBorder
        }

        // === Text Size ===
        Text {
            text: "Text Size"
            font.pixelSize: Theme.fontBody
            font.bold: true
            color: Theme.textPrimary
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
                Theme.setTextScale(item.id);
            }

            KeyNavigation.up: reduceMotionScope
        }

        Item {
            Layout.fillHeight: true
        }

        HintBar {
            text: "Text Scale: " + (Theme.textScale === 1.0 ? "Default" : Theme.textScale === 1.15 ? "Large" : "Larger")
        }
    }
}
