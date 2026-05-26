import QtQuick
import QtQuick.Layouts

FocusScope {
    id: root

    property var bindings: [
        { action: "select",     label: "Select / Confirm",  keys: ["A"],           category: "Navigation", description: "Confirm selection or activate" },
        { action: "back",       label: "Back / Cancel",      keys: ["B"],           category: "Navigation", description: "Go back or cancel" },
        { action: "altSelect",  label: "Alt Select",         keys: ["Y"],           category: "Navigation", description: "Tab navigation" },
        { action: "confirm",    label: "Confirm",            keys: ["Start"],       category: "Navigation", description: "Confirm / enter" },
        { action: "up",         label: "Navigate Up",        keys: ["D-Pad Up"],    category: "Navigation", description: "Move focus up" },
        { action: "down",       label: "Navigate Down",      keys: ["D-Pad Down"],  category: "Navigation", description: "Move focus down" },
        { action: "left",       label: "Navigate Left",      keys: ["D-Pad Left"],  category: "Navigation", description: "Move focus left" },
        { action: "right",      label: "Navigate Right",     keys: ["D-Pad Right"], category: "Navigation", description: "Move focus right" },
        { action: "stickUp",    label: "Stick Up",           keys: ["Left Stick ↑"], category: "Navigation", description: "Move focus up" },
        { action: "stickDown",  label: "Stick Down",         keys: ["Left Stick ↓"], category: "Navigation", description: "Move focus down" },
        { action: "stickLeft",  label: "Stick Left",         keys: ["Left Stick ←"], category: "Navigation", description: "Move focus left" },
        { action: "stickRight", label: "Stick Right",        keys: ["Left Stick →"], category: "Navigation", description: "Move focus right" },
        { action: "drawer",     label: "Open Drawer / Wake AV", keys: ["Home"],     category: "System",     description: "Toggle navigation drawer; wakes AV if system is off" },
        { action: "forceQuit",  label: "Force Quit",         keys: ["Back", "Home", "LB", "RB"], category: "System", description: "Instantly kill foreground app" },
        { action: "endSession", label: "End Session",        keys: ["B", "Home"],   category: "System",     description: "Hold 3 seconds to end game session" }
    ]

    property var navigationBindings: bindings.filter(function(b) { return b.category === "Navigation" })
    property var systemBindings: bindings.filter(function(b) { return b.category === "System" })

    onVisibleChanged: {
        if (visible) {
            bindingsList.forceActiveFocus()
        }
    }

    ColumnLayout {
        anchors.fill: parent
        anchors.margins: Theme.padding
        spacing: 32

        // Navigation section header
        Text {
            text: "Navigation"
            font.pixelSize: Theme.fontBody
            font.bold: true
            color: Theme.textPrimary
        }

        // Navigation bindings list
        ListView {
            id: bindingsList
            Layout.fillWidth: true
            Layout.preferredHeight: Math.min(root.navigationBindings.length * 88, 1060)
            spacing: 8
            clip: true
            model: root.navigationBindings
            focus: true
            interactive: false

            KeyNavigation.down: systemList

            delegate: Rectangle {
                required property int index
                required property var modelData
                width: bindingsList.width
                height: 80
                radius: 16
                color: bindingsList.currentIndex === index && bindingsList.activeFocus
                       ? Theme.surfaceHover : Theme.surface
                border.width: 2
                border.color: Theme.surfaceBorder

                Behavior on color { ColorAnimation { duration: 150 } }

                RowLayout {
                    anchors.fill: parent
                    anchors.leftMargin: 24
                    anchors.rightMargin: 24
                    spacing: 16

                    // Action label
                    Text {
                        text: modelData.label
                        font.pixelSize: Theme.fontSmall
                        color: Theme.textPrimary
                        Layout.fillWidth: true
                    }

                    // Key cap badges
                    Row {
                        spacing: 8

                        Repeater {
                            model: modelData.keys

                            Row {
                                spacing: 8

                                // "+" separator for combo keys (after first key)
                                Text {
                                    text: "+"
                                    font.pixelSize: Theme.fontSmall
                                    color: Theme.textMuted
                                    visible: index > 0
                                    anchors.verticalCenter: parent.verticalCenter
                                }

                                Rectangle {
                                    width: keyText.implicitWidth + 24
                                    height: keyText.implicitHeight + 16
                                    radius: 8
                                    color: Theme.surface
                                    border.width: 2
                                    border.color: Theme.ember

                                    Text {
                                        id: keyText
                                        anchors.centerIn: parent
                                        text: modelData
                                        font.pixelSize: Theme.fontSmall
                                        color: Theme.textPrimary
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        // System section header
        Text {
            text: "System"
            font.pixelSize: Theme.fontBody
            font.bold: true
            color: Theme.textPrimary
        }

        // System bindings list
        ListView {
            id: systemList
            Layout.fillWidth: true
            Layout.preferredHeight: Math.min(root.systemBindings.length * 108, 400)
            spacing: 8
            clip: true
            model: root.systemBindings
            interactive: false

            KeyNavigation.up: bindingsList

            delegate: Rectangle {
                required property int index
                required property var modelData
                width: systemList.width
                height: modelData.action === "drawer" ? 100 : 80
                radius: 16
                color: systemList.currentIndex === index && systemList.activeFocus
                       ? Theme.surfaceHover : Theme.surface
                border.width: 2
                border.color: Theme.surfaceBorder

                Behavior on color { ColorAnimation { duration: 150 } }

                RowLayout {
                    anchors.fill: parent
                    anchors.leftMargin: 24
                    anchors.rightMargin: 24
                    spacing: 16

                    // Action label + optional subtitle
                    ColumnLayout {
                        Layout.fillWidth: true
                        spacing: 4

                        Text {
                            text: modelData.label
                            font.pixelSize: Theme.fontSmall
                            color: Theme.textPrimary
                        }

                        Text {
                            text: "Also wakes AV system from standby"
                            font.pixelSize: Theme.fontHint
                            color: Theme.textMuted
                            visible: modelData.action === "drawer"
                        }
                    }

                    // Key cap badges
                    Row {
                        spacing: 8

                        Repeater {
                            model: modelData.keys

                            Row {
                                spacing: 8

                                // "+" separator for combo keys (after first key)
                                Text {
                                    text: "+"
                                    font.pixelSize: Theme.fontSmall
                                    color: Theme.textMuted
                                    visible: index > 0
                                    anchors.verticalCenter: parent.verticalCenter
                                }

                                Rectangle {
                                    width: sysKeyText.implicitWidth + 24
                                    height: sysKeyText.implicitHeight + 16
                                    radius: 8
                                    color: Theme.surface
                                    border.width: 2
                                    border.color: Theme.ember

                                    Text {
                                        id: sysKeyText
                                        anchors.centerIn: parent
                                        text: modelData
                                        font.pixelSize: Theme.fontSmall
                                        color: Theme.textPrimary
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        Item { Layout.fillHeight: true }

        Text {
            text: "Key bindings are read-only"
            font.pixelSize: Theme.fontHint
            color: Theme.textSecondary
            Layout.alignment: Qt.AlignHCenter
        }
    }
}
