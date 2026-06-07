import QtQuick
import QtQuick.Layouts
import Quickshell.Io

// Web Apps settings page (#187) — P0 foundation slice.
//
// Lists web apps from the registry mirror (SettingsStore.webApps) with an
// empty state. The registry is sourced over IPC; until the daemon ships the
// `webapp-list` command (P1) this gracefully shows the empty state. Adding /
// generating Chromium `--app` launchers is a documented follow-up — see
// docs/WEB_APPS.md (phases P1–P3). This page intentionally ships read-only +
// a disabled-looking "Add" affordance describing the follow-up, matching the
// repo's "ship a clean stub, build on the plan" approach.
//
// Controller-first: focusFirst() lands on the (placeholder) Add button; Left/B
// returns focus to the SettingsPanel sidebar.
FocusScope {
    id: root
    implicitHeight: mainCol.implicitHeight + 2 * Theme.padding

    // Read-through registry mirror. SettingsStore owns the IPC load; this page
    // just renders it. Empty array is the expected P0 state.
    readonly property var webApps: SettingsStore.webApps || []

    function focusFirst() {
        addScope.forceActiveFocus();
    }

    Component.onCompleted: SettingsStore.loadWebApps()

    ColumnLayout {
        id: mainCol
        anchors.fill: parent
        anchors.margins: Theme.padding
        spacing: 48

        // === Intro ===
        Text {
            text: "Web Apps"
            font.pixelSize: Theme.fontBody
            font.bold: true
            color: Theme.textPrimary
        }

        Text {
            text: "Add a website (YouTube, a dashboard, …) as an app. It appears on the home Applications row and launches like any installed app."
            font.pixelSize: Theme.fontSmall
            color: Theme.textSecondary
            wrapMode: Text.WordWrap
            Layout.fillWidth: true
        }

        // === Add (placeholder until P2 add-flow) ===
        FocusScope {
            id: addScope
            width: addBtn.width
            height: addBtn.height
            focus: true
            activeFocusOnTab: true

            KeyNavigation.down: root.webApps.length > 0 ? webAppList : addScope
            KeyNavigation.left: null

            SettingsButton {
                id: addBtn
                text: "Add Web App…"
                focus: parent.activeFocus
                anchors.fill: parent

                // P0: no add flow yet. Surface a clear note rather than a dead
                // button — the real flow (presets + URL entry) lands in P2.
                onActivated: noteRow.flash()

                MouseArea {
                    anchors.fill: parent
                    hoverEnabled: true
                    cursorShape: Qt.PointingHandCursor
                    onClicked: {
                        addScope.forceActiveFocus();
                        addBtn.activated();
                    }
                }
            }
        }

        Text {
            id: noteRow
            property bool _flashed: false
            text: _flashed ? "Adding web apps lands in a follow-up (see docs/WEB_APPS.md, issue #187)." : "Coming soon — see docs/WEB_APPS.md (#187)."
            font.pixelSize: Theme.fontHint
            color: _flashed ? Theme.focusBorder : Theme.textMuted
            wrapMode: Text.WordWrap
            Layout.fillWidth: true
            function flash() {
                _flashed = true;
            }
        }

        // === Existing web apps ===
        Text {
            text: "Installed Web Apps"
            font.pixelSize: Theme.fontBody
            font.bold: true
            color: Theme.textPrimary
            visible: root.webApps.length > 0
        }

        SettingsList {
            id: webAppList
            rowStride: 104
            maxHeight: 600
            minRows: 0
            spacing: 8
            visible: root.webApps.length > 0
            model: root.webApps
            keyNavigationEnabled: true

            KeyNavigation.up: addScope

            delegate: Rectangle {
                required property var modelData
                width: webAppList.width
                height: 96
                radius: 16
                color: ListView.isCurrentItem && webAppList.activeFocus ? Theme.surfaceHover : Theme.surface
                border.width: ListView.isCurrentItem && webAppList.activeFocus ? 3 : 2
                border.color: ListView.isCurrentItem && webAppList.activeFocus ? Theme.focusBorder : Theme.surfaceBorder

                RowLayout {
                    anchors.fill: parent
                    anchors.leftMargin: 24
                    anchors.rightMargin: 24
                    spacing: 24

                    ColumnLayout {
                        spacing: 2
                        Layout.fillWidth: true

                        Text {
                            text: modelData.name || modelData.id || "Web App"
                            font.pixelSize: Theme.fontBody
                            color: Theme.textPrimary
                            elide: Text.ElideRight
                            Layout.fillWidth: true
                        }

                        Text {
                            text: modelData.url || ""
                            font.pixelSize: Theme.fontHint
                            color: Theme.textSecondary
                            elide: Text.ElideRight
                            Layout.fillWidth: true
                        }
                    }
                }
            }
        }

        // Empty state when no web apps exist yet (the P0 default).
        SettingsEmptyState {
            Layout.fillWidth: true
            Layout.preferredHeight: 220
            visible: root.webApps.length === 0
            icon: "\u{1F310}"  // globe
            line: "No web apps yet"
            hint: "Add a site to launch it like an app"
        }

        Item {
            Layout.fillHeight: true
        }
    }
}
