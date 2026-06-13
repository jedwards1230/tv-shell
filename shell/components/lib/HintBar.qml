import QtQuick
import QtQuick.Layouts
import "../"

Text {
    // NOTE: do NOT redeclare `text` here — Text already has a built-in `text`
    // property. A `property string text` on a Text root SHADOWS the built-in one,
    // so the string gets set but never rendered (the hint shows blank). Callers
    // set the built-in property directly: `HintBar { text: "..." }`.
    font.pixelSize: Theme.fontHint
    color: Theme.textSecondary
    Layout.alignment: Qt.AlignHCenter
}
