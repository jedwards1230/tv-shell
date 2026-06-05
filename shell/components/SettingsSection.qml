import QtQuick
import QtQuick.Layouts

// Header + content block used by every settings page. Header is a bold body Text;
// children are top-packed. Pages compose this; the single trailing fillHeight
// spacer stays at the page's ColumnLayout root, not here.
ColumnLayout {
    id: root
    property string title: ""
    Layout.fillWidth: true
    spacing: 16

    Text {
        visible: root.title !== ""
        text: root.title
        font.pixelSize: Theme.fontBody
        font.bold: true
        color: Theme.textPrimary
        Layout.fillWidth: true
    }
}
