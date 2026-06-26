import QtQuick
import "lib"

// Now-playing media widget (#22) — the standalone full-card renderer. Kept as a
// thin MprisPlayerBase host wrapping the shared NowPlayingCard visual, so callers
// that want the card directly (e.g. SessionQAM's Now-Playing tab) instantiate one
// component with the full MPRIS + focus contract. The home screen no longer uses
// this directly — it uses the size-switching NowPlayingWidget, which reuses the
// same NowPlayingCard / NowPlayingStripView visuals.
MprisPlayerBase {
    id: root
    contentCard: card

    NowPlayingCard {
        id: card
        base: root
    }
}
