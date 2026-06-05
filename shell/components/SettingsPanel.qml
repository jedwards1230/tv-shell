import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import QtQuick.Window

Rectangle {
    id: root
    color: Theme.background
    visible: false

    signal closed

    property int currentSection: 0
    property int _pendingSection: 0

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
        id: accessibilityComp
        AccessibilitySettings {}
    }

    Component {
        id: powerComp
        PowerSettings {}
    }

    Component {
        id: systemComp
        SystemSettings {}
    }

    // The streaming section is contributed by the active provider's
    // settingsComponent; with the no-streaming provider it's null and the
    // section is omitted entirely.
    readonly property var sections: {
        let s = [
            {
                id: "audio",
                name: "Audio",
                iconSource: "icons/audio.svg",
                fallback: "♫",
                component: audioComp
            },
            {
                id: "bluetooth",
                name: "Bluetooth",
                iconSource: "icons/bluetooth.svg",
                fallback: "ᛒ",
                component: bluetoothComp
            },
            {
                id: "network",
                name: "Network",
                iconSource: "icons/network.svg",
                fallback: "⇅",
                component: networkComp
            },
            {
                id: "display",
                name: "Display",
                iconSource: "icons/display.svg",
                fallback: "\u{1F5A5}",
                component: displayComp
            },
            {
                id: "controllers",
                name: "Controllers",
                iconSource: "icons/controllers.svg",
                fallback: "\u{1F3AE}",
                component: controllerComp
            },
            {
                id: "keybindings",
                name: "Key Bindings",
                iconSource: "icons/keybindings.svg",
                fallback: "⌨",
                component: keyBindingsComp
            },
            {
                id: "avcontrol",
                name: "AV Control",
                iconSource: "icons/avcontrol.svg",
                fallback: "\u{1F4FA}",
                component: avControlComp
            }
        ];
        let provider = StreamProviders.active;
        if (provider.settingsComponent)
            s.push({
                id: provider.id || "streaming",
                name: provider.displayName,
                iconSource: "icons/moonlight.svg",
                fallback: "\u{1F319}",
                component: provider.settingsComponent
            });
        s.push({
            id: "accessibility",
            name: "Accessibility",
            iconSource: "icons/accessibility.svg",
            fallback: "\u{267F}",
            component: accessibilityComp
        });
        s.push({
            id: "power",
            name: "Power",
            iconSource: "icons/power.svg",
            fallback: "⏻",
            component: powerComp
        });
        s.push({
            id: "system",
            name: "System",
            iconSource: "icons/display.svg",
            fallback: "\u{1F4BB}",
            component: systemComp
        });
        return s;
    }

    onVisibleChanged: {
        if (visible) {
            currentSection = _pendingSection;
            sidebarList.currentIndex = _pendingSection;
            _pendingSection = 0;
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
                                    source: Qt.resolvedUrl(modelData.iconSource)
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
                        source: Qt.resolvedUrl(root.sections[root.currentSection].iconSource)
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

                Flickable {
                    id: contentFlick
                    anchors.fill: parent
                    clip: true
                    interactive: false
                    contentWidth: width
                    contentHeight: contentLoader.height
                    boundsBehavior: Flickable.StopAtBounds

                    ScrollBar.vertical: ScrollBar {
                        policy: contentFlick.contentHeight > contentFlick.height ? ScrollBar.AlwaysOn : ScrollBar.AlwaysOff
                    }

                    Behavior on contentY {
                        NumberAnimation {
                            duration: 150
                            easing.type: Easing.OutCubic
                        }
                    }

                    function ensureVisible(it) {
                        if (!it) return;
                        var p = it.mapToItem(contentFlick.contentItem, 0, 0);
                        if (p.y < contentFlick.contentY)
                            contentFlick.contentY = Math.max(0, p.y - 24);
                        else if (p.y + it.height > contentFlick.contentY + contentFlick.height)
                            contentFlick.contentY = Math.min(
                                p.y + it.height - contentFlick.height + 24,
                                Math.max(0, contentFlick.contentHeight - contentFlick.height));
                    }

                    Loader {
                        id: contentLoader
                        width: contentFlick.width
                        height: Math.max(item ? item.implicitHeight : 0, contentFlick.height)
                        sourceComponent: root.sections[root.currentSection].component
                        onLoaded: contentFlick.contentY = 0
                    }
                }

                Connections {
                    target: Window.window
                    ignoreUnknownSignals: true
                    function onActiveFocusItemChanged() {
                        var fi = Window.window ? Window.window.activeFocusItem : null;
                        if (fi && contentFlick)
                            contentFlick.ensureVisible(fi);
                    }
                }
            }
        }
    }

    // Global key handling
    Keys.onEscapePressed: root.closed()

    function openSection(idx) {
        if (visible) {
            currentSection = idx;
            sidebarList.currentIndex = idx;
            contentFlick.contentY = 0;
            // Move focus straight to the sidebar and return — the root is a plain
            // Rectangle (not a FocusScope), so a trailing root.forceActiveFocus()
            // would steal focus back from the sidebar in the already-visible case.
            sidebarList.forceActiveFocus();
            return;
        }
        _pendingSection = idx;
        visible = true;
        forceActiveFocus();
    }

    function openSectionById(id) {
        for (let i = 0; i < sections.length; i++) {
            if (sections[i].id === id) {
                openSection(i);
                return true;
            }
        }
        return false;
    }

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
