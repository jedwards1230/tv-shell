import QtQuick
import QtQuick.Layouts

// Shared settings list: row-count sizing (NEVER contentHeight, which balloons
// inside a ColumnLayout — #123/#139). Size = min(count*rowStride, cap). A single
// trailing fillHeight spacer on the OWNING page absorbs slack so content top-packs.
ListView {
    id: root
    // rowStride = delegate height + spacing (caller sets both to match its delegate).
    // callers MUST override rowStride and maxHeight to match their delegate geometry
    property int rowStride: 104
    property int maxHeight: 300
    property int minRows: 0

    Layout.fillWidth: true
    Layout.preferredHeight: Math.min(Math.max(count, minRows) * rowStride, maxHeight)
    clip: true
}
