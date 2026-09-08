# tv-shell v2 shell.
#
# This is the build step v1's shell deliberately does not have. See
# docs/V2_SHELL.md for why it is now unavoidable (pre-map X11 tagging needs C++)
# and what it costs (v2's shell is no longer rsync-deployable).
#
# ONE QML module, one URI. v1 has eleven qmldir registries and 69 relative-dir
# imports; that is the thing this file exists to make impossible. Everything
# under qml/TvShell/ is `import TvShell`, there is no hand-written qmldir, and
# there is no second module to add a type to.
cmake_minimum_required(VERSION 3.21)

project(tv-shell-v2 VERSION 0.1.0 LANGUAGES CXX)

set(CMAKE_CXX_STANDARD 17)
set(CMAKE_CXX_STANDARD_REQUIRED ON)

# Network is for QLocalSocket only: the shell speaks to the core over a Unix
# socket and to nothing else. There is no HTTP client here and there must not be.
find_package(Qt6 6.5 REQUIRED COMPONENTS Core Gui Network Qml Quick)
qt_standard_project_setup(REQUIRES 6.5)

# XCB is a hard requirement, not an optional extra: without it there is no
# pre-map tagging and therefore no v2 shell. Failing at configure time is the
# honest outcome.
find_package(PkgConfig REQUIRED)
pkg_check_modules(XCB REQUIRED IMPORTED_TARGET xcb)

# ---------------------------------------------------------------------------
# tvshell-pure — the pure decision layer.
#
# Deliberately its own target with NO Qt GUI and NO X11 in its link line, so its
# unit test is a plain executable that runs anywhere: no display server, no
# platform plugin, no compositor. This is v1's `.pragma library` discipline
# expressed in CMake — if this target ever gains a GUI or X dependency, the split
# has been lost and the test stops being cheap enough to always run.
# ---------------------------------------------------------------------------
# (Renamed from tvshell-tags when path resolution joined it: the name should say
# what the target IS -- everything decidable without a display -- not name one of
# the things in it.)
add_library(tvshell-pure STATIC src/surfacetags.cpp src/paths.cpp)
target_include_directories(tvshell-pure PUBLIC ${CMAKE_CURRENT_SOURCE_DIR}/src)
set_target_properties(tvshell-pure PROPERTIES POSITION_INDEPENDENT_CODE ON)

# ---------------------------------------------------------------------------
# The single QML module. A library rather than an app-embedded module so the
# QML test executable can import the SAME module the shell ships — no assembled
# copy of the production files, which is how v1's headless suite has to do it.
# ---------------------------------------------------------------------------
qt_add_library(tvshell-qml STATIC)

# Flatten the module's resource paths. Without an alias, `qml/TvShell/Main.qml`
# lands at `:/qt/qml/TvShell/qml/TvShell/Main.qml` — the source layout leaks into
# every import path, and a test that reaches focusGraph.js directly has to spell
# the doubled prefix. With the alias, the module directory on disk and the module
# directory in the resource tree are the same shape: `:/qt/qml/TvShell/<file>`.
foreach(_qmlfile
        Main.qml HomeScreen.qml DrawerScreen.qml Rail.qml Card.qml Tokens.qml
        FocusRouter.qml FocusSlot.qml
        focusGraph.js catalog.js homeModel.js viewport.js)
    set_source_files_properties(qml/TvShell/${_qmlfile}
        PROPERTIES QT_RESOURCE_ALIAS ${_qmlfile})
endforeach()

# Tokens is the module's only singleton. Declared here rather than in a qmldir
# because there IS no hand-written qmldir -- the generated one carries it. It has
# to be set BEFORE qt_add_qml_module: set afterwards it is silently ignored, and
# the symptom is every `Tokens.*` reading `undefined` at runtime with no error
# beyond a pile of "Unable to assign [undefined]" warnings.
set_source_files_properties(qml/TvShell/Tokens.qml PROPERTIES QT_QML_SINGLETON_TYPE TRUE)

qt_add_qml_module(tvshell-qml
    URI TvShell
    VERSION 1.0
    # Must end in the URI's path or qmllint cannot resolve the module and the
    # `all_qmllint` target silently checks nothing useful. CI builds that target.
    OUTPUT_DIRECTORY ${CMAKE_BINARY_DIR}/qml/TvShell
    QML_FILES
        qml/TvShell/Main.qml
        qml/TvShell/HomeScreen.qml
        qml/TvShell/DrawerScreen.qml
        qml/TvShell/Rail.qml
        qml/TvShell/Card.qml
        qml/TvShell/Tokens.qml
        qml/TvShell/FocusRouter.qml
        qml/TvShell/FocusSlot.qml
    RESOURCES
        qml/TvShell/focusGraph.js
        qml/TvShell/catalog.js
        qml/TvShell/homeModel.js
        qml/TvShell/viewport.js
    SOURCES
        src/surface.h
        src/surface.cpp
        src/x11tagger.h
        src/x11tagger.cpp
        src/coreclient.h
        src/coreclient.cpp
        src/shellconfig.h
        src/shellconfig.cpp
)


target_link_libraries(tvshell-qml
    PUBLIC Qt6::Gui Qt6::Quick Qt6::Network tvshell-pure
    PRIVATE PkgConfig::XCB
)
# ---------------------------------------------------------------------------
# The application.
# ---------------------------------------------------------------------------
qt_add_executable(tv-shell-v2 src/main.cpp)
target_link_libraries(tv-shell-v2 PRIVATE Qt6::Gui Qt6::Quick tvshell-qmlplugin)

include(GNUInstallDirs)
install(TARGETS tv-shell-v2 RUNTIME DESTINATION ${CMAKE_INSTALL_BINDIR})

# ---------------------------------------------------------------------------
# Tests. Three lanes, in increasing order of what they need to run:
#
#   tags     pure C++, no display, no Qt GUI          — always runs
#   qml      offscreen QML, no display server         — always runs
#   premap   real X server, asserts property-vs-map   — TV_SHELL_TEST_XVFB only
#
# The gating on the third mirrors core/'s TV_SHELL_TEST_XVFB convention, and CI
# sets that variable so the leg actually executes. An ignore-gated leg wired to
# nothing defends nothing (jedwards1230/tv-shell#469).
# ---------------------------------------------------------------------------
# ---------------------------------------------------------------------------
# qmllint, strictly -- for the one category that survives contact with both Qt
# versions in play.
#
# The generated `all_qmllint` target runs qmllint with its DEFAULTS, and the
# defaults are weaker than they look: an unqualified access passes it silently.
# That matters here more than in most Qt projects because there is no screenshot
# path on a v2 session -- a binding that resolves to the wrong scope is a value
# that is quietly never set, on a screen nobody can look at.
#
# So this target re-runs the SAME response file -- same include paths, same
# resources, no second list of files to drift -- with `unqualified` promoted and
# `--max-warnings 0`, so any warning at all fails. CI builds it.
#
# WHY `missing-property` IS *NOT* ENABLED, THOUGH IT WOULD BE THE BIGGER CATCH
#
# It cannot be, portably. qmllint does not resolve the C++ `Surface` type out of
# this static QML module on the Qt 6.8 that CI pins -- it reports
# "Type Surface is used but it is not resolved" -- and every property on a
# Surface then reads as missing, so Main.qml fails six times while being
# correct. Naming the module's qmltypes explicitly with `-i` does not fix it and
# makes it worse: on Qt 6.11, where discovery works by itself, `-i` breaks the
# same resolution and adds a cascade of spurious `unqualified` warnings.
#
# Both were measured, not assumed. The category is therefore left at its default
# (it still runs under `all_qmllint`, non-fatally) and the gap is recorded in
# docs/V2_SHELL.md rather than papered over with a per-file exclusion that would
# rot. `unresolved-type` is silenced here because on 6.8 it is that same Qt
# limitation talking, not a defect in the QML.
# ---------------------------------------------------------------------------
qt_query_qml_module(tvshell-qml)
find_program(QMLLINT_EXECUTABLE NAMES qmllint qmllint6 HINTS ${QT6_INSTALL_PREFIX}/${QT6_INSTALL_BINS})

add_custom_target(qmllint_strict
    COMMAND ${QMLLINT_EXECUTABLE}
        --unqualified warning
        --unresolved-type disable
        --max-warnings 0
        "@${CMAKE_BINARY_DIR}/.rcc/qmllint/tvshell-qml.rsp"
    DEPENDS all_qmllint
    WORKING_DIRECTORY ${CMAKE_CURRENT_SOURCE_DIR}
    COMMENT "Running qmllint strictly (unqualified access, zero warnings tolerated)"
    VERBATIM
)

option(TVSHELL_V2_TESTS "Build the shell-v2 test lanes" ON)

if(TVSHELL_V2_TESTS)
    enable_testing()
    add_subdirectory(tests)
endif()
