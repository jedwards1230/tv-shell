find_package(Qt6 6.5 REQUIRED COMPONENTS Test QuickTest)

# --- lane 1: the pure mapping. No display, no Qt GUI, no X. -----------------
qt_add_executable(tst_surfacetags tst_surfacetags.cpp)
target_link_libraries(tst_surfacetags PRIVATE Qt6::Test tvshell-pure)
add_test(NAME surfacetags COMMAND tst_surfacetags)

# --- lane 1b: path resolution + the core client, over a REAL socket. --------
# Needs an event loop but no display: QLocalServer/QLocalSocket are Qt Network,
# not Qt GUI. Asserts K1..K4 against a fake core speaking the real framing rather
# than against a mock of CoreClient, so a change to the framing fails here.
qt_add_executable(tst_coreclient tst_coreclient.cpp)
target_link_libraries(tst_coreclient PRIVATE Qt6::Test Qt6::Network tvshell-qml)
add_test(NAME coreclient COMMAND tst_coreclient)
set_tests_properties(coreclient PROPERTIES ENVIRONMENT "QT_QPA_PLATFORM=offscreen")

# --- lane 1c: a real window, offscreen. ------------------------------------
# The ONLY lane that instantiates a Surface. Every other lane builds the screens
# directly, which is why a base window created at Qt's default 160x160 reached
# hardware before anything noticed. Needs a platform plugin but no display.
qt_add_executable(tst_geometry tst_geometry.cpp)
target_link_libraries(tst_geometry PRIVATE Qt6::Test Qt6::Quick tvshell-qml)
add_test(NAME geometry COMMAND tst_geometry)
# QT_FATAL_WARNINGS here too, and for a reason specific to this lane: it is the
# only one that builds a real Surface, so it is the only one positioned to catch
# the C++ side's own silent-degradation warnings ("base surface has no QScreen",
# "ignoring role change: surface already created") -- each of which describes a
# window that renders, looks plausible, and is wrong. The `coreclient` lane is
# deliberately NOT included: it provokes a warning on purpose, and see below for
# why that cannot be ignored away.
set_tests_properties(geometry PROPERTIES ENVIRONMENT
    "QT_QPA_PLATFORM=offscreen;QT_QUICK_BACKEND=software;QT_FATAL_WARNINGS=1")

# --- lane 2: the focus graph + the QML types, offscreen. -------------------
# Imports the SAME TvShell module the shell ships (linked, not copied), so there
# is no drift between what is tested and what runs.
qt_add_executable(tst_qml qml_main.cpp)
target_link_libraries(tst_qml PRIVATE Qt6::QuickTest Qt6::Qml tvshell-qmlplugin)
add_test(NAME qml COMMAND tst_qml -input ${CMAKE_CURRENT_SOURCE_DIR}/qml)
# QT_FATAL_WARNINGS is the compensating control for the qmllint gap above: an
# undefined QML binding is a *warning*, never an error, so without this a typo
# like `Tokens.radiusTypo` renders wrong and the lane still reports green.
# Mutation-confirmed both ways -- see jedwards1230/tv-shell#491, where it was
# documented in five places and set in none.
#
# IT IS NOT COMPATIBLE WITH ignoreWarning(). The fatal check lives in
# QMessageLogger::warning AFTER the installed handler has run, so QTest's ignore
# list suppresses the log line and the process still aborts. A lane that runs
# under this variable must therefore emit no warnings at all -- which is why the
# tagger reports "no X connection" at info level on a non-xcb platform, where
# the offscreen tests provoke it by construction and main() has already warned
# once at startup.
set_tests_properties(qml PROPERTIES ENVIRONMENT
    "QT_QPA_PLATFORM=offscreen;QT_QUICK_BACKEND=software;QT_FATAL_WARNINGS=1")

# --- lane 3: the ordering assertion, against a real X server. --------------
# Needs an X display, so it is registered only when TV_SHELL_TEST_XVFB names one.
# Same opt-in shape as core/'s atoms_xvfb.rs. The variable is read at CONFIGURE
# time on purpose: a lane that silently registers and then skips itself is
# indistinguishable from a lane that passed, which is the failure mode #469 was
# about. Configure without it and `ctest` reports 2 tests; configure with it and
# it reports 3.
qt_add_executable(tst_premap tst_premap.cpp)
target_link_libraries(tst_premap PRIVATE Qt6::Test Qt6::Quick tvshell-qml PkgConfig::XCB)

if(DEFINED ENV{TV_SHELL_TEST_XVFB} AND NOT "$ENV{TV_SHELL_TEST_XVFB}" STREQUAL "")
    message(STATUS "shell-v2: premap lane ENABLED on DISPLAY=$ENV{TV_SHELL_TEST_XVFB}")
    add_test(NAME premap COMMAND tst_premap)
    set_tests_properties(premap PROPERTIES ENVIRONMENT
        "QT_QPA_PLATFORM=xcb;QT_QUICK_BACKEND=software;DISPLAY=$ENV{TV_SHELL_TEST_XVFB}")
else()
    message(STATUS "shell-v2: premap lane SKIPPED (set TV_SHELL_TEST_XVFB=:99 to enable)")
endif()
