import QtQuick
import "lib"

BaseCard {
    id: root

    required property var app
    // `running` is inherited from BaseCard, which renders the ember status dot
    // beside the name and enables Resume/Close actions instead of Launch.

    label: root.app.name || "Unknown"
    // Expose running state on the card itself (mirrors StreamCard's
    // online/session Accessible.description) rather than via a separate a11y node.
    Accessible.description: root.running ? "Running" : ""

    // Resolve the icon imperatively (not via a plain binding) so a recycled
    // ListView delegate can never keep showing the PREVIOUS app's texture — the
    // reported #194 bug (the Plex card rendered the Discord icon). Clearing the
    // source the instant `app` changes drops any retained pixmap before the new
    // icon loads, so a card never shows a neighbour's logo. (Apps whose icon name
    // isn't in the theme still render the provider's placeholder tile rather than
    // the letter — a separate, pre-existing cosmetic gap, tracked for follow-up.)
    //
    // NOTE: AppIcon is used here for the shared icon+fallback rendering, but the
    // imperative _refreshIcon() clears iconWidget.iconSource before re-assigning
    // it — same stale-texture fix applied via the component's property.
    function _refreshIcon() {
        iconWidget.iconSource = "";
        if (root.app && root.app.icon)
            iconWidget.iconSource = root.app.icon;
    }
    onAppChanged: _refreshIcon()
    Component.onCompleted: _refreshIcon()

    AppIcon {
        id: iconWidget
        anchors.fill: parent
        fallbackText: root.app ? root.app.name : ""
    }
}
