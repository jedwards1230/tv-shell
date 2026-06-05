import QtQuick
import QtQuick.Layouts

// Accessibility settings page (#109, #110).
// Controller-first: A/Return toggles Reduce Motion; A/Return on the Text Size
// selector opens the dropdown (same open-on-A rule as audio output dropdown).
// Left/B from here returns focus to the SettingsPanel sidebar.
FocusScope {
    id: root

    readonly property var textSizeOptions: [
        {id: 1.0, label: "Default", desc: "Standard couch-readable size"},
        {id: 1.15, label: "Large", desc: "~15% larger text"},
        {id: 1.3, label: "Larger", desc: "~30% larger text"}
    ]

    function focusFirst() {
        reduceMotionScope.forceActiveFocus();
    }

    ColumnLayout {
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

                    MouseArea {
                        anchors.fill: parent
                        hoverEnabled: true
                        cursorShape: Qt.PointingHandCursor
                        onClicked: {
                            reduceMotionScope.forceActiveFocus();
                            Theme.setReduceMotion(!Theme.reduceMotion);
                        }
                    }
                }

                Keys.onReturnPressed: {
                    Theme.setReduceMotion(!Theme.reduceMotion);
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

        FocusScope {
            id: textSizeScope
            Layout.fillWidth: true
            Layout.preferredHeight: _dropOpen
                ? Math.min(root.textSizeOptions.length * 80 + 88, 400)
                : 88

            property bool _dropOpen: false
            property string currentLabel: {
                for (var i = 0; i < root.textSizeOptions.length; i++) {
                    if (Math.abs(root.textSizeOptions[i].id - Theme.textScale) < 0.01)
                        return root.textSizeOptions[i].label;
                }
                return "Default";
            }

            KeyNavigation.up: reduceMotionScope

            Behavior on Layout.preferredHeight {
                NumberAnimation {
                    duration: Theme.reduceMotion ? 0 : 200
                    easing.type: Easing.OutCubic
                }
            }

            // Dropdown header
            Rectangle {
                id: textSizeHeader
                width: parent.width
                height: 88
                radius: 16
                color: textSizeScope.activeFocus && !textSizeScope._dropOpen
                    ? Theme.surfaceHover : Theme.surface
                border.width: 2
                border.color: textSizeScope.activeFocus ? Theme.focusBorder : Theme.surfaceBorder

                Behavior on color {
                    ColorAnimation {
                        duration: Theme.reduceMotion ? 0 : 150
                    }
                }
                Behavior on border.color {
                    ColorAnimation {
                        duration: Theme.reduceMotion ? 0 : 150
                    }
                }

                RowLayout {
                    anchors.fill: parent
                    anchors.leftMargin: 24
                    anchors.rightMargin: 24
                    spacing: 16

                    Text {
                        text: textSizeScope.currentLabel
                        font.pixelSize: Theme.fontSmall
                        color: Theme.textPrimary
                        Layout.fillWidth: true
                    }

                    Text {
                        text: "(current)"
                        font.pixelSize: Theme.fontHint
                        color: Theme.textMuted
                    }

                    Text {
                        text: textSizeScope._dropOpen ? "▲" : "▼"
                        font.pixelSize: Theme.fontSmall
                        color: Theme.textSecondary
                    }
                }

                MouseArea {
                    anchors.fill: parent
                    cursorShape: Qt.PointingHandCursor
                    onClicked: {
                        textSizeScope.forceActiveFocus();
                        textSizeScope._dropOpen = !textSizeScope._dropOpen;
                    }
                }
            }

            // Dropdown list
            ListView {
                id: textSizeList
                anchors.top: textSizeHeader.bottom
                anchors.topMargin: 8
                width: parent.width
                height: parent.height - textSizeHeader.height - 8
                spacing: 4
                clip: true
                visible: textSizeScope._dropOpen
                model: root.textSizeOptions
                keyNavigationEnabled: true
                highlightFollowsCurrentItem: true
                highlightMoveDuration: Theme.reduceMotion ? 0 : 100

                delegate: Rectangle {
                    required property int index
                    required property var modelData
                    width: textSizeList.width
                    height: 76
                    radius: 12

                    property bool isCurrent: Math.abs(modelData.id - Theme.textScale) < 0.01

                    color: {
                        if (isCurrent)
                            return Theme.sidebarActive;
                        if (textSizeList.currentIndex === index && textSizeList.activeFocus)
                            return Theme.surfaceHover;
                        return Theme.cardBackground;
                    }
                    border.width: isCurrent ? 2 : 1
                    border.color: isCurrent ? Theme.focusBorder : Theme.surfaceBorder

                    Behavior on color {
                        ColorAnimation {
                            duration: Theme.reduceMotion ? 0 : 150
                        }
                    }

                    ColumnLayout {
                        anchors.centerIn: parent
                        spacing: 2

                        Text {
                            text: modelData.label + (isCurrent ? "  ✓" : "")
                            font.pixelSize: Theme.fontSmall
                            font.bold: isCurrent
                            color: isCurrent ? Theme.textOnDark : Theme.textPrimary
                            Layout.alignment: Qt.AlignHCenter
                        }

                        Text {
                            text: modelData.desc
                            font.pixelSize: Theme.fontHint
                            color: isCurrent ? Theme.textOnDarkMuted : Theme.textSecondary
                            Layout.alignment: Qt.AlignHCenter
                        }
                    }

                    MouseArea {
                        anchors.fill: parent
                        cursorShape: Qt.PointingHandCursor
                        onClicked: {
                            textSizeList.currentIndex = index;
                            textSizeList.forceActiveFocus();
                            Theme.setTextScale(modelData.id);
                        }
                    }
                }

                Keys.onReturnPressed: {
                    if (currentIndex >= 0 && currentIndex < root.textSizeOptions.length) {
                        Theme.setTextScale(root.textSizeOptions[currentIndex].id);
                        textSizeScope._dropOpen = false;
                        textSizeScope.forceActiveFocus();
                    }
                }

                Keys.onEscapePressed: {
                    textSizeScope._dropOpen = false;
                    textSizeScope.forceActiveFocus();
                }
            }

            // Open-on-A only (never on a directional key)
            Keys.onReturnPressed: {
                if (!_dropOpen) {
                    _dropOpen = true;
                    // pre-select the current scale in the list
                    for (var i = 0; i < root.textSizeOptions.length; i++) {
                        if (Math.abs(root.textSizeOptions[i].id - Theme.textScale) < 0.01) {
                            textSizeList.currentIndex = i;
                            break;
                        }
                    }
                    textSizeList.forceActiveFocus();
                }
            }

            Keys.onEscapePressed: {
                if (_dropOpen) {
                    _dropOpen = false;
                } else {
                    event.accepted = false;
                }
            }
        }

        Item {
            Layout.fillHeight: true
        }

        Text {
            text: "Text Scale: " + (Theme.textScale === 1.0 ? "Default" : Theme.textScale === 1.15 ? "Large" : "Larger")
            font.pixelSize: Theme.fontHint
            color: Theme.textSecondary
            Layout.alignment: Qt.AlignHCenter
        }
    }
}
