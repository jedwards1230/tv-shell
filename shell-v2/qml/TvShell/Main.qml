// The shell's composition root: three toplevels, one core connection, and the
// wiring between them. No layout and no decisions — those are in the screens and
// in the pure modules respectively.
//
// WHY THE THREE SURFACES ARE SIBLINGS
//
// A Window declared inside a Window becomes a transient child (WM_TRANSIENT_FOR),
// which is a different thing to gamescope than an independent toplevel. §5/§7
// require the drawer and toasts to be their own toplevels, so they are siblings
// of a non-visual root and each declares its own role.
//
// This also settles input routing with no code: gamescope gives keyboard focus
// to the overlay when one is up (STEAM_INPUT_FOCUS), so the base surface simply
// stops receiving keys while the drawer is open. There is no focus stack in this
// process, no "who owns input" flag, and no way for the two to disagree —
// because the compositor is the only thing that decides, and it decides for us.
//
// WHEN STATE IS FETCHED, AND WHY THERE IS NO TIMER
//
// `core/src/protocol.rs` states there is deliberately no event stream yet, so
// there is nothing to subscribe to. The shell asks for a snapshot at the three
// moments that can have changed it — the connection coming up, one of our own
// commands completing, and the drawer closing — and is otherwise silent. That is
// not polling, and a timer here would be inventing liveness the core does not
// publish. The consequence is honest and worth stating: an app that exits on its
// own leaves a stale "Running" badge until the next of those moments. The fix
// belongs in the core, not in a poll here.
pragma ComponentBehavior: Bound

import QtQuick
import TvShell
import "catalog.js" as Catalog
import "homeModel.js" as HomeModel
import "viewport.js" as Viewport

QtObject {
    id: root

    // The id the base window tags itself with; also the id the home model
    // filters out of its rails so the shell never offers to switch to itself.
    readonly property int shellAppId: 9001

    property var catalogEntries: []
    property var snapshot: null
    property bool drawerOpen: false
    property string toastText: ""

    readonly property var homeModel: HomeModel.build(root.catalogEntries, root.snapshot, root.shellAppId)

    // ---- backends ---------------------------------------------------------

    readonly property ShellConfig config: ShellConfig {
        onCatalogTextChanged: {
            const parsed = Catalog.parse(root.config.catalogText);
            for (const problem of parsed.problems)
                console.warn("catalog:", problem);
            root.catalogEntries = parsed.entries;
        }
    }

    readonly property CoreClient core: CoreClient {
        onConnectedChanged: if (root.core.connected)
            root.refresh()
        onReplyReceived: (command, reply) => root.onReply(command, reply)
        // A refusal is a bug in this file, not a transport fault, so it is loud
        // and it is visible on the television rather than only in a journal.
        onRequestRefused: (command, reason) => {
            console.warn("core refused:", command, "--", reason);
            root.toast(reason);
        }
    }

    // ---- core conversation ------------------------------------------------

    function refresh() {
        root.core.request("screen-state");
    }

    function onReply(command: string, reply: string) {
        if (reply.startsWith("error:") || reply === "unknown") {
            root.toast(reply);
            return;
        }
        if (command === "screen-state") {
            try {
                root.snapshot = JSON.parse(reply);
            } catch (e) {
                // A snapshot we cannot parse is worse than none: `homeModel`
                // treats null as "the core told us nothing" (H6) and keeps the
                // screen usable, whereas a half-parsed object would render
                // confidently wrong.
                console.warn("screen-state did not parse:", e);
                root.snapshot = null;
            }
            return;
        }
        // `show`, `launch` and `home` all change what is on screen, and none of
        // them reports the new state. So the snapshot is re-read after each,
        // which is the closest thing to an event the core currently offers.
        root.refresh();
    }

    function activate(entry: var) {
        const command = HomeModel.commandFor(entry);
        if (command === "")
            return;
        root.core.request(command);
    }

    function toast(text: string) {
        root.toastText = text;
        toastTimer.restart();
    }

    readonly property Timer toastTimer: Timer {
        id: toastTimer

        interval: 4000
        onTriggered: root.toastText = ""
    }

    // ---- routers ----------------------------------------------------------
    // One per surface, because each surface takes input independently. They
    // never interact: a router knows only the slots registered with it.

    readonly property FocusRouter homeRouter: FocusRouter {}
    readonly property FocusRouter drawerRouter: FocusRouter {}

    // ---- base surface -----------------------------------------------------

    readonly property Surface base: Surface {
        id: baseSurface

        role: Surface.Base
        appId: root.shellAppId
        visible: true
        color: Tokens.background
        title: "tv-shell"

        // Tokens has exactly one writer, and this is it.
        onHeightChanged: Tokens.scale = Viewport.scaleFor(baseSurface.height)
        Component.onCompleted: Tokens.scale = Viewport.scaleFor(baseSurface.height)

        HomeScreen {
            anchors.fill: parent
            focus: true

            model: root.homeModel
            router: root.homeRouter
            onScreenAppId: {
                const id = HomeModel.onScreenId(root.snapshot);
                return id === null ? -1 : id;
            }
            coreError: root.core.connected ? "" : (root.core.lastError !== "" ? root.core.lastError : "core not connected")

            Keys.onLeftPressed: root.homeRouter.move("left")
            Keys.onRightPressed: root.homeRouter.move("right")
            Keys.onUpPressed: root.homeRouter.move("up")
            Keys.onDownPressed: root.homeRouter.move("down")
            Keys.onMenuPressed: root.drawerOpen = true

            onActivated: entry => root.activate(entry)
        }
    }

    // ---- overlay surface (the drawer) -------------------------------------

    readonly property Surface drawer: Surface {
        id: drawerSurface

        role: Surface.Overlay
        visible: root.drawerOpen
        width: Tokens.drawerWidth
        height: 1080
        color: Tokens.scrim
        title: "tv-shell-drawer"

        // Closing the drawer is a moment the screen may have changed underneath
        // it — the user may have pressed Home. Re-read rather than assume.
        onVisibleChanged: if (!drawerSurface.visible && root.core.connected)
            root.refresh()

        DrawerScreen {
            anchors.fill: parent
            focus: true

            router: root.drawerRouter

            Keys.onUpPressed: root.drawerRouter.move("up")
            Keys.onDownPressed: root.drawerRouter.move("down")
            Keys.onEscapePressed: root.drawerOpen = false
            Keys.onBackPressed: root.drawerOpen = false
            Keys.onMenuPressed: root.drawerOpen = false

            onGoHome: {
                root.core.request("home");
                root.drawerOpen = false;
            }
            onReloadCatalog: {
                root.config.reload();
                root.drawerOpen = false;
            }
            onDismissed: root.drawerOpen = false
        }
    }

    // ---- toast surface ----------------------------------------------------

    readonly property Surface toastSurface: Surface {
        role: Surface.Toast
        visible: root.toastText !== ""
        width: Tokens.cardWidth * 2
        height: Tokens.cardHeight / 2
        color: Tokens.surface
        title: "tv-shell-toast"

        Text {
            anchors.centerIn: parent
            anchors.margins: Tokens.spaceM
            width: parent.width - 2 * Tokens.spaceM
            text: root.toastText
            color: Tokens.textPrimary
            font.pixelSize: Tokens.fontCaption
            elide: Text.ElideRight
        }
    }

    Component.onCompleted: {
        root.config.reload();
        root.core.connectToCore();
    }
}
