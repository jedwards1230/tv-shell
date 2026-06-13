import QtQuick
import QtQuick.Layouts
import "../"

Text {
    property bool block: true
    font.pixelSize: Theme.fontBody
    font.bold: true
    color: Theme.textPrimary
    Layout.fillWidth: block
}
