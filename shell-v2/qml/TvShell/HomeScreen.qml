// The home screen. Renders the model; owns none of it.
//
// Everything on this screen is a pure function of two inputs — the catalog and
// the core's last `screen-state` snapshot — computed by `homeModel.build()`.
// This file contains no idea of what is running, no launch bookkeeping and no
// cache. That is the whole of "the core owns state": if the screen looks wrong,
// either the snapshot is wrong or `homeModel.js` is, and the second is testable
// without a television.
//
// FOCUS
//
// One router for the whole screen. A rail is a row, a card is a column, and no
// component names a neighbour. Rails appear and disappear as apps start and
// stop — Continue is not there at all when nothing is running — and that costs
// nothing, because rows are recomputed from the live set on every move.
//
// The empty state is a FOCUSABLE card, not a label. `homeModel` H8 guarantees it
// is present whenever every rail is empty, so the screen always contains at
// least one focusable cell and `focusUnplaceable` cannot fire here. That is the
// screen-level half of the stranding guarantee.
pragma ComponentBehavior: Bound

import QtQuick

FocusScope {
    id: home

    // { rails: [...], empty: bool } from homeModel.build().
    required property var model
    required property FocusRouter router
    // The core's id for what is on screen, or -1 when unknown. Shown, not acted
    // on: nothing here decides anything from it.
    property int onScreenAppId: -1
    // "" when the core is connected. Rendered as a quiet line, not a dialog: a
    // shell that cannot reach the core is still a shell, and a modal would make
    // it a shell you cannot dismiss.
    property string coreError: ""

    signal activated(var entry)

    Rectangle {
        anchors.fill: parent
        color: Tokens.background
    }

    Column {
        anchors.fill: parent
        anchors.topMargin: Tokens.spaceXL
        spacing: Tokens.spaceL

        // ---- header -------------------------------------------------------
        Item {
            width: parent.width
            height: clock.height

            Text {
                id: clock

                x: Tokens.spaceXL
                text: home.clockText
                color: Tokens.textPrimary
                font.pixelSize: Tokens.fontDisplay
            }

            Column {
                anchors.right: parent.right
                anchors.rightMargin: Tokens.spaceXL
                anchors.verticalCenter: clock.verticalCenter
                spacing: Tokens.spaceXS

                Text {
                    anchors.right: parent.right
                    text: home.onScreenAppId >= 0 ? ("On screen: " + home.onScreenAppId) : "Nothing on screen"
                    color: Tokens.textSecondary
                    font.pixelSize: Tokens.fontCaption
                }

                Text {
                    anchors.right: parent.right
                    text: home.coreError
                    visible: home.coreError !== ""
                    color: Tokens.ember
                    font.pixelSize: Tokens.fontCaption
                }
            }
        }

        // ---- rails --------------------------------------------------------
        Repeater {
            model: home.model.rails

            Rail {
                id: railItem

                required property int index
                required property var modelData

                width: home.width
                railData: railItem.modelData
                rowIndex: railItem.index
                router: home.router

                onActivated: entry => home.activated(entry)
            }
        }

        // ---- empty state --------------------------------------------------
        Item {
            width: parent.width
            height: Tokens.cardHeight
            visible: home.model.empty

            Card {
                x: Tokens.spaceXL
                // Row 0 is safe: when this is visible there are no rails, so no
                // rail owns row 0. When rails exist this card is invisible, and
                // FocusRouter.cells() treats an invisible slot as unfocusable —
                // the same "two ways to say not now, one meaning" the router
                // documents — so it never competes for a row it does not own.
                slotId: "empty"
                row: 0
                column: 0
                router: home.router
                width: Tokens.cardWidth * 2

                title: "No apps configured"
                subtitle: home.coreError !== "" ? "The core is unreachable." : "Add apps to ~/.config/tv-shell/shell.json"
            }
        }
    }

    // ---- clock ------------------------------------------------------------
    // The one timer in the shell, and it drives nothing but this string. It is
    // NOT a poll of anything: no state is fetched here, and removing it would
    // change only what the clock reads.
    property string clockText: ""

    function tick() {
        home.clockText = Qt.formatTime(new Date(), "h:mm");
    }

    Component.onCompleted: home.tick()

    readonly property Timer clockTimer: Timer {
        interval: 10000
        running: true
        repeat: true
        onTriggered: home.tick()
    }
}
