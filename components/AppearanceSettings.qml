import QtQuick
import QtQuick.Layouts

FocusScope {
    id: root

    property var modes: [
        {
            id: "auto",
            icon: "◐",
            label: "Auto",
            desc: "Follows time of day"
        },
        {
            id: "light",
            icon: "☀",
            label: "Light",
            desc: "Light background"
        },
        {
            id: "dark",
            icon: "☽",
            label: "Dark",
            desc: "OLED optimized"
        }
    ]

    // Focus entry is driven by SettingsPanel via focusFirst() on Right; start
    // the cursor on the currently applied mode rather than always index 0.
    function focusFirst() {
        modeList.focusIndex = modeList.currentIndex;
        modeList.forceActiveFocus();
    }

    ColumnLayout {
        anchors.fill: parent
        anchors.margins: Theme.padding
        spacing: 48

        Text {
            text: "Theme Mode"
            font.pixelSize: Theme.fontBody
            font.bold: true
            color: Theme.textPrimary
        }

        Item {
            Layout.fillHeight: true
            Layout.maximumHeight: 80
        }

        // Mode cards
        RowLayout {
            id: modeList
            Layout.alignment: Qt.AlignHCenter
            spacing: 40
            focus: true

            property int currentIndex: {
                for (var i = 0; i < root.modes.length; i++) {
                    if (root.modes[i].id === Theme.themeMode)
                        return i;
                }
                return 0;
            }

            property int focusIndex: 0

            Keys.onLeftPressed: event => {
                // At the leftmost card, let Left bubble to SettingsPanel so it
                // returns focus to the sidebar instead of being swallowed.
                if (focusIndex > 0)
                    focusIndex--;
                else
                    event.accepted = false;
            }
            Keys.onRightPressed: {
                if (focusIndex < root.modes.length - 1)
                    focusIndex++;
            }
            Keys.onReturnPressed: {
                Theme.setThemeMode(root.modes[focusIndex].id);
            }

            Repeater {
                model: root.modes

                Rectangle {
                    required property var modelData
                    required property int index
                    width: 400
                    height: 280
                    radius: Theme.cardRadius
                    color: Theme.surface
                    clip: true
                    // Focus cursor (crimson ring) is independent of the applied
                    // mode (green "Active" badge) so both are visible at once —
                    // even when the cursor sits on the applied card.
                    border.width: modeList.focusIndex === index && modeList.activeFocus ? 4 : 2
                    border.color: modeList.focusIndex === index && modeList.activeFocus ? Theme.focusBorder : Theme.surfaceBorder

                    Behavior on border.color {
                        ColorAnimation {
                            duration: 150
                        }
                    }

                    // Focus highlight background — shown wherever the cursor is.
                    Rectangle {
                        anchors.fill: parent
                        radius: parent.radius
                        color: Theme.surfaceHover
                        visible: modeList.focusIndex === index && modeList.activeFocus
                    }

                    // Applied-mode badge — persistent, independent of focus.
                    Rectangle {
                        anchors.top: parent.top
                        anchors.right: parent.right
                        anchors.margins: 16
                        width: appliedLabel.implicitWidth + 28
                        height: appliedLabel.implicitHeight + 14
                        radius: height / 2
                        color: Theme.online
                        visible: Theme.themeMode === modelData.id
                        z: 1

                        Text {
                            id: appliedLabel
                            anchors.centerIn: parent
                            text: "✓ Active"
                            font.pixelSize: Theme.fontCaption
                            font.bold: true
                            color: Theme.textOnDark
                        }
                    }

                    ColumnLayout {
                        anchors.fill: parent
                        anchors.margins: 32
                        spacing: 12

                        Item {
                            Layout.fillHeight: true
                        }

                        Text {
                            text: modelData.icon
                            font.pixelSize: Theme.fontTitle
                            color: Theme.themeMode === modelData.id ? Theme.online : Theme.textPrimary
                            Layout.alignment: Qt.AlignHCenter
                        }

                        Text {
                            text: modelData.label
                            font.pixelSize: Theme.fontBody
                            font.bold: true
                            color: Theme.textPrimary
                            Layout.alignment: Qt.AlignHCenter
                        }

                        Text {
                            text: modelData.desc
                            font.pixelSize: Theme.fontSmall
                            color: Theme.textSecondary
                            Layout.alignment: Qt.AlignHCenter
                            Layout.maximumWidth: parent.width
                            horizontalAlignment: Text.AlignHCenter
                            wrapMode: Text.WordWrap
                        }

                        Item {
                            Layout.fillHeight: true
                        }
                    }

                    MouseArea {
                        anchors.fill: parent
                        hoverEnabled: true
                        cursorShape: Qt.PointingHandCursor
                        onClicked: {
                            modeList.focusIndex = index;
                            modeList.forceActiveFocus();
                            Theme.setThemeMode(modelData.id);
                        }
                    }
                }
            }
        }

        Item {
            Layout.fillHeight: true
        }

        Text {
            text: "Current: " + Theme.themeMode.charAt(0).toUpperCase() + Theme.themeMode.slice(1)
            font.pixelSize: Theme.fontHint
            color: Theme.textSecondary
            Layout.alignment: Qt.AlignHCenter
        }
    }
}
