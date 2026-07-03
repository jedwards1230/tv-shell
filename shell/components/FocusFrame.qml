import QtQuick
import QtQuick.Effects

// Card focus chrome: a scale-up + brighter fill + crimson ring + glow on focus.
//
// HAZARD — center-origin focus scale overflows its container. The default
// `scaleEnabled: true` grows the frame `focusScale`× about its centre, so a
// FULL-WIDTH frame's scaled bounds spill past BOTH parent/screen edges on focus
// (hit repeatedly, worked around ad hoc by bounding the frame's own width or by
// `scaleEnabled: false`). To make it safe by construction, give the frame an
// `availableWidth` (the parent clip bound): the effective focus scale is then
// clamped so the scaled width never exceeds it. When `availableWidth` is left 0
// (unbounded) the behavior is UNCHANGED — callers that already bound their frame
// (WakeCard) or disable scale (NowPlayingCard) render identically.
Item {
    id: root

    property bool focused: false
    property int focusBorderWidth: 3
    property int restBorderWidth: 2
    property color focusBorderColor: Theme.cardFocusBorder
    property color restBorderColor: Theme.surfaceBorder
    property bool scaleEnabled: true
    property real focusScale: 1.06
    property real restScale: 1.0

    // Optional clip bound (px) the SCALED frame must fit within. 0 = unbounded
    // (legacy behavior — no clamp). When > 0, the effective focus scale is capped
    // at availableWidth/width so a center-origin scale can't overflow the edges.
    property real availableWidth: 0

    // The focus scale actually applied: focusScale, capped so the scaled width
    // (width × scale) stays within availableWidth when a bound is provided. Never
    // shrinks below restScale.
    readonly property real effectiveFocusScale: {
        if (root.availableWidth <= 0 || root.width <= 0)
            return root.focusScale;
        return Math.max(root.restScale, Math.min(root.focusScale, root.availableWidth / root.width));
    }

    // Opt-in dev warning: an unbounded scaling full-width frame is the overflow
    // trap above. Set true on a suspect call site to get a one-line console hint.
    property bool debugScaleBounds: false
    Component.onCompleted: {
        if (root.debugScaleBounds && root.scaleEnabled && root.focusScale > root.restScale && root.availableWidth <= 0)
            console.warn("FocusFrame: scaling with no availableWidth — a full-width frame will overflow its container on focus. Set availableWidth to the parent clip bound.");
    }
    property color backgroundColor: Theme.cardBackground
    property color focusBackgroundColor: Theme.surfaceHover
    property color glowColor: Theme.cardFocusGlow
    property int glowSize: 12
    property real radius: Theme.cardRadius
    property int scaleDuration: 180
    property int borderDuration: 180
    property int focusZ: 10
    property int restZ: 0

    // Reduce-motion guard (#109): bind directly to Theme.reduceMotion now that the
    // setting is live. When true, all animation durations collapse to 0 and scale is
    // suppressed — the static ring+fill+glow cues remain so focus is still visible.
    property bool animationsEnabled: !Theme.reduceMotion

    default property alias content: contentArea.data

    z: root.focused ? root.focusZ : root.restZ

    transform: [
        Scale {
            origin.x: root.width / 2
            origin.y: root.height / 2
            // Under reduce-motion (animationsEnabled=false) the scale is removed
            // ENTIRELY — the target stays at restScale, it is NOT just animated at
            // duration 0. Focus remains clearly marked by the static ring + glow +
            // brighter fill below, so do not "restore" a zero-duration scale here.
            xScale: root.scaleEnabled && root.focused && root.animationsEnabled ? root.effectiveFocusScale : root.restScale
            yScale: root.scaleEnabled && root.focused && root.animationsEnabled ? root.effectiveFocusScale : root.restScale
            Behavior on xScale {
                NumberAnimation {
                    duration: root.animationsEnabled ? root.scaleDuration : 0
                    easing.type: Easing.OutCubic
                }
            }
            Behavior on yScale {
                NumberAnimation {
                    duration: root.animationsEnabled ? root.scaleDuration : 0
                    easing.type: Easing.OutCubic
                }
            }
        }
    ]

    // Soft elevation glow — a real blurred drop shadow that fades out gradually
    // (not a solid translucent ring). A colored rounded-rect source is blurred by
    // a MultiEffect and painted behind the card surface.
    Rectangle {
        id: glowSource
        anchors.fill: frame
        radius: root.radius
        color: root.glowColor
        visible: false
        layer.enabled: true
    }

    MultiEffect {
        anchors.fill: glowSource
        source: glowSource
        autoPaddingEnabled: true
        blurEnabled: true
        blur: 1.0
        blurMax: root.glowSize
        opacity: root.focused ? 1.0 : 0.0
        visible: opacity > 0
        z: -1

        Behavior on opacity {
            NumberAnimation {
                duration: root.animationsEnabled ? root.borderDuration : 0
            }
        }
    }

    Rectangle {
        id: frame
        anchors.fill: parent
        radius: root.radius
        color: root.focused ? root.focusBackgroundColor : root.backgroundColor
        border.width: root.focused ? root.focusBorderWidth : root.restBorderWidth
        border.color: root.focused ? root.focusBorderColor : root.restBorderColor

        Behavior on color {
            ColorAnimation {
                duration: root.animationsEnabled ? root.borderDuration : 0
            }
        }
        Behavior on border.width {
            NumberAnimation {
                duration: root.animationsEnabled ? root.borderDuration : 0
            }
        }
        Behavior on border.color {
            ColorAnimation {
                duration: root.animationsEnabled ? root.borderDuration : 0
            }
        }

        Item {
            id: contentArea
            anchors.fill: parent
        }
    }
}
