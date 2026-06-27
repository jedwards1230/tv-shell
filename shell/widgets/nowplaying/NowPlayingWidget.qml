import QtQuick
import "../../components"
import "../../components/lib"

// Unified home-screen Now-Playing widget (#249) — ONE widget with a `size` that
// selects which visual renders, replacing the former pair of complementary
// NowPlayingStrip / MediaWidget instances (each gated on the size). A single
// MprisPlayerBase host owns the MPRIS selection, capability guards, transport
// activation, and the home-tile focus contract; an internal Loader swaps the
// pure-visual renderer by size:
//   small  = NowPlayingStripView (compact transport strip)
//   medium = NowPlayingCard      (full card + progress bar) — default
// Both reuse the same MprisPlayerBase-derived focus stop, so the widget is a
// single focusable region in the home vertical chain.
MprisPlayerBase {
    id: root
    contentCard: npLoader

    Loader {
        id: npLoader
        width: parent.width
        sourceComponent: root.size === "small" ? stripComp : cardComp
    }

    Component {
        id: cardComp
        NowPlayingCard {
            base: root
        }
    }

    Component {
        id: stripComp
        NowPlayingStripView {
            base: root
        }
    }
}
