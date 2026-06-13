import QtQuick
import QtQuick.Layouts
import "../"

RowLayout {
    id: prow
    property string label: ""
    property string description: ""
    default property alias trailing: holder.data
    Layout.fillWidth: true
    spacing: 24

    ColumnLayout {
        spacing: 2
        Layout.fillWidth: true

        Text {
            text: prow.label
            font.pixelSize: Theme.fontSmall
            color: Theme.textSecondary
        }

        Text {
            visible: prow.description !== ""
            text: prow.description
            font.pixelSize: Theme.fontHint
            color: Theme.textSecondary
            wrapMode: Text.WordWrap
            Layout.fillWidth: true
        }
    }

    Item {
        id: holder
        Layout.alignment: Qt.AlignRight | Qt.AlignVCenter
        implicitWidth: childrenRect.width
        implicitHeight: childrenRect.height
    }
}
