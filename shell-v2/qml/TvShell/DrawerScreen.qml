// The overlay drawer's content: a vertical list of actions.
//
// It exists in this PR to prove the overlay toplevel is a real, usable, focusable
// surface rather than a coloured rectangle — the same three actions would
// otherwise have had to live on the home screen, where they would have made the
// base surface the only thing that ever takes input and left §7's separate-
// toplevel requirement untested by anything.
//
// It shares the shell's ONE focusable type. A drawer row is a Card in a single
// column, with the same ring, in the same colour, at the same width — so the
// drawer cannot drift from the home screen's focus treatment without someone
// changing Card, which is the point of having one.
pragma ComponentBehavior: Bound

import QtQuick

FocusScope {
    id: drawer

    required property FocusRouter router

    signal goHome
    signal reloadCatalog
    signal dismissed

    // Every row, as data. A row is one focus cell; adding one is one object here
    // and nothing else — no neighbour to wire, no index to renumber.
    readonly property var actions: [
        {
            "id": "home",
            "title": "Home",
            "subtitle": "Return to the shell"
        },
        {
            "id": "reload",
            "title": "Reload apps",
            "subtitle": "Re-read shell.json"
        },
        {
            "id": "close",
            "title": "Close",
            "subtitle": "Back to where you were"
        }
    ]

    Rectangle {
        anchors.fill: parent
        color: Tokens.surface
    }

    Column {
        anchors.fill: parent
        anchors.margins: Tokens.spaceL
        spacing: Tokens.spaceM

        Text {
            text: "Menu"
            color: Tokens.textPrimary
            font.pixelSize: Tokens.fontTitle
        }

        Repeater {
            model: drawer.actions

            Card {
                id: rowCard

                required property int index
                required property var modelData

                slotId: "drawer:" + rowCard.modelData.id
                row: rowCard.index
                column: 0
                router: drawer.router

                width: drawer.width - 2 * Tokens.spaceL
                title: rowCard.modelData.title
                subtitle: rowCard.modelData.subtitle

                onActivated: drawer.dispatch(rowCard.modelData.id)
            }
        }
    }

    // One place maps a row id to an effect. A row that grew its own handler
    // would be a row whose behaviour is invisible from the list above.
    function dispatch(id: string) {
        if (id === "home")
            drawer.goHome();
        else if (id === "reload")
            drawer.reloadCatalog();
        else
            drawer.dismissed();
    }
}
