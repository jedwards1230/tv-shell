import QtQuick
import QtQuick.Layouts

Rectangle {
    id: root
    color: Theme.background
    visible: false

    signal closed

    property int currentSection: 0

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
        id: controllerComp
        ControllerSettings {}
    }

    Component {
        id: keyBindingsComp
        KeyBindingsSettings {}
    }

    Component {
        id: avControlComp
        AVControlSettings {}
    }

    Component {
        id: appearanceComp
        AppearanceSettings {}
    }

    Component {
        id: powerComp
        PowerSettings {}
    }

    // The streaming section is contributed by the active provider's
    // settingsComponent; with the no-streaming provider it's null and the
    // section is omitted entirely.
    readonly property var sections: {
        let s = [
            {
                name: "Audio",
                iconName: "audio-volume-high",
                iconDir: "status/22",
                fallback: "♫",
                component: audioComp
            },
            {
                name: "Bluetooth",
                iconName: "bluetooth",
                iconDir: "status/22",
                fallback: "ᛒ",
                component: bluetoothComp
            },
            {
                name: "Network",
                iconName: "network-wired",
                iconDir: "status/22",
                fallback: "⇅",
                component: networkComp
            },
            {
                name: "Display",
                iconName: "video-display",
                iconDir: "devices/22",
                fallback: "\u{1F5A5}",
                component: displayComp
            },
            {
                name: "Controllers",
                iconName: "input-gaming",
                iconDir: "devices/22",
                fallback: "\u{1F3AE}",
                component: controllerComp
            },
            {
                name: "Key Bindings",
                iconName: "input-keyboard",
                iconDir: "devices/22",
                fallback: "⌨",
                component: keyBindingsComp
            },
            {
                name: "AV Control",
                iconName: "video-television",
                iconDir: "devices/22",
                fallback: "\u{1F4FA}",
                component: avControlComp
            }
        ];
        let provider = StreamProviders.active;
        if (provider.settingsComponent)
            s.push({
                name: provider.displayName,
                iconName: "applications-games",
                iconDir: "apps/22",
                fallback: "\u{1F319}",
                component: provider.settingsComponent
            });
        s.push({
            name: "Appearance",
            iconName: "preferences-desktop-theme",
            iconDir: "apps/22",
            fallback: "\u{1F3A8}",
            component: appearanceComp
        });
        s.push({
            name: "Power",
            iconName: "system-shutdown",
            iconDir: "actions/22",
            fallback: "⏻",
            component: powerComp
        });
        return s;
    }

    onVisibleChanged: {
        if (visible) {
            currentSection = 0;
            sidebarList.currentIndex = 0;
            // Delay focus slightly to ensure Loader has settled
            focusTimer.restart();
        }
    }

    Timer {
        id: focusTimer
        interval: 50
        onTriggered: {
            sidebarList.forceActiveFocus();
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

                // Settings title — plain text, no colored bar
                Item {
                    Layout.fillWidth: true
                    height: Theme.statusBarHeight

                    Text {
                        anchors.left: parent.left
                        anchors.leftMargin: 48
                        anchors.verticalCenter: parent.verticalCenter
                        text: "Settings"
                        font.pixelSize: Theme.fontHero * 0.6
                        font.bold: true
                        color: Theme.textPrimary
                    }
                }

                // Section list
                ListView {
                    id: sidebarList
                    Layout.fillWidth: true
                    Layout.fillHeight: true
                    model: root.sections
                    currentIndex: 0
                    focus: true
                    clip: true

                    delegate: Rectangle {
                        required property int index
                        required property var modelData
                        width: sidebarList.width
                        height: 100
                        color: {
                            if (root.currentSection === index)
                                return Theme.sidebarActive;
                            if (sidebarList.currentIndex === index && sidebarList.activeFocus && !Theme.mouseMode)
                                return Theme.surfaceHover;
                            if (sidebarMA.containsMouse && Theme.mouseMode)
                                return Theme.surfaceHover;
                            return "transparent";
                        }

                        Behavior on color {
                            ColorAnimation {
                                duration: 150
                            }
                        }

                        // Left accent bar on focused item
                        Rectangle {
                            anchors.left: parent.left
                            anchors.verticalCenter: parent.verticalCenter
                            width: 4
                            height: parent.height - 16
                            radius: 2
                            color: Theme.focusBorder
                            visible: (sidebarList.currentIndex === index && sidebarList.activeFocus && !Theme.mouseMode) || (sidebarMA.containsMouse && Theme.mouseMode)
                        }

                        RowLayout {
                            anchors.fill: parent
                            anchors.leftMargin: 40
                            anchors.rightMargin: 40
                            spacing: 20

                            Item {
                                Layout.preferredWidth: 64
                                Layout.fillHeight: true

                                Image {
                                    id: secIcon
                                    anchors.centerIn: parent
                                    source: IconTheme.base ? "file://" + IconTheme.base + "/" + modelData.iconDir + "/" + modelData.iconName + ".svg" : ""
                                    sourceSize: Qt.size(Units.iconSizeMD, Units.iconSizeMD)
                                    width: Units.iconSizeMD
                                    height: Units.iconSizeMD
                                    fillMode: Image.PreserveAspectFit
                                    visible: status === Image.Ready
                                }

                                Text {
                                    anchors.centerIn: parent
                                    text: modelData.fallback
                                    font.pixelSize: Theme.fontBody
                                    color: root.currentSection === index ? Theme.textPrimary : Theme.textSecondary
                                    horizontalAlignment: Text.AlignHCenter
                                    visible: secIcon.status !== Image.Ready
                                }
                            }

                            Text {
                                text: modelData.name
                                font.pixelSize: Theme.fontBody
                                font.bold: root.currentSection === index
                                color: root.currentSection === index ? Theme.textPrimary : Theme.textSecondary
                                Layout.fillWidth: true
                            }
                        }

                        MouseArea {
                            id: sidebarMA
                            anchors.fill: parent
                            hoverEnabled: true
                            cursorShape: Qt.PointingHandCursor
                            onClicked: {
                                sidebarList.currentIndex = index;
                                root.currentSection = index;
                                sidebarList.forceActiveFocus();
                            }
                        }

                        Connections {
                            target: Theme
                            function onMouseModeChanged() {
                                if (!Theme.mouseMode && sidebarMA.containsMouse) {
                                    sidebarList.currentIndex = index;
                                    sidebarList.forceActiveFocus();
                                }
                            }
                        }
                    }

                    Keys.onReturnPressed: {
                        root.currentSection = currentIndex;
                    }

                    // Right is a pure focus-shifter into the currently open
                    // page — it must NOT swap which section is shown. Swapping
                    // waits for A (Return). This lets you move the sidebar
                    // cursor away from the open page and still fall back into
                    // that page on right, instead of force-opening the hovered
                    // item's submenu.
                    //
                    // Every page exposes focusFirst() which focuses its real
                    // first interactive element. We call that instead of a bare
                    // forceActiveFocus() on the page root, because root-level
                    // focus only delegates correctly when the root points
                    // focus:true at a real key-handling item — several pages
                    // don't, and would dead-end on a silent layout.
                    Keys.onRightPressed: {
                        if (contentLoader.item && contentLoader.item.focusFirst)
                            contentLoader.item.focusFirst();
                    }

                    Keys.onUpPressed: {
                        if (currentIndex > 0)
                            currentIndex--;
                    }

                    Keys.onDownPressed: {
                        if (currentIndex < root.sections.length - 1)
                            currentIndex++;
                    }

                    Keys.onPressed: event => {
                        if (event.key === Qt.Key_B && !event.modifiers) {
                            root.closed();
                            event.accepted = true;
                        }
                    }
                }

                // Back hint
                Rectangle {
                    Layout.fillWidth: true
                    height: 80
                    color: Theme.surfaceHover

                    Text {
                        anchors.centerIn: parent
                        text: "B: Back to Home"
                        font.pixelSize: Theme.fontHint
                        color: Theme.textSecondary
                    }
                }
            }
        }

        // Divider
        Rectangle {
            Layout.fillHeight: true
            width: 2
            color: Theme.surfaceBorder
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

                RowLayout {
                    anchors.left: parent.left
                    anchors.leftMargin: 48
                    anchors.verticalCenter: parent.verticalCenter
                    spacing: 16

                    Image {
                        id: headerIcon
                        source: IconTheme.base ? "file://" + IconTheme.base + "/" + root.sections[root.currentSection].iconDir + "/" + root.sections[root.currentSection].iconName + ".svg" : ""
                        sourceSize: Qt.size(Theme.fontTitle, Theme.fontTitle)
                        width: Theme.fontTitle
                        height: Theme.fontTitle
                        fillMode: Image.PreserveAspectFit
                        visible: status === Image.Ready
                    }

                    Text {
                        text: root.sections[root.currentSection].fallback
                        font.pixelSize: Theme.fontTitle
                        color: Theme.textPrimary
                        visible: headerIcon.status !== Image.Ready
                    }

                    Text {
                        text: root.sections[root.currentSection].name
                        font.pixelSize: Theme.fontTitle
                        font.bold: true
                        color: Theme.textPrimary
                    }
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
                    sourceComponent: root.sections[root.currentSection].component
                }
            }
        }
    }

    // Global key handling
    Keys.onEscapePressed: root.closed()

    function returnToSidebar() {
        sidebarList.forceActiveFocus();
    }

    Keys.onLeftPressed: {
        if (!sidebarList.activeFocus) {
            returnToSidebar();
        }
    }

    Keys.onPressed: event => {
        if (event.key === Qt.Key_B && !event.modifiers) {
            if (!sidebarList.activeFocus) {
                returnToSidebar();
                event.accepted = true;
            }
        }
    }
}
