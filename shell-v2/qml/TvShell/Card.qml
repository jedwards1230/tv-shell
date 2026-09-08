// A card: the shell's one focusable thing.
//
// Every activatable element in v2 is this type. That is deliberate and is the
// answer to "consistency and reliability": there is one focus ring, drawn once,
// at one width, in one colour, and a screen cannot introduce a second focus
// treatment without adding a second type — which review would see.
//
// It is a FocusSlot, so it declares WHERE IT SITS and never who is next to it.
// The only thing it adds to a slot is a look and an `activated` signal.
//
// A DISABLED CARD STAYS VISIBLE AND STOPS BEING FOCUSABLE. That combination is
// the point: `slotEnabled: false` removes it from the focus graph — the router
// re-homes if it was focused — while it stays on screen greyed, so the layout
// does not reflow under the user's hands. v1 conflated the two and every
// "focus stranded" bug lived in that gap.
pragma ComponentBehavior: Bound

import QtQuick

FocusSlot {
    id: card

    property string title: ""
    property string subtitle: ""
    // "" means "use the theme default". A card never invents a colour; the
    // catalog may name one, and only then does it differ.
    property string accent: ""
    // A badge string, or "" for none. Used for "Running" today.
    property string badge: ""

    signal activated

    width: Tokens.cardWidth
    height: Tokens.cardHeight

    // A/Enter activates. B/Escape is deliberately NOT handled here: back is a
    // screen-level decision (which surface closes, what is behind it), and a
    // card that swallowed it would make that decision from the wrong altitude.
    Keys.onReturnPressed: event => card.maybeActivate(event)
    Keys.onEnterPressed: event => card.maybeActivate(event)
    Keys.onSpacePressed: event => card.maybeActivate(event)

    // A disabled card must not fire even if focus somehow reaches it — the model
    // should make that impossible, and this makes the consequence impossible too.
    function maybeActivate(event: var) {
        if (!card.slotEnabled) {
            event.accepted = false;
            return;
        }
        event.accepted = true;
        card.activated();
    }

    Rectangle {
        anchors.fill: parent
        radius: Tokens.radius
        color: card.slotEnabled ? Tokens.surfaceRaised : Tokens.surface
        border.width: card.current ? Tokens.focusRingWidth : 1
        border.color: card.current ? Tokens.focusRing : Tokens.surfaceBorder

        Behavior on border.color {
            ColorAnimation {
                duration: Tokens.durationFast
            }
        }

        // The accent stripe is the only place a per-card colour appears, so a
        // catalog accent can never end up behind text and ruin its contrast.
        Rectangle {
            width: Tokens.spaceXS
            height: parent.height - 2 * Tokens.spaceM
            anchors.left: parent.left
            anchors.leftMargin: Tokens.spaceM
            anchors.verticalCenter: parent.verticalCenter
            radius: width / 2
            visible: card.accent !== ""
            color: card.accent !== "" ? card.accent : Tokens.ember
        }

        Column {
            anchors.left: parent.left
            anchors.leftMargin: Tokens.spaceM + (card.accent !== "" ? Tokens.spaceM : 0)
            anchors.right: parent.right
            anchors.rightMargin: Tokens.spaceM
            anchors.verticalCenter: parent.verticalCenter
            spacing: Tokens.spaceXS

            Text {
                width: parent.width
                text: card.title
                color: card.slotEnabled ? Tokens.textPrimary : Tokens.textMuted
                font.pixelSize: Tokens.fontBody
                elide: Text.ElideRight
                maximumLineCount: 2
                wrapMode: Text.WordWrap
            }

            Text {
                width: parent.width
                text: card.subtitle
                visible: card.subtitle !== ""
                color: Tokens.textSecondary
                font.pixelSize: Tokens.fontCaption
                elide: Text.ElideRight
            }
        }

        // The running badge. Green rather than crimson on purpose: crimson means
        // "this is where you are", and a running app you are not focused on is
        // not that.
        Row {
            anchors.top: parent.top
            anchors.right: parent.right
            anchors.margins: Tokens.spaceM
            spacing: Tokens.spaceXS
            visible: card.badge !== ""

            Rectangle {
                width: Tokens.spaceS
                height: width
                radius: width / 2
                anchors.verticalCenter: parent.verticalCenter
                color: Tokens.online
            }

            Text {
                text: card.badge
                color: Tokens.textSecondary
                font.pixelSize: Tokens.fontCaption
            }
        }
    }
}
