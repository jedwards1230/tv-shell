import QtQuick
import QtQuick.Layouts
import "../"

Rectangle {
    id: pill
    // "good" = online/green, "warn" = gold, "bad" = offline/red, "neutral" = muted
    property string pillState: "neutral"
    property string text: ""
    property bool showDot: true
    readonly property color _accent: pillState === "good" ? Theme.online
        : pillState === "warn" ? Theme.gold
        : pillState === "bad" ? Theme.offline : Theme.textMuted
    implicitHeight: 56
    implicitWidth: pillRow.implicitWidth + 32
    radius: height / 2
    color: Qt.rgba(_accent.r, _accent.g, _accent.b, 0.2)

    RowLayout {
        id: pillRow
        anchors.centerIn: parent
        spacing: 12

        Rectangle {
            visible: pill.showDot
            Layout.preferredWidth: 16
            Layout.preferredHeight: 16
            radius: 8
            color: pill._accent
        }

        Text {
            text: pill.text
            font.pixelSize: Theme.fontSmall
            color: pill._accent
        }
    }
}
