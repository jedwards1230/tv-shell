// A Base surface is the output. Nothing else in the suite instantiates a real
// window, which is exactly why this bug reached hardware.
//
// THE BUG THIS LANE EXISTS FOR
//
// `Main.qml`'s base Surface set `role`, `appId`, `visible`, `color` and `title`
// and no size, so Qt fell back to a default. Under an ordinary window manager
// that is invisible, because the WM sizes the window for you. Under gamescope
// the client's own size is honoured, so the whole shell rendered tiny and was
// upscaled to fill a 4K panel — a blurry, clipped fragment of a UI that was
// otherwise correct. Found by looking at a television
// (`xprop WM_NORMAL_HINTS` -> "user specified size: 160 by 160"), because every
// other lane builds `HomeScreen` and `DrawerScreen` directly and never creates a
// window at all.
//
// A NOTE ON WHAT THIS LANE CAN AND CANNOT ASSERT
//
// The first version of this file also asserted `size() != QSize(160, 160)`,
// naming the exact number seen on hardware. That assertion was worthless and
// worse than nothing: 160x160 is the **xcb** default, and under the offscreen
// platform an unsized window comes out **1x1**, so the check passed happily with
// the bug fully present — verified by mutation. A test that stays green while
// the defect is live is the failure mode this whole suite is meant to avoid, so
// it is gone rather than adjusted.
//
// What holds on every platform is the relationship: a base surface is AT LEAST
// the size of the screen it is on. That is what is asserted, and it is
// mutation-confirmed to fail when the sizing is removed.
#include "surface.h"

#include <QGuiApplication>
#include <QScreen>
#include <QTest>

using namespace tvshell;

class TstGeometry : public QObject
{
    Q_OBJECT

private Q_SLOTS:

    // The size is applied on the same path as the tags — before `create()` —
    // because under gamescope the size at creation is the size that counts.
    void aBaseSurfaceFillsTheScreen()
    {
        const QScreen *screen = QGuiApplication::primaryScreen();
        QVERIFY(screen);
        QVERIFY(screen->geometry().width() > 0);

        Surface surface;
        surface.setRole(Surface::Base);
        surface.setVisible(true);

        QCOMPARE(surface.size(), screen->geometry().size());
    }

    // The same fact stated as an inequality, which is the form that survives a
    // platform whose screen size is a fiction: whatever else is true, the base
    // window is never a small default sitting inside a large output. This is the
    // shape of the hardware failure — 160x160 inside 3840x2160 — without
    // depending on either number.
    void aBaseSurfaceIsNeverSmallerThanItsOutput()
    {
        const QScreen *screen = QGuiApplication::primaryScreen();
        QVERIFY(screen);

        Surface surface;
        surface.setRole(Surface::Base);
        surface.setVisible(true);

        QVERIFY2(surface.width() >= screen->geometry().width()
                         && surface.height() >= screen->geometry().height(),
                 "the base surface is smaller than its output — under gamescope the "
                 "client's own size is honoured and this gets upscaled to fill the "
                 "display");
    }

    // ---- hiding an overlay destroys it -----------------------------------
    //
    // gamescope keeps compositing an overlay after it unmaps: on hardware,
    // closing the drawer left the television showing it over a base surface that
    // was provably still painting. Destroying the window is the event that
    // actually drops it (the shell exiting is what dislodged the frame), so
    // hiding an overlay has to destroy its platform window rather than just
    // unmap it.
    //
    // That is asserted here as `handle()`, which is the closest thing to
    // "does an X window exist for this" that is reachable without an X server —
    // and it is the same predicate `applyVisibility()` branches on, so a
    // re-show provably goes back through create() + applyTags() rather than
    // re-mapping an untagged window.
    void hidingAnOverlayDestroysItsPlatformWindow()
    {
        Surface overlay;
        overlay.setRole(Surface::Overlay);
        overlay.resize(720, 1080);

        overlay.setVisible(true);
        QVERIFY2(overlay.handle(), "showing an overlay should create a platform window");

        overlay.setVisible(false);
        QVERIFY2(!overlay.handle(),
                 "hiding an overlay must DESTROY its platform window, not just unmap it — "
                 "gamescope goes on compositing an unmapped overlay");

        // And it comes back, through the create-and-tag path rather than a bare
        // re-map. A drawer that opens once is not a drawer.
        overlay.setVisible(true);
        QVERIFY2(overlay.handle(), "re-showing an overlay should create a platform window again");
    }

    // A toast is an overlay too — same atom, same compositing path, no input
    // focus. It was tempting to reason that it is unaffected because it never
    // takes focus; the hardware measurement says the stale frame is NOT
    // focus-related, so that reasoning was wrong and Toast is in scope. A
    // notification that wedges the screen over a live game is worse than a
    // drawer that does, because nobody opened it deliberately.
    void hidingAToastDestroysItToo()
    {
        Surface toast;
        toast.setRole(Surface::Toast);
        toast.resize(880, 130);

        toast.setVisible(true);
        QVERIFY(toast.handle());
        toast.setVisible(false);
        QVERIFY2(!toast.handle(), "hiding a toast must destroy its platform window, as for any overlay");
    }

    // Base is exempt, and that is a role decision rather than an oversight:
    // a base surface is never hidden, and destroying the shell's own root window
    // is not what anyone would want if one ever were.
    void hidingABaseSurfaceDoesNotDestroyIt()
    {
        Surface base;
        base.setRole(Surface::Base);
        base.setVisible(true);
        QVERIFY(base.handle());

        base.setVisible(false);
        QVERIFY2(base.handle(), "a Base surface must keep its platform window when hidden");
    }

    // The role decides, so the roles that legitimately have their own geometry
    // must NOT be overridden. A drawer is a side panel and a toast is small;
    // sizing either to the output would be a different bug with the same cause.
    void anOverlayKeepsTheSizeItWasGiven()
    {
        Surface overlay;
        overlay.setRole(Surface::Overlay);
        overlay.resize(720, 1080);
        overlay.setVisible(true);
        QCOMPARE(overlay.size(), QSize(720, 1080));

        Surface toast;
        toast.setRole(Surface::Toast);
        toast.resize(880, 130);
        toast.setVisible(true);
        QCOMPARE(toast.size(), QSize(880, 130));
    }
};

QTEST_MAIN(TstGeometry)
#include "tst_geometry.moc"
