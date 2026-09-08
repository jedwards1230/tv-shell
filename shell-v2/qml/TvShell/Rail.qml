// A horizontal rail of cards with a header. The shell's one row primitive.
//
// It owns exactly one thing beyond layout: keeping the focused card in view. In
// v1 that was `ensureVisibleRequested`, a signal every widget had to remember to
// emit and every host had to remember to connect — one of the eight duck-typed
// members. Here the rail WATCHES the router instead. Nothing asks to be scrolled
// to; the rail notices that one of its own cards is the current one and moves.
// A card added tomorrow gets the behaviour without knowing it exists.
//
// The arithmetic is in `viewport.js`, so "does focusing the last card scroll
// exactly far enough and no further" is answerable without a window.
pragma ComponentBehavior: Bound

import QtQuick
import "viewport.js" as Viewport

Item {
    id: rail

    // { id, title, entries: [...] } from homeModel.build().
    //
    // NOT named `rail`: a property shadows the outer `id` of the same name
    // inside every delegate below, so `rail.rail` would resolve to null exactly
    // where it matters. The same trap is documented in tst_focusgraph.qml.
    required property var railData
    // This rail's row in the focus graph. Cards take their column from their
    // index, so the whole rail's participation is these two numbers.
    required property int rowIndex
    required property FocusRouter router

    signal activated(var entry)

    implicitHeight: header.height + Tokens.spaceS + Tokens.cardHeight

    Text {
        id: header

        x: Tokens.spaceXL
        text: rail.railData.title
        color: Tokens.textPrimary
        font.pixelSize: Tokens.fontTitle
    }

    Flickable {
        id: flick

        anchors.left: parent.left
        anchors.right: parent.right
        y: header.height + Tokens.spaceS
        height: Tokens.cardHeight
        contentWidth: strip.width + 2 * Tokens.spaceXL
        contentHeight: height
        // The rail is driven by focus, never by a drag: on a couch there is no
        // pointer, and a flick that could leave the focused card off-screen
        // would put the two sources of truth in conflict.
        interactive: false
        clip: true

        Behavior on contentX {
            NumberAnimation {
                duration: Tokens.durationBase
                easing.type: Easing.OutCubic
            }
        }

        Row {
            id: strip

            x: Tokens.spaceXL
            spacing: Tokens.spaceM

            Repeater {
                model: rail.railData.entries

                Card {
                    id: cardItem

                    required property int index
                    required property var modelData

                    // The id must be unique across the WHOLE router, not just
                    // this rail, so it carries the rail id. Two rails can hold
                    // the same app — Continue and Apps routinely do — and a bare
                    // app id would collide, which focusGraph.problems() would
                    // report but only after the graph was already ambiguous.
                    slotId: rail.railData.id + ":" + cardItem.modelData.appId
                    row: rail.rowIndex
                    column: cardItem.index
                    router: rail.router

                    title: cardItem.modelData.title
                    subtitle: cardItem.modelData.subtitle
                    accent: cardItem.modelData.accent
                    badge: cardItem.modelData.running ? "Running" : ""

                    onActivated: rail.activated(cardItem.modelData)

                    // The scroll-follow. Watching `current` rather than being
                    // told to scroll is what removes the host-side wiring.
                    onCurrentChanged: if (cardItem.current)
                        rail.reveal(cardItem)
                }
            }
        }
    }

    // Bring `item` into view, one pure computation and one assignment.
    function reveal(item: Item) {
        flick.contentX = Viewport.scrollOffset(flick.contentX, item.x + strip.x, item.width, flick.width, flick.contentWidth, Tokens.spaceXL);
    }
}
