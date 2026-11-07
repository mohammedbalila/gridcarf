//! Public command boundaries own one before/result checkpoint, including partial errors.
use std::sync::Arc;

use gridcraft_model::Workbook;
use serde_json::{Value as Json, json};

use super::{s, v};
use crate::Session;

fn workbook(s: &Session) -> Arc<Workbook> {
    s.doc().unwrap().wb.clone()
}

fn set(s: &mut Session, cell: &str, input: &str) {
    s.execute("cell.set", json!({"cell": cell, "input": input})).unwrap();
}

fn two_undos_two_redos(s: &mut Session, base: &Workbook, before: &Workbook, after: &Workbook) {
    for (id, expected) in [("edit.undo", before), ("edit.undo", base), ("edit.redo", before), ("edit.redo", after)] {
        s.execute(id, json!({})).unwrap();
        assert!(workbook(s).as_ref() == expected, "{id} must reach the independent action boundary");
    }
}

fn alias_case(id: &str, setup: impl FnOnce(&mut Session), params: Json, check: impl FnOnce(&Session)) {
    let mut s = s();
    setup(&mut s);
    let base = workbook(&s);
    set(&mut s, "Z1", "prior");
    let before = workbook(&s);
    s.execute(id, params).unwrap();
    check(&s);
    assert_eq!(s.doc().unwrap().undo.last().unwrap().label, crate::find_command(id).unwrap().label.trim_end_matches('…'));
    let after = workbook(&s);
    assert_ne!(before.as_ref(), after.as_ref(), "fixture must change workbook content or formatting");
    two_undos_two_redos(&mut s, &base, &before, &after);
}

#[test]
fn owned_symbol_history() {
    alias_case("insert.symbol", |_| {}, json!({"char":"€"}), |s| assert_eq!(v(s, "A1").display(), "€"));
}

#[test]
fn owned_border_history() {
    alias_case(
        "home.drawBorder",
        |_| {},
        json!({"range":"A1", "edge":"bottom", "style":"thin"}),
        |s| {
            assert_ne!(s.doc().unwrap().wb.active().unwrap().style_id(gridcraft_core::CellRef::default()), Default::default());
        },
    );
}

#[test]
fn owned_theme_fonts_history() {
    alias_case(
        "pageLayout.themeFonts",
        |_| {},
        json!({"major":"Synthetic Serif", "minor":"Synthetic Sans"}),
        |s| {
            assert_eq!(workbook(s).theme.major_font, "Synthetic Serif");
            assert_eq!(workbook(s).theme.minor_font, "Synthetic Sans");
        },
    );
}

#[test]
fn owned_duplicates_history() {
    alias_case(
        "table.removeDuplicates",
        |s| {
            s.execute("range.setValues", json!({"range":"A1", "values":[["Name"],["same"],["same"]]})).unwrap();
            s.execute("insert.table", json!({"range":"A1:A3", "name":"Items", "header":true})).unwrap();
        },
        json!({"table":"Items"}),
        |s| assert!(v(s, "A3").is_empty()),
    );
}

#[test]
fn owned_spelling_history() {
    alias_case(
        "review.changeSpelling",
        |s| set(s, "A1", "helo"),
        json!({"cell":"A1", "word":"helo", "to":"hello"}),
        |s| {
            assert_eq!(v(s, "A1").display(), "hello");
        },
    );
}

#[test]
fn owned_chart_type_history() {
    alias_case(
        "chart.changeType",
        |s| {
            s.execute("range.setValues", json!({"range":"A1", "values":[["x", "y"],[1,2],[2,3]]})).unwrap();
            s.execute("insert.chart", json!({"range":"A1:B3", "type":"column"})).unwrap();
        },
        json!({"chart":1, "type":"line"}),
        |s| assert_eq!(workbook(s).active().unwrap().charts[0].kind, gridcraft_model::ChartKind::Line),
    );
}

#[test]
fn owned_flash_fill_history() {
    alias_case(
        "data.flashFill",
        |s| {
            s.execute("range.setValues", json!({"range":"A1", "values":[["2024-01-15", "15/01/2024"],["2023-12-03", null]]})).unwrap();
            s.execute("selection.set", json!({"cell":"B2"})).unwrap();
        },
        json!({}),
        |s| assert_eq!(v(s, "B2").display(), "03/12/2023"),
    );
}

#[test]
fn owned_import_history_never_resurrects_an_intermediate_sheet() {
    alias_case(
        "data.fromTableRange",
        |s| {
            s.execute("range.setValues", json!({"range":"A1", "values":[["Item", "Count"],["a",1],["b",2]]})).unwrap();
        },
        json!({"range":"A1:B3"}),
        |s| {
            let wb = workbook(s);
            assert_eq!(wb.sheets.len(), 2);
            assert_eq!(wb.active().unwrap().tables.len(), 1);
            assert_eq!(v(s, "B3").display(), "2");
        },
    );
}

fn script(s: &mut Session, steps: Json) {
    s.execute("automate.newScript", json!({"name":"Owned", "commands":steps})).unwrap();
}

#[test]
fn owned_multistep_script_history() {
    alias_case(
        "automate.runScript",
        |s| {
            script(
                s,
                json!([
                    {"command":"cell.set", "params":{"cell":"A1", "input":"1"}},
                    {"command":"cell.set", "params":{"cell":"A2", "input":"2"}}
                ]),
            )
        },
        json!({"name":"Owned", "trusted":true}),
        |s| {
            assert_eq!(v(s, "A1").display(), "1");
            assert_eq!(v(s, "A2").display(), "2");
        },
    );
}

#[test]
fn owned_failed_script_history_keeps_partial_results() {
    let mut s = s();
    script(
        &mut s,
        json!([
            {"command":"cell.set", "params":{"cell":"A1", "input":"partial"}},
            {"command":"cell.set", "params":{"cell":"not a cell", "input":"invalid"}}
        ]),
    );
    let base = workbook(&s);
    set(&mut s, "Z1", "prior");
    let before = workbook(&s);
    assert!(s.execute("automate.runScript", json!({"name":"Owned", "trusted":true})).is_err());
    assert_eq!(v(&s, "A1").display(), "partial");
    let after = workbook(&s);
    two_undos_two_redos(&mut s, &base, &before, &after);
}

#[test]
fn owned_script_history_restores_entries_consumed_by_child_undo() {
    let mut s = s();
    script(
        &mut s,
        json!([
            {"command":"edit.undo", "params":{"steps":2}},
            {"command":"cell.set", "params":{"cell":"A1", "input":"after undo"}}
        ]),
    );
    set(&mut s, "W1", "older");
    let base = workbook(&s);
    set(&mut s, "Z1", "prior");
    let before = workbook(&s);
    s.execute("automate.runScript", json!({"name":"Owned", "trusted":true})).unwrap();
    assert!(v(&s, "Z1").is_empty());
    assert!(v(&s, "W1").is_empty());
    assert_eq!(v(&s, "A1").display(), "after undo");
    let after = workbook(&s);
    two_undos_two_redos(&mut s, &base, &before, &after);
}

#[test]
fn owned_script_history_absorbs_child_redo() {
    let mut s = s();
    script(
        &mut s,
        json!([
            {"command":"edit.redo", "params":{}},
            {"command":"cell.set", "params":{"cell":"A1", "input":"after redo"}}
        ]),
    );
    let base = workbook(&s);
    set(&mut s, "Z1", "prior");
    let before = workbook(&s);
    set(&mut s, "B1", "redo target");
    s.execute("edit.undo", json!({})).unwrap();
    s.execute("automate.runScript", json!({"name":"Owned", "trusted":true})).unwrap();
    assert_eq!(v(&s, "B1").display(), "redo target");
    assert_eq!(v(&s, "A1").display(), "after redo");
    let after = workbook(&s);
    two_undos_two_redos(&mut s, &base, &before, &after);
}

#[test]
fn split_retains_one_delegated_freeze_or_unfreeze_step() {
    for initially_frozen in [false, true] {
        let mut s = s();
        s.execute("selection.set", json!({"cell":"C3"})).unwrap();
        if initially_frozen {
            s.execute("view.freezePanes", json!({"cell":"C3"})).unwrap();
        }
        let base = workbook(&s);
        set(&mut s, "Z1", "prior");
        let before = workbook(&s);
        assert!(!crate::find_command("view.split").unwrap().undoable);
        s.execute("view.split", json!({})).unwrap();
        assert_eq!(workbook(&s).active().unwrap().freeze, if initially_frozen { None } else { Some((2, 2)) });
        assert_eq!(s.doc().unwrap().undo.last().unwrap().label, if initially_frozen { "Unfreeze Panes" } else { "Freeze Panes" });
        let after = workbook(&s);
        two_undos_two_redos(&mut s, &base, &before, &after);
    }
}

#[test]
fn owned_noop_publication_and_prepublication_error_keep_distinct_history_rules() {
    let mut s = s();
    let base = workbook(&s);
    set(&mut s, "Z1", "prior");
    let before = workbook(&s);
    // Flash Fill commits before reporting that no examples matched. Equal workbook values
    // still publish an Arc, so the partial-error owner retains one (not two) undo steps.
    assert!(s.execute("data.flashFill", json!({})).is_err());
    let after = workbook(&s);
    assert!(!Arc::ptr_eq(&before, &after));
    assert_eq!(before, after);
    two_undos_two_redos(&mut s, &base, &before, &after);
    s.execute("edit.undo", json!({})).unwrap();
    let before = workbook(&s);
    let undo = s.doc().unwrap().undo.len();
    let redo = s.doc().unwrap().redo.len();
    assert!(s.execute("automate.runScript", json!({"name":"Absent"})).is_err());
    assert!(Arc::ptr_eq(&before, &workbook(&s)));
    assert_eq!((s.doc().unwrap().undo.len(), s.doc().unwrap().redo.len()), (undo, redo));
    s.execute("edit.redo", json!({})).unwrap();
    assert_eq!(workbook(&s), after);
}

#[test]
fn nonundoable_dispatcher_retains_each_owned_child_action() {
    crate::cmd::register_test_command("test.nestedHistoryDispatcher", |s, _| {
        s.execute("insert.symbol", json!({"char":"€"}))?;
        s.execute("home.bold", json!({"range":"A1"}))
    });
    let mut s = s();
    let base = workbook(&s);
    set(&mut s, "Z1", "prior");
    let before = workbook(&s);
    s.execute("test.nestedHistoryDispatcher", json!({})).unwrap();
    let after = workbook(&s);
    assert_eq!(s.doc().unwrap().undo.len(), 3);
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(v(&s, "A1").display(), "€");
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(workbook(&s), before);
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(workbook(&s), base);
    s.execute("edit.redo", json!({"steps":3})).unwrap();
    assert_eq!(workbook(&s), after);
}

#[test]
fn owned_history_follows_original_document_while_other_children_keep_theirs() {
    crate::cmd::register_undoable_test_command("test.nestedHistoryAcrossDocuments", |s, p| {
        s.execute("insert.symbol", json!({"char":"€"}))?;
        s.execute("window.activate", p.clone())?;
        s.execute("insert.symbol", json!({"char":"£"}))
    });
    let mut s = s();
    let original = s.doc().unwrap().uid;
    let base = workbook(&s);
    set(&mut s, "Z1", "original prior");
    let before = workbook(&s);
    s.new_workbook();
    let other = s.doc().unwrap().uid;
    let other_base = workbook(&s);
    set(&mut s, "Z1", "other prior");
    let other_before = workbook(&s);
    s.execute("window.activate", json!({"doc":original})).unwrap();
    s.execute("test.nestedHistoryAcrossDocuments", json!({"doc":other})).unwrap();
    assert_eq!(s.doc().unwrap().uid, other);
    assert_eq!(v(&s, "A1").display(), "£");
    let other_after = workbook(&s);
    two_undos_two_redos(&mut s, &other_base, &other_before, &other_after);
    s.execute("window.activate", json!({"doc":original})).unwrap();
    assert_eq!(v(&s, "A1").display(), "€");
    let after = workbook(&s);
    two_undos_two_redos(&mut s, &base, &before, &after);
}

#[test]
fn closing_owned_document_does_not_replace_the_next_documents_history() {
    crate::cmd::register_undoable_test_command("test.nestedHistoryClose", |s, _| {
        s.execute("insert.symbol", json!({"char":"€"}))?;
        s.execute("file.close", json!({"force":true}))
    });
    let mut s = s();
    let survivor = s.doc().unwrap().uid;
    let base = workbook(&s);
    set(&mut s, "Z1", "survivor");
    let before = workbook(&s);
    s.new_workbook();
    let closed = s.doc().unwrap().uid;
    s.execute("test.nestedHistoryClose", json!({})).unwrap();
    assert!(s.documents().iter().all(|d| d.uid != closed));
    assert_eq!(s.doc().unwrap().uid, survivor);
    assert_eq!(workbook(&s), before);
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(workbook(&s), base);
    s.execute("edit.redo", json!({})).unwrap();
    assert_eq!(workbook(&s), before);
}

#[test]
fn owned_history_collapses_bounded_child_tail_at_full_history() {
    crate::cmd::register_undoable_test_command("test.nestedHistoryLong", |s, _| {
        let start = s.doc()?.undo.len();
        for _ in 0..crate::UNDO_LIMIT + 50 {
            s.execute("insert.symbol", json!({"char":"€"}))?;
            assert!(s.doc()?.undo.len() <= start + crate::UNDO_LIMIT);
        }
        Ok(json!({}))
    });
    let mut s = s();
    for i in 0..crate::UNDO_LIMIT - 1 {
        set(&mut s, "Z1", &i.to_string());
    }
    let base = workbook(&s);
    set(&mut s, "Z1", "prior");
    let before = workbook(&s);
    let older: Vec<_> = s.doc().unwrap().undo.iter().skip(1).map(|e| e.wb.clone()).collect();
    s.execute("test.nestedHistoryLong", json!({})).unwrap();
    let d = s.doc().unwrap();
    assert_eq!(d.undo.len(), crate::UNDO_LIMIT);
    assert!(d.undo.iter().take(crate::UNDO_LIMIT - 1).zip(&older).all(|(entry, wb)| Arc::ptr_eq(&entry.wb, wb)));
    assert_eq!(d.undo.last().unwrap().label, "test.nestedHistoryLong");
    let after = workbook(&s);
    assert_eq!(v(&s, "A1").display(), "€".repeat(crate::UNDO_LIMIT + 50));
    two_undos_two_redos(&mut s, &base, &before, &after);
}

#[test]
fn owned_history_keeps_the_latest_oversized_step_after_bounding_child_bytes() {
    crate::cmd::register_undoable_test_command("test.nestedHistoryPayload", |s, p| {
        crate::cmd::edit(s, |cx| {
            // Retention accounts for capacity, without needing to fill hundreds of MiB in a test.
            let mut payload = String::with_capacity(crate::UNDO_BYTES_LIMIT + 1);
            payload.push_str(p["text"].as_str().unwrap());
            cx.wb.props.description = payload;
            Ok(json!({}))
        })
    });
    crate::cmd::register_undoable_test_command("test.nestedHistoryPayloadOwner", |s, _| {
        let start = s.doc()?.undo.len();
        for text in ["first", "second", "last"] {
            s.execute("test.nestedHistoryPayload", json!({"text":text}))?;
            let d = s.doc()?;
            assert_eq!(d.undo.len(), start + 1, "only the latest oversized child step survives the tail budget");
            assert!(d.undo.last().unwrap().retained_bytes > crate::UNDO_BYTES_LIMIT);
        }
        Ok(json!({}))
    });
    let mut s = s();
    s.execute("test.nestedHistoryPayload", json!({"text":"before"})).unwrap();
    let before = workbook(&s);
    s.execute("test.nestedHistoryPayloadOwner", json!({})).unwrap();
    let after = workbook(&s);
    assert_eq!(after.props.description, "last");
    let d = s.doc().unwrap();
    assert_eq!(d.undo.len(), 1);
    assert!(d.undo[0].retained_bytes > crate::UNDO_BYTES_LIMIT);
    assert_eq!(d.undo[0].label, "test.nestedHistoryPayloadOwner");
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(workbook(&s), before);
    s.execute("edit.redo", json!({})).unwrap();
    assert_eq!(workbook(&s), after);
}
