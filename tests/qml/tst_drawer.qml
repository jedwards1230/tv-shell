import QtQuick
import QtTest
import components

// Headless regression test for Drawer's clipping contract.
//
// The bug this pins: a closed drawer is only TRANSLATED offscreen
// (`x: -drawerWidth`), it is not hidden. So any child laid out wider than the
// drawer paints past the panel's right edge, and once the panel slides to
// -drawerWidth that overflow lands back at screen x≈0 — visible on the TV while
// the drawer is supposed to be shut. Live symptom: the nav drawer's Resume rail
// sizes tiles so exactly two fit, so a third running app spilled onto the screen.
//
// The fix is `clip: true` on the drawer panel (NOT on the rail — the rail's
// NavigableRow sets `clip: false` on purpose so focus-scaled tiles can bleed).
// These tests assert the panel clips and that an oversized child is contained
// in both the open and closed states.
TestCase {
    id: testCase
    name: "Drawer"
    when: windowShown
    visible: true
    width: 800
    height: 600

    Component {
        id: rigComp
        Item {
            width: 800
            height: 600
            property alias drawer: d
            property alias overflow: wide

            Drawer {
                id: d
                anchors.fill: parent
                edge: "left"
                drawerWidth: 300

                // A child deliberately wider than the drawer — the Resume rail's
                // shape in miniature (its content is wider than the panel).
                Rectangle {
                    id: wide
                    width: 900          // 3x the drawer width
                    height: 80
                    color: "red"
                }
            }
        }
    }

    // Walk up from `item` to `root`, intersecting each ancestor's clip rect, and
    // report whether any painted part of `item` survives. Mirrors what the
    // scene graph actually does with clip flags.
    function visibleWidthWithinClips(item, root) {
        var left = 0;
        var right = item.width;
        var node = item;
        while (node && node !== root) {
            var parent = node.parent;
            if (!parent)
                break;
            // Map the surviving span into the parent's coordinates.
            left += node.x;
            right += node.x;
            if (parent.clip) {
                left = Math.max(left, 0);
                right = Math.min(right, parent.width);
            }
            node = parent;
        }
        // Finally clamp to the drawer root itself — it fills the window, so
        // anything outside [0, width] is off-screen. Without this the closed
        // case reports the 300px span sitting at x ∈ [-300, 0], which is
        // clipped-but-offscreen, i.e. not actually painted to the display.
        left = Math.max(left, 0);
        right = Math.min(right, root.width);
        return Math.max(0, right - left);
    }

    function test_panel_clips() {
        var rig = createTemporaryObject(rigComp, testCase);
        verify(rig, "rig created");
        var panel = rig.overflow.parent;
        verify(panel, "overflow child has a parent panel");
        // The content container and/or the panel above it must clip. Walk up to
        // the Drawer root and require at least one clipping ancestor.
        var clips = false;
        var node = rig.overflow.parent;
        while (node && node !== rig.drawer) {
            if (node.clip) {
                clips = true;
                break;
            }
            node = node.parent;
        }
        verify(clips, "some ancestor between the child and the Drawer root must clip");
    }

    function test_overflow_is_contained_when_open() {
        var rig = createTemporaryObject(rigComp, testCase);
        rig.drawer.opened = true;
        wait(50);
        var visible = visibleWidthWithinClips(rig.overflow, rig.drawer);
        // An open 300-wide drawer may show at most 300px of the 900px child.
        verify(visible <= rig.drawer.drawerWidth, "open drawer must not paint more than drawerWidth of an oversized child, got " + visible);
    }

    function test_overflow_is_hidden_when_closed() {
        var rig = createTemporaryObject(rigComp, testCase);
        rig.drawer.opened = false;
        wait(50);
        // Closed: the panel sits at x=-drawerWidth, so with clipping nothing of
        // the oversized child may remain on screen. This is the exact assertion
        // that fails without `clip: true` — the third Resume tile leaking out.
        var visible = visibleWidthWithinClips(rig.overflow, rig.drawer);
        compare(visible, 0, "a closed drawer must paint none of an oversized child");
    }
}
