import QtQuick
import QtQuick.Layouts

Rectangle {
    id: root
    color: Theme.background
    visible: false

    signal closed()

    property int currentSection: 0
    property var sections: [
        { name: "Audio",     icon: "Vol" },
        { name: "Bluetooth", icon: "BT"  },
        { name: "Network",   icon: "Net" },
        { name: "Display",   icon: "Scr" },
        { name: "Power",     icon: "Pwr" }
    ]

    onVisibleChanged: {
        if (visible) {
            currentSection = 0
            sidebarList.forceActiveFocus()
        }
    }

    RowLayout {
        anchors.fill: parent
        spacing: 0

        // Left sidebar
        Rectangle {
            Layout.fillHeight: true
            Layout.preferredWidth: 560
            color: Theme.surface

            ColumnLayout {
                anchors.fill: parent
                spacing: 0

                // Settings header
                Rectangle {
                    Layout.fillWidth: true
                    height: Theme.statusBarHeight
                    color: Theme.primary

                    Text {
                        anchors.centerIn: parent
                        text: "Settings"
                        font.pixelSize: Theme.fontTitle
                        font.bold: true
                        color: "#ffffff"
                    }
                }

                // Section list
                ListView {
                    id: sidebarList
                    Layout.fillWidth: true
                    Layout.fillHeight: true
                    model: root.sections
                    currentIndex: root.currentSection
                    focus: true
                    clip: true

                    delegate: Rectangle {
                        required property int index
                        required property var modelData
                        width: sidebarList.width
                        height: 140
                        color: {
                            if (root.currentSection === index)
                                return sidebarList.activeFocus ? Theme.accent : Theme.primary
                            if (sidebarList.currentIndex === index && sidebarList.activeFocus)
                                return Theme.surfaceHover
                            return "transparent"
                        }

                        Behavior on color { ColorAnimation { duration: 150 } }

                        RowLayout {
                            anchors.fill: parent
                            anchors.leftMargin: 40
                            anchors.rightMargin: 40
                            spacing: 28

                            // Active indicator bar
                            Rectangle {
                                width: 8
                                height: 72
                                radius: 4
                                color: root.currentSection === index ? "#ffffff" : "transparent"
                            }

                            Text {
                                text: modelData.icon
                                font.pixelSize: Theme.fontBody
                                font.bold: true
                                color: root.currentSection === index ? "#ffffff" : Theme.textDim
                                Layout.preferredWidth: 80
                            }

                            Text {
                                text: modelData.name
                                font.pixelSize: Theme.fontBody
                                font.bold: root.currentSection === index
                                color: root.currentSection === index ? "#ffffff" : Theme.text
                                Layout.fillWidth: true
                            }

                            Text {
                                text: root.currentSection === index ? ">" : ""
                                font.pixelSize: Theme.fontBody
                                color: "#ffffff"
                            }
                        }

                        MouseArea {
                            anchors.fill: parent
                            hoverEnabled: true
                            cursorShape: Qt.PointingHandCursor
                            onClicked: {
                                sidebarList.currentIndex = index
                                root.currentSection = index
                                sidebarList.forceActiveFocus()
                            }
                        }
                    }

                    Keys.onReturnPressed: {
                        root.currentSection = currentIndex
                        contentLoader.item.forceActiveFocus()
                    }

                    Keys.onRightPressed: {
                        root.currentSection = currentIndex
                        contentLoader.item.forceActiveFocus()
                    }

                    Keys.onUpPressed: {
                        if (currentIndex > 0) currentIndex--
                    }

                    Keys.onDownPressed: {
                        if (currentIndex < root.sections.length - 1) currentIndex++
                    }
                }

                // Back hint
                Rectangle {
                    Layout.fillWidth: true
                    height: 100
                    color: Theme.surfaceHover

                    Text {
                        anchors.centerIn: parent
                        text: "B: Back to Home"
                        font.pixelSize: Theme.fontHint
                        color: Theme.textDim
                    }
                }
            }
        }

        // Divider
        Rectangle {
            Layout.fillHeight: true
            width: 3
            color: Theme.surfaceHover
        }

        // Right content area
        Item {
            Layout.fillWidth: true
            Layout.fillHeight: true

            // Section title bar
            Rectangle {
                id: sectionHeader
                anchors.top: parent.top
                anchors.left: parent.left
                anchors.right: parent.right
                height: Theme.statusBarHeight
                color: Theme.surfaceHover

                Text {
                    anchors.centerIn: parent
                    text: root.sections[root.currentSection].name
                    font.pixelSize: Theme.fontTitle
                    font.bold: true
                    color: Theme.text
                }
            }

            // Content loader
            Item {
                id: contentArea
                anchors.top: sectionHeader.bottom
                anchors.left: parent.left
                anchors.right: parent.right
                anchors.bottom: parent.bottom

                Loader {
                    id: contentLoader
                    anchors.fill: parent
                    sourceComponent: {
                        switch(root.currentSection) {
                            case 0: return audioComp
                            case 1: return bluetoothComp
                            case 2: return networkComp
                            case 3: return displayComp
                            case 4: return powerComp
                            default: return audioComp
                        }
                    }
                }

                Component {
                    id: audioComp
                    AudioSettings {}
                }

                Component {
                    id: bluetoothComp
                    BluetoothSettings {}
                }

                Component {
                    id: networkComp
                    NetworkSettings {}
                }

                Component {
                    id: displayComp
                    DisplaySettings {}
                }

                Component {
                    id: powerComp
                    PowerSettings {}
                }
            }
        }
    }

    // Global key handling
    Keys.onEscapePressed: root.closed()

    function returnToSidebar() {
        sidebarList.forceActiveFocus()
    }

    Keys.onLeftPressed: {
        if (!sidebarList.activeFocus) {
            returnToSidebar()
        }
    }

    Keys.onPressed: (event) => {
        if (event.key === Qt.Key_B && !event.modifiers) {
            if (!sidebarList.activeFocus) {
                returnToSidebar()
                event.accepted = true
            }
        }
    }
}
