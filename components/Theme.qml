pragma Singleton
import QtQuick

QtObject {
    readonly property color background: "#1a1a2e"
    readonly property color surface: "#16213e"
    readonly property color surfaceHover: "#1a2744"
    readonly property color primary: "#0f3460"
    readonly property color accent: "#e94560"
    readonly property color text: "#eee"
    readonly property color textDim: "#aaa"
    readonly property color online: "#4ade80"
    readonly property color offline: "#f87171"
    readonly property color warning: "#fbbf24"

    readonly property int cardWidth: 320
    readonly property int cardHeight: 200
    readonly property int cardSpacing: 24
    readonly property int cardRadius: 12
    readonly property int statusBarHeight: 48
    readonly property int padding: 24

    readonly property int fontTitle: 28
    readonly property int fontBody: 18
    readonly property int fontSmall: 14
    readonly property int fontStatus: 16
}
