import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import QtQuick.Window
import Qt5Compat.GraphicalEffects
import "../components"
import "../components/lib"

FocusScope {
    id: root
    visible: false

    signal closed

    property int currentSection: 0
    property int _pendingSection: 0

    // Opaque panel background. Lives as a child Rectangle now that the root is a
    // FocusScope (C4) rather than a Rectangle — the FocusScope properly delegates
    // focus into the sidebar/page, so the old "root is a plain Rectangle so
    // forceActiveFocus() won't steal focus" workarounds are gone.
    Rectangle {
        anchors.fill: parent
        color: Theme.background
    }

    // ── Public API ──────────────────────────────────────────────────────────
    // The only surface shell.qml / ShellLayout should call. The internals
    // (openSection / openSectionById / visible) stay private impl below; route
    // navigation through these so the host never reaches into panel state.
    //   signal closed  — emitted when the user backs out (page → sidebar → here);
    //                    the host listens for it to return Home.

    // open() — show the panel on its first section, mirroring the fresh-open path
    // in openSection() (visible=true then forceActiveFocus; onVisibleChanged drives
    // the focusTimer that lands focus on the sidebar). The FocusScope root forwards
    // focus to its focused child, so the sidebar ends up focused either way.
    function open() {
        if (visible) {
            forceActiveFocus();
            return;
        }
        _pendingSection = 0;
        visible = true;
        forceActiveFocus();
    }

    // openPage(id) — deep-link to a section by slug; returns false for an unknown
    // id so the caller can log it. The "widgets"/"moonlight"/"streaming" slugs are
    // intercepted in ShellLayout.openSettings (top-level Widgets surface), so they
    // never reach here.
    function openPage(id) {
        return openSectionById(id);
    }

    // close() — dismiss without emitting closed() (no user-intent echo); used by
    // the host's reset paths. Not idle-gated, so it works from any shell state.
    function close() {
        visible = false;
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
        // Widgets is promoted to a top-level surface (WidgetsApp), reached from
        // the nav drawer / the `widgets` deep-link (intercepted in
        // ShellLayout.openSettings) — it is no longer a Settings sidebar page. The
        // demoted "moonlight"/"streaming" deep-links also reroute there.
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
            iconSource: "icons/system.svg",
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

    // Set by Return on the sidebar so the page is entered (focusFirst) once its
    // Loader has swapped in — see contentLoader.onLoaded. Entering a page is gated
    // on A: Right no longer crosses into the page, and Left no longer backs out.
    property bool _pendingEnter: false

    Timer {
        id: focusTimer
        interval: 50
        onTriggered: sidebarList.forceActiveFocus()
    }

    RowLayout {
        anchors.fill: parent
        spacing: 0

        // Left sidebar
        Rectangle {
            Layout.fillHeight: true
            Layout.preferredWidth: Units.sidebarWidth
            color: Theme.surface

            ColumnLayout {
                anchors.fill: parent
                spacing: 0

                // Settings title — plain text, no colored bar.
                // Layout.preferredHeight (not bare height) — a bare `height` is
                // ignored inside a ColumnLayout, collapsing the title to zero.
                Item {
                    Layout.fillWidth: true
                    Layout.preferredHeight: Theme.statusBarHeight

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
                        height: Units.settingsRowHeight
                        color: {
                            if (root.currentSection === index)
                                return Theme.sidebarActive;
                            if (sidebarList.currentIndex === index && sidebarList.activeFocus && !InputMode.mouseMode)
                                return Theme.surfaceHover;
                            if (sidebarMA.containsMouse && InputMode.mouseMode)
                                return Theme.surfaceHover;
                            return "transparent";
                        }

                        Behavior on color {
                            ColorAnimation {
                                duration: 150
                            }
                        }

                        // Left accent bar on focused item
                        FocusAccentBar {
                            active: (sidebarList.currentIndex === index && sidebarList.activeFocus && !InputMode.mouseMode) || (sidebarMA.containsMouse && InputMode.mouseMode)
                        }

                        RowLayout {
                            anchors.fill: parent
                            anchors.leftMargin: 40
                            anchors.rightMargin: 40
                            spacing: 20

                            Item {
                                Layout.preferredWidth: 64
                                Layout.fillHeight: true

                                // Sidebar icons use a color overlay (#161) so all icons
                                // (including the multicolor appearance.svg) render in one
                                // consistent style that tracks the theme in both dark and
                                // light modes.  Selected = textPrimary, unselected = textSecondary.
                                Image {
                                    id: secIcon
                                    anchors.centerIn: parent
                                    source: Qt.resolvedUrl(modelData.iconSource)
                                    sourceSize: Qt.size(Units.iconSizeMD, Units.iconSizeMD)
                                    width: Units.iconSizeMD
                                    height: Units.iconSizeMD
                                    fillMode: Image.PreserveAspectFit
                                    visible: status === Image.Ready
                                    layer.enabled: status === Image.Ready
                                    layer.effect: ColorOverlay {
                                        color: root.currentSection === index ? Theme.textOnDark : Theme.textSecondary
                                    }
                                }

                                Text {
                                    anchors.centerIn: parent
                                    text: modelData.fallback
                                    font.pixelSize: Theme.fontBody
                                    color: root.currentSection === index ? Theme.textOnDark : Theme.textSecondary
                                    horizontalAlignment: Text.AlignHCenter
                                    visible: secIcon.status !== Image.Ready
                                }
                            }

                            Text {
                                text: modelData.name
                                font.pixelSize: Theme.fontBody
                                font.bold: root.currentSection === index
                                color: root.currentSection === index ? Theme.textOnDark : Theme.textSecondary
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
                                if (!InputMode.mouseMode && sidebarMA.containsMouse) {
                                    sidebarList.currentIndex = index;
                                    sidebarList.forceActiveFocus();
                                }
                            }
                        }
                    }

                    // A (Return) is the single gate INTO a page: it loads the
                    // highlighted section (if not already shown) and enters it,
                    // focusing the page's first control. If the section is already
                    // shown, enter directly; otherwise flag _pendingEnter and let
                    // contentLoader.onLoaded enter once the Loader has swapped (the
                    // "defer focus until the FocusScope is realized" rule). Every
                    // page exposes focusFirst() (its real first interactive
                    // element) — a bare root forceActiveFocus() dead-ends on the
                    // several pages whose root isn't a focus:true key handler.
                    Keys.onReturnPressed: {
                        if (root.currentSection === currentIndex) {
                            if (contentLoader.item && contentLoader.item.focusFirst)
                                contentLoader.item.focusFirst();
                        } else {
                            root._pendingEnter = true;
                            root.currentSection = currentIndex;
                        }
                    }

                    // Right does NOT cross into the page — entering is gated on A.

                    Keys.onUpPressed: {
                        if (currentIndex > 0)
                            currentIndex--;
                    }

                    Keys.onDownPressed: {
                        if (currentIndex < root.sections.length - 1)
                            currentIndex++;
                    }

                    // B/Escape on the sidebar bubbles up to the root's unified
                    // _back() handler (#5 C5) — no sidebar-local Key_B handler.
                }

                // Back hint
                Rectangle {
                    Layout.fillWidth: true
                    height: Units.settingsHintHeight
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
                        layer.enabled: status === Image.Ready
                        layer.effect: ColorOverlay {
                            color: Theme.textPrimary
                        }
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
                    // Interactive so wheel/drag (mouse mode) can scroll the pane —
                    // and so the visible scrollbar is actually draggable. Controller
                    // nav still drives it via focus-follow (ensureVisible below).
                    interactive: true
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
                        if (!it)
                            return;
                        var p = it.mapToItem(contentFlick.contentItem, 0, 0);
                        var maxY = Math.max(0, contentFlick.contentHeight - contentFlick.height);
                        if (p.y < contentFlick.contentY) {
                            // Scrolling up: reveal the item (with a small top margin).
                            contentFlick.contentY = Math.max(0, p.y - 24);
                        } else if (p.y + it.height > contentFlick.contentY + contentFlick.height) {
                            // Scrolling down: normally reveal the item with a small
                            // bottom margin — but if it sits within the FINAL
                            // screenful of content, jump to the very bottom so any
                            // trailing non-focusable content below the last control
                            // (hint bar, status/format text) comes into view too,
                            // instead of being stranded just under the fold. This
                            // makes the whole page reachable on every settings page
                            // without per-page layout workarounds.
                            contentFlick.contentY = (p.y >= maxY) ? maxY : Math.min(p.y + it.height - contentFlick.height + 24, maxY);
                        }
                    }

                    Loader {
                        id: contentLoader
                        width: contentFlick.width
                        height: Math.max(item ? item.implicitHeight : 0, contentFlick.height)
                        sourceComponent: root.sections[root.currentSection].component
                        onLoaded: {
                            contentFlick.contentY = 0;
                            // Return-to-enter: the page just swapped in, so focus
                            // its first control now that the Loader has realized it.
                            if (root._pendingEnter) {
                                root._pendingEnter = false;
                                if (item && item.focusFirst)
                                    item.focusFirst();
                            }
                        }
                    }
                }

                // Follow keyboard/controller focus — scroll the pane to keep the
                // focused control visible. Window.activeFocusItem is the attached
                // property that works here (Window.window can't be a Connections target).
                property Item _afItem: Window.activeFocusItem
                on_AfItemChanged: if (contentArea._afItem)
                    contentFlick.ensureVisible(contentArea._afItem)
            }
        }
    }

    // Global back handling — B / Escape is hierarchical and UNIFIED (#5 C5).
    // The controller B button arrives as Escape; a literal keyboard 'B' is the
    // other source. Both now route through one _back(): from inside a settings
    // page it backs focus out to the sidebar; from the sidebar it closes the
    // panel and returns Home. So: page -> B -> sidebar -> B -> Home, identically
    // for Escape and keyboard-B.
    //
    // Behavior change: keyboard-B on the sidebar now CLOSES the panel (via this
    // unified path). Previously the sidebar's own Key_B handler closed it while
    // the root Key_B handler was a no-op on the sidebar — the two are now one.
    function _back() {
        if (!sidebarList.activeFocus)
            returnToSidebar();
        else
            root.closed();
    }

    Keys.onEscapePressed: root._back()

    function openSection(idx) {
        if (visible) {
            currentSection = idx;
            sidebarList.currentIndex = idx;
            contentFlick.contentY = 0;
            sidebarList.forceActiveFocus();
            return;
        }
        _pendingSection = idx;
        visible = true;
        forceActiveFocus();
    }

    function openSectionById(id) {
        // Widgets is a top-level surface and the moonlight/streaming deep-links
        // reroute to it — both are intercepted in ShellLayout.openSettings before
        // SettingsApp is ever asked, so they never reach here. Any unknown slug
        // returns false (the caller logs it); no crash.
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

    // Left does NOT back out of a page — backing out is gated on B/Escape
    // (page → sidebar → Home). Left is left to the focused control for in-row
    // movement, matching the "directional keys don't cross boundaries" model.

    Keys.onPressed: event => {
        if (event.key === Qt.Key_B && !event.modifiers) {
            root._back();
            event.accepted = true;
        }
    }
}
