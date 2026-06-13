import QtQuick
import QtQuick.Layouts
import "../"

Text {
    // NOTE: do NOT redeclare `text` here — Text already has a built-in `text`
    // property. A `property string text` on a Text root SHADOWS the built-in one,
    // so the string gets set but never rendered (the hint shows blank). Callers
    // set the built-in property directly: `HintBar { text: "..." }`.
    // `muted` lets overlay popovers use the dimmer textMuted; settings pages keep
    // the default textSecondary.
    property bool muted: false
    font.pixelSize: Theme.fontHint
    color: muted ? Theme.textMuted : Theme.textSecondary
    Layout.alignment: Qt.AlignHCenter
}
