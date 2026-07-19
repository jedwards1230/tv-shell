import QtQuick
import QtQuick.Layouts
import "../components"
import "../components/lib"

// Web Apps settings page (#187, P0 stub). Read-only list of the web-app
// registry — a daemon-owned mirror (SettingsStore.webApps; the daemon is the
// sole writer via the P1 webapp-add/webapp-remove IPC). This P0 slice ships
// a display-only list: rows are focusable/navigable so the page holds up as
// a controller-first surface, but carry no actions yet (edit/remove is P4;
// the add flow needs the on-screen keyboard, out of scope here).
SettingsPageBase {
    id: root
    hintText: "B: Back"

    readonly property bool hasApps: SettingsStore.webApps.length > 0

    function focusFirst() {
        if (root.hasApps)
            appList.forceActiveFocus();
    }

    SectionHeader {
        text: "Web Apps"
    }

    SettingsList {
        id: appList
        // rowStride = delegate 96 + spacing 8 (#123/#139 row-count sizing).
        rowStride: 104
        maxHeight: 400
        spacing: 8
        visible: root.hasApps
        model: SettingsStore.webApps

        delegate: SettingsListRow {
            required property int index
            required property var modelData
            width: appList.width
            selected: appList.currentIndex === index && appList.activeFocus

            RowLayout {
                anchors.fill: parent
                anchors.leftMargin: 24
                anchors.rightMargin: 24
                anchors.topMargin: 16
                anchors.bottomMargin: 16
                spacing: 16

                Text {
                    text: modelData.name
                    font.pixelSize: Theme.fontSmall
                    color: Theme.textPrimary
                    elide: Text.ElideRight
                    Layout.fillWidth: true
                }

                Text {
                    text: modelData.url
                    font.pixelSize: Theme.fontSmall
                    color: Theme.textMuted
                    elide: Text.ElideMiddle
                    Layout.maximumWidth: appList.width * 0.5
                }
            }
        }
    }

    SettingsEmptyState {
        Layout.fillWidth: true
        Layout.preferredHeight: visible ? Units.gridUnit * 4 : 0
        visible: !root.hasApps
        icon: "\u{1F310}"
        line: "No web apps yet"
        hint: "Add YouTube, Plex, and more from here in a future update."
    }
}
