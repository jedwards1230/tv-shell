import QtQuick
import "../"

// Focus wrapper around SettingsButton: a FocusScope that participates in
// KeyNavigation chains and forwards click/activation with the standard
// forceActiveFocus() sequence baked in. Replaces the repeated
// FocusScope{ SettingsButton{ MouseArea{...} } } boilerplate in settings pages.
//
// Sizes itself to buttonWidth x buttonHeight (defaults to SettingsButton
// implicit size). Override for fixed-size toggles (e.g. 160x72).
FocusScope {
    id: scope

    property alias text: btn.text
    // Optional fill for "On" state (e.g. Theme.sidebarActive).
    // Set fillActive: true to apply fillColor; otherwise defaults to
    // focus-aware surface/hover handled by SettingsButton's own bindings.
    property color fillColor: Theme.sidebarActive
    property bool fillActive: false

    property int buttonWidth: 0    // 0 = use btn.implicitWidth
    property int buttonHeight: 0   // 0 = use btn.implicitHeight

    signal activated

    activeFocusOnTab: true

    implicitWidth: buttonWidth > 0 ? buttonWidth : btn.implicitWidth
    implicitHeight: buttonHeight > 0 ? buttonHeight : btn.implicitHeight

    Keys.onReturnPressed: scope.activated()
    Keys.onEnterPressed: scope.activated()

    SettingsButton {
        id: btn
        anchors.fill: parent
        focus: scope.activeFocus
        // Mirror call-site fill logic: explicit "On" fill, else let
        // SettingsButton's own focus-aware color binding take over.
        color: scope.fillActive ? scope.fillColor : (scope.activeFocus ? Theme.surfaceHover : Theme.surface)

        onActivated: scope.activated()
    }

    // Top-level MouseArea that intercepts clicks and gives focus to the
    // FocusScope (not just the inner btn), then fires the activated signal.
    // This matches the original FocusScope{SettingsButton{MouseArea{...}}} pattern
    // where the inner MouseArea explicitly called scope.forceActiveFocus().
    MouseArea {
        anchors.fill: parent
        hoverEnabled: true
        cursorShape: Qt.PointingHandCursor
        onPositionChanged: mouse => {
            let p = mapToItem(null, mouse.x, mouse.y);
            InputMode.pointerMoved(p.x, p.y);
        }
        onClicked: {
            scope.forceActiveFocus();
            scope.activated();
        }
    }
}
