import QtQuick

// Minimal navigation model for the secondary-screen layer. Home is the
// always-present base; at most one secondary surface (Library or Settings) is
// active over it. This centralizes the imperative show/hide + focus handoff that
// the host (ShellLayout) previously duplicated at every call site (home
// QuickActions, nav drawer, overlay drawer, shell.qml intents).
//
// Scope guard — what this does NOT own:
//   - Visibility/focus *bindings*: HomeScreen.visible / .focus stay declarative,
//     derived from the surfaces' own `.visible` (not from `active`). The router
//     drives the imperative transitions only.
//   - Modal/overlay back-handling (notification center, power, QAM, nav drawer)
//     and the Settings-internal B-stack (page -> sidebar -> Home). Those keep
//     their existing handlers. The router reacts to each surface's `closed`
//     signal; it never intercepts Escape.
//
// Gating (idle / shell state) stays the caller's responsibility, matching the
// pre-router call sites: shell.qml intent handlers keep their `state === "idle"`
// guard; the overlay-drawer path transitions to idle first, then pushes.
QtObject {
    id: mgr

    // Wired by the host — the manager toggles these, never creates them.
    property var libraryScreen
    property var settingsApp
    property var widgetsScreen
    property var homeFocusTimer   // host's debounced "refocus Home" timer

    // Which screen owns the secondary layer. Informational single-source-of-truth
    // for now (no binding depends on it yet) + a hook for future screens.
    property string active: "home"   // "home" | "library" | "settings" | "widgets"
    signal screenChanged(string id)

    // push(id, params) — show a secondary screen.
    //   "settings": params.page (optional) deep-links a page; returns the
    //               openPage() bool so the caller can log an unknown slug.
    //   "library":  opens the browse surface; returns true.
    //   anything else: falls back to Home.
    function push(id, params) {
        if (id === "settings") {
            let ok = true;
            if (params && params.page !== undefined && params.page !== "")
                ok = settingsApp.openPage(params.page);
            else
                settingsApp.open();
            active = "settings";
            screenChanged(active);
            return ok;
        }
        if (id === "library") {
            libraryScreen.visible = true;
            libraryScreen.forceActiveFocus();
            libraryScreen.focusDefaultPosition();
            active = "library";
            screenChanged(active);
            return true;
        }
        if (id === "widgets") {
            widgetsScreen.visible = true;
            widgetsScreen.forceActiveFocus();
            if (params && params.target)
                widgetsScreen.applyDeepTarget(params.target);
            else
                widgetsScreen.focusFirst();
            active = "widgets";
            screenChanged(active);
            return true;
        }
        popToHome();
        return true;
    }

    // popToHome(refocus) — dismiss whatever secondary screen is up and return to
    // Home. close()/visible=false are idempotent, so this is a quiet no-op when
    // already home. Pass refocus=false from the leave-idle safety net (the state
    // machine owns focus when not idle); the default refocuses Home.
    function popToHome(refocus) {
        if (settingsApp && settingsApp.visible)
            settingsApp.close();
        if (libraryScreen)
            libraryScreen.visible = false;
        if (widgetsScreen)
            widgetsScreen.visible = false;
        active = "home";
        screenChanged(active);
        if (refocus !== false && homeFocusTimer)
            homeFocusTimer.restart();
    }

    // closeSettings() — ensure Settings is dismissed without touching Library or
    // refocusing (the host's reset paths handle focus themselves). Faithful to the
    // old imperative `settingsApp.visible = false`, but keeps `active` honest.
    function closeSettings() {
        if (settingsApp && settingsApp.visible)
            settingsApp.close();
        if (active === "settings") {
            active = "home";
            screenChanged(active);
        }
    }
}
