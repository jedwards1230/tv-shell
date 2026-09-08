// catalog.js, headless. Pins C1..C5.
//
// docs/V2_SHELL.md records which mutation of catalog.js each assertion caught.
import QtQuick
import QtTest
import "qrc:/qt/qml/TvShell/catalog.js" as Catalog

Item {
    id: harness

    function json(obj) {
        return JSON.stringify(obj);
    }

    TestCase {
        name: "Catalog"

        // --- C1: never throws, never null ----------------------------------

        function test_malformed_json_is_an_empty_catalog_with_a_problem() {
            const r = Catalog.parse("{ this is not json");
            compare(r.entries.length, 0);
            compare(r.problems.length, 1);
            verify(r.problems[0].indexOf("not valid JSON") >= 0);
        }

        function test_a_non_object_document_is_reported() {
            compare(Catalog.parse("[1,2,3]").entries.length, 0);
            compare(Catalog.parse("[1,2,3]").problems.length, 1);
            compare(Catalog.parse("42").entries.length, 0);
        }

        function test_apps_must_be_an_array() {
            const r = Catalog.parse(harness.json({
                "apps": {
                    "id": 1
                }
            }));
            compare(r.entries.length, 0);
            compare(r.problems.length, 1);
        }

        // --- C4: absence is not an error ------------------------------------
        // Deliberately distinct from C1: "no catalog" and "a broken catalog"
        // must not produce the same output, or a typo looks like an empty box.

        function test_absence_is_silent() {
            const texts = ["", "   ", "\n"];
            for (let i = 0; i < texts.length; ++i) {
                const r = Catalog.parse(texts[i]);
                compare(r.entries.length, 0, "entries for " + JSON.stringify(texts[i]));
                compare(r.problems.length, 0, "problems for " + JSON.stringify(texts[i]));
            }
            // An object with no `apps` key is a configured-but-empty catalog,
            // which is also not a mistake.
            compare(Catalog.parse("{}").entries.length, 0);
            compare(Catalog.parse("{}").problems.length, 0);
        }

        // --- C2: one bad row does not discard the good ones ------------------

        function test_a_bad_row_drops_only_itself() {
            const r = Catalog.parse(harness.json({
                "apps": [
                    {
                        "id": 9003,
                        "title": "Moonlight"
                    },
                    {
                        "title": "no id"
                    },
                    {
                        "id": 9004
                    },
                    {
                        "id": 9005,
                        "title": "Steam"
                    }
                ]
            }));
            compare(r.entries.length, 2);
            compare(r.entries[0].appId, 9003);
            compare(r.entries[1].appId, 9005);
            compare(r.problems.length, 2);
        }

        function test_an_id_must_be_a_whole_non_negative_number() {
            const bad = [1.5, -1, "9003", null, 4294967296];
            for (let i = 0; i < bad.length; ++i) {
                const r = Catalog.parse(harness.json({
                    "apps": [
                        {
                            "id": bad[i],
                            "title": "x"
                        }
                    ]
                }));
                compare(r.entries.length, 0, "id " + JSON.stringify(bad[i]) + " should be rejected");
            }
            // The lower boundary itself is valid.
            compare(Catalog.parse(harness.json({
                "apps": [
                    {
                        "id": 0,
                        "title": "x"
                    }
                ]
            })).entries.length, 1);
        }

        function test_optional_fields_default_rather_than_drop_the_row() {
            const r = Catalog.parse(harness.json({
                "apps": [
                    {
                        "id": 1,
                        "title": "T"
                    }
                ]
            }));
            compare(r.entries.length, 1);
            compare(r.entries[0].subtitle, "");
            // "" and not a colour: the theme owns the default, so a catalog that
            // names no accent cannot pin one here.
            compare(r.entries[0].accent, "");
        }

        // --- C3: first wins on a duplicate ----------------------------------

        function test_duplicate_id_keeps_the_first() {
            const r = Catalog.parse(harness.json({
                "apps": [
                    {
                        "id": 7,
                        "title": "First"
                    },
                    {
                        "id": 7,
                        "title": "Second"
                    }
                ]
            }));
            compare(r.entries.length, 1);
            compare(r.entries[0].title, "First");
            compare(r.problems.length, 1);
            verify(r.problems[0].indexOf("duplicate") >= 0);
        }

        // --- C5: file order is preserved ------------------------------------

        function test_order_is_file_order() {
            const r = Catalog.parse(harness.json({
                "apps": [
                    {
                        "id": 30,
                        "title": "C"
                    },
                    {
                        "id": 10,
                        "title": "A"
                    },
                    {
                        "id": 20,
                        "title": "B"
                    }
                ]
            }));
            compare(r.entries.length, 3);
            compare(r.entries[0].appId, 30);
            compare(r.entries[1].appId, 10);
            compare(r.entries[2].appId, 20);
        }
    }
}
