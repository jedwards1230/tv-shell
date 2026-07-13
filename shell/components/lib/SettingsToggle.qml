import QtQuick
import "../"

// A 10-foot On/Off toggle rendered as a segmented pill: two labelled segments
// with the SELECTED one filled — ember when On, a muted raised grey when Off —
// so state reads at couch distance and is visually distinct from an action
// button (issue #270). Single focus stop; A / Return / click toggles. The
// component is CONTROLLED: `checked` reflects the backing setting and the call
// site flips that setting in `onToggled` (mirrors the old fillActive/onActivated
// pattern it replaces).
FocusScope {
    id: root

    // Backing state (bind to the setting). The component never mutates it — the
    // call site flips the source in onToggled.
    property bool checked: false

    // Segment labels (default On/Off; e.g. Enabled/Disabled for reduce-motion).
    property string onLabel: "On"
    property string offLabel: "Off"

    // Fired on every activation (A / Return / Enter / click). The whole pill is
    // one toggle affordance — a click anywhere flips, matching a physical switch.
    signal toggled

    activeFocusOnTab: true

    readonly property bool showFocus: activeFocus && !InputMode.mouseMode
    readonly property int segPadding: 28

    // Both segments sized to the wider label so the sliding knob never clips
    // (handles On/Off and the longer Enabled/Disabled pair alike).
    TextMetrics {
        id: onMetrics
        font.pixelSize: Theme.fontBody
        font.bold: true
        text: root.onLabel
    }
    TextMetrics {
        id: offMetrics
        font.pixelSize: Theme.fontBody
        font.bold: true
        text: root.offLabel
    }
    readonly property int segmentWidth: Math.round(Math.max(onMetrics.width, offMetrics.width) + 2 * segPadding)

    implicitWidth: segmentWidth * 2
    implicitHeight: 96

    Keys.onReturnPressed: root.toggled()
    Keys.onEnterPressed: root.toggled()

    // Track (the pill background + focus ring).
    Rectangle {
        id: track
        anchors.fill: parent
        radius: height / 2
        color: Theme.surface
        border.width: root.showFocus ? Units.borderThick : Units.borderThin
        border.color: root.showFocus ? Theme.focusBorder : Theme.surfaceBorder

        Behavior on border.color {
            ColorAnimation {
                duration: Theme.reduceMotion ? 0 : 150
            }
        }

        // Selected-segment fill — slides between the On (left) and Off (right)
        // halves; ember when On, a muted raised grey when Off. Color carries the
        // primary at-a-distance signal (orange present = On).
        Rectangle {
            id: knob
            readonly property int inset: 4
            width: root.segmentWidth - inset
            height: parent.height - 2 * inset
            radius: height / 2
            y: inset
            x: root.checked ? inset : parent.width - width - inset
            color: root.checked ? Theme.ember : Theme.surfaceHover

            Behavior on x {
                NumberAnimation {
                    duration: Theme.reduceMotion ? 0 : 150
                    easing.type: Easing.OutCubic
                }
            }
            Behavior on color {
                ColorAnimation {
                    duration: Theme.reduceMotion ? 0 : 150
                }
            }
        }

        Row {
            anchors.fill: parent

            Item {
                width: root.segmentWidth
                height: parent.height

                Text {
                    anchors.centerIn: parent
                    text: root.onLabel
                    font.pixelSize: Theme.fontBody
                    font.bold: root.checked
                    color: root.checked ? Theme.textOnDark : Theme.textMuted
                }
            }

            Item {
                width: root.segmentWidth
                height: parent.height

                Text {
                    anchors.centerIn: parent
                    text: root.offLabel
                    font.pixelSize: Theme.fontBody
                    font.bold: !root.checked
                    color: root.checked ? Theme.textMuted : Theme.textPrimary
                }
            }
        }
    }

    Accessible.role: Accessible.CheckBox
    Accessible.name: root.checked ? root.onLabel : root.offLabel
    Accessible.checked: root.checked
    Accessible.focusable: true
    Accessible.onToggleAction: root.toggled()
    Accessible.onPressAction: root.toggled()

    // Whole-pill click = toggle (physical-switch idiom). Mirrors FocusButton's
    // pointer handling: flip to mouse-mode only on a genuine pointer move, and on
    // click enter mouse-mode + focus the scope + fire the signal.
    MouseArea {
        anchors.fill: parent
        hoverEnabled: true
        cursorShape: Qt.PointingHandCursor
        onPositionChanged: mouse => {
            let p = mapToItem(null, mouse.x, mouse.y);
            InputMode.pointerMoved(p.x, p.y);
        }
        onClicked: {
            InputMode.enterMouseMode();
            root.forceActiveFocus();
            root.toggled();
        }
    }
}
