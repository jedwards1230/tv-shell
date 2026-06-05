import QtQuick

Item {
    property string icon: ""
    property string line: ""
    property string hint: ""

    Column {
        anchors.centerIn: parent
        spacing: Units.spacingSM

        Text {
            anchors.horizontalCenter: parent.horizontalCenter
            text: icon
            textFormat: Text.PlainText
            font.pixelSize: Theme.fontTitle
            color: Theme.textMuted
            horizontalAlignment: Text.AlignHCenter
            visible: icon !== ""
        }

        Text {
            anchors.horizontalCenter: parent.horizontalCenter
            text: line
            textFormat: Text.PlainText
            font.pixelSize: Theme.fontBody
            color: Theme.textMuted
            horizontalAlignment: Text.AlignHCenter
        }

        Text {
            anchors.horizontalCenter: parent.horizontalCenter
            text: hint
            textFormat: Text.PlainText
            font.pixelSize: Theme.fontHint
            color: Theme.textMuted
            horizontalAlignment: Text.AlignHCenter
            visible: hint !== ""
        }
    }
}
