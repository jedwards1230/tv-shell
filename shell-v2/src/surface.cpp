#include "surface.h"

#include <QScreen>
#include "x11tagger.h"

#include <QLoggingCategory>
#include <QVariantMap>

Q_DECLARE_LOGGING_CATEGORY(lcTag)

namespace tvshell {
namespace {

// The shell's app id is private to the deployment (docs/V2_DESIGN.md §5): under
// `--steam`, 769 is the Steam client's own id and carries gamescope's
// `window_is_steam` behaviour, so the shell must not use it. 9001 is the id the
// measured prototype used (`dev/gamescope/launch.sh`), kept here so the shim and
// the bench agree. The env var name is the prototype's, for the same reason.
constexpr uint kDefaultShellAppId = 9001;

uint defaultShellAppId()
{
    static const uint value = [] {
        bool ok = false;
        const uint fromEnv = qEnvironmentVariableIntValue("TV_SHELL_GS_SHELL_APPID", &ok);
        return (ok && fromEnv != 0) ? fromEnv : kDefaultShellAppId;
    }();
    return value;
}

} // namespace

// m_complete starts true: a Surface built from C++ has no parser status to wait
// for, so setVisible() acts immediately. classBegin() clears it for the QML path.
Surface::Surface(QWindow *parent) : QQuickWindow(parent), m_appId(defaultShellAppId()) { }

bool Surface::frozen(const char *what) const
{
    if (!handle())
        return false;
    qCWarning(lcTag, "ignoring %s change: surface already created (role decides tagging at map)",
              what);
    return true;
}

void Surface::setRole(Role role)
{
    if (m_role == role || frozen("role"))
        return;
    m_role = role;
    Q_EMIT roleChanged();
}

void Surface::setAppId(uint appId)
{
    if (m_appId == appId || frozen("appId"))
        return;
    m_appId = appId;
    Q_EMIT appIdChanged();
}

QVariantMap Surface::plannedTags() const
{
    QVariantMap out;
    for (const SurfaceTag &tag : tagsForRole(pureRole(), m_appId))
        out.insert(QString::fromLatin1(tag.name), tag.value);
    return out;
}

void Surface::setVisible(bool visible)
{
    m_wantVisible = visible;
    // Before componentComplete() the role may not have been assigned yet, so
    // showing now could tag with a stale role. Hold the request; componentComplete
    // replays it. Outside QML (a C++-constructed Surface, as in the tests) there
    // is no parser status to wait for, so m_complete is set in the constructor.
    if (m_complete)
        applyVisibility();
}

void Surface::componentComplete()
{
    m_complete = true;
    applyVisibility();
}

void Surface::applyVisibility()
{
    if (!m_wantVisible) {
        QQuickWindow::setVisible(false);

        // AN OVERLAY MUST BE DESTROYED, NOT MERELY UNMAPPED.
        //
        // gamescope goes on compositing an overlay after it unmaps. Measured on
        // hardware 2026-09-08: close the drawer and the television keeps showing
        // it, over a base surface that is provably still painting (its own window
        // contents advance — the clock ticks — while the output does not).
        //
        // Every lever that addresses the layer UNDERNEATH was tried and none of
        // them helped, which is what makes this the overlay's own problem rather
        // than a focus or base-layer one: the core's `show 9001` base-layer write,
        // `show 9003` (switching the base layer to a different app entirely),
        // `xdotool windowfocus` and `windowactivate` all left the frame byte for
        // byte identical. `windowfocus` DID restore X input focus without
        // unfreezing anything, which is what separates the two symptoms: the
        // dangling focus and the stale frame are independent, and only the second
        // one is this.
        //
        // What did change the output was the shell exiting — i.e. its windows
        // being DESTROYED. So destruction is the event gamescope acts on, and
        // unmapping is not. Qt's `destroy()` issues XDestroyWindow, which is the
        // same X-level event without the process having to die.
        //
        // Base is exempt because a base surface is never hidden; if one ever is,
        // destroying the shell's own root window is not the behaviour anyone
        // wants.
        //
        // The QML scene is NOT destroyed by this: `QWindow::destroy()` releases
        // the platform window and the scene graph's GPU resources, and leaves the
        // QQuickItem tree — every FocusSlot, and its registration with the router
        // — alive. Re-showing costs a scene-graph rebuild, not a re-instantiation,
        // so nothing re-registers and no focus state is lost.
        if (m_role != Base && handle()) {
            destroy();
            // The tags died with the window. Say so, rather than leaving a stale
            // `tagged: true` claiming properties are on a window that is gone —
            // the next show goes through create() + applyTags() again precisely
            // because `handle()` is now null.
            if (m_tagged) {
                m_tagged = false;
                Q_EMIT taggedChanged();
            }
        }
        return;
    }

    // A BASE SURFACE IS THE OUTPUT. Sized here, before `create()`, for the same
    // reason the tags are written here: under gamescope the client's own size is
    // honoured, so whatever the window is created at is what the compositor
    // scales to fill the screen.
    //
    // Qt's default for a QWindow that was never given a size is 160x160. Under an
    // ordinary window manager you never see that, because the WM sizes the window
    // for you. Under gamescope nothing does, so the shell rendered at 160x160 and
    // was upscaled 24x to 3840x2160 — a blurry, clipped fragment of a correct UI.
    // Measured on hardware 2026-09-08 (`xprop WM_NORMAL_HINTS` -> "user specified
    // size: 160 by 160"), which is the only way it could have been found: every
    // offscreen lane builds the screens directly and never instantiates a window.
    //
    // It is a property of the ROLE, not of the caller, so it lives here rather
    // than at the one call site that got it wrong. A base surface has no
    // legitimate size other than the output's; an Overlay or Toast does, and is
    // therefore left alone.
    //
    // ON THE NULL BRANCH, AND WHY IT IS LOUD.
    //
    // `screen()` can legitimately return null — a QWindow is not guaranteed to
    // have one, and on a platform with no screens it will not. Every platform
    // this shell actually runs on supplies one (xcb from the X server, and the
    // offscreen plugin synthesizes an 800x800 screen, which is what the geometry
    // lane asserts against), so this branch is not expected to be taken.
    //
    // But "not expected" is exactly what the 160x160 window was. Silently
    // skipping the sizing leaves the window at Qt's default, which is the same
    // defect reached through a different door — and the reason that one survived
    // to a television is that nothing said anything. So it warns rather than
    // returning quietly: if this ever fires, the next person gets a sentence
    // instead of a mystery.
    if (m_role == Base && !handle()) {
        if (const QScreen *s = screen()) {
            setGeometry(s->geometry());
        } else {
            qCWarning(lcTag,
                      "base surface has no QScreen: leaving it at Qt's default size (%dx%d). "
                      "Under gamescope the client's own size is honoured, so this will be "
                      "upscaled to fill the display.",
                      width(), height());
        }
    }

    // The three-step ordering this whole class exists for. `create()` issues X
    // CreateWindow without mapping; the base `setVisible(true)` below issues
    // MapWindow. Everything between the two is guaranteed to reach the server
    // first, because it goes out on the same connection.
    if (!handle())
        create();

    const bool ok = applyTags(static_cast<std::uint32_t>(winId()), tagsForRole(pureRole(), m_appId));
    if (ok != m_tagged) {
        m_tagged = ok;
        Q_EMIT taggedChanged();
    }

    QQuickWindow::setVisible(true);
}

} // namespace tvshell
