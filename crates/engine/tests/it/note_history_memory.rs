use std::sync::Arc;

use gridcraft_core::{CellRef, Value};
use gridcraft_engine::Session;
use serde_json::{Value as Json, json};

fn run(s: &mut Session, command: &str, params: Json) {
    s.execute(command, params).unwrap();
}

fn with_notes(count: u32, text: &str) -> Session {
    let mut s = Session::new();
    s.new_workbook();
    for row in 0..count {
        run(&mut s, "review.newNote", json!({"cell": CellRef::new(row, 0).a1(), "text": text}));
        // Fixture construction does not need to retain intermediate note maps.
        s.doc_mut().unwrap().undo.clear();
    }
    s
}

#[test]
fn unrelated_edits_keep_notes_unchanged() {
    let text = "n".repeat(32_767);
    let mut s = with_notes(64, &text);
    let original = s.doc().unwrap().wb.clone();
    for edit in 1..=80 {
        run(&mut s, "cell.set", json!({"cell": "B1", "input": edit.to_string()}));
    }
    let d = s.doc().unwrap();
    assert_eq!(d.undo.len(), 80);
    for wb in std::iter::once(&d.wb).chain(d.undo.iter().map(|h| &h.wb)) {
        let notes = &wb.active().unwrap().comments;
        assert!(Arc::ptr_eq(notes, &original.active().unwrap().comments));
        assert_eq!(notes.len(), 64);
        for row in 0..64 {
            let at = CellRef::new(row, 0);
            assert_eq!(notes[&at].text, text);
            assert_eq!(notes[&at].text.as_ptr(), original.active().unwrap().comments[&at].text.as_ptr(), "note payload duplicated at {}", at.a1());
        }
    }
    run(&mut s, "edit.undo", json!({"steps": 80}));
    assert_eq!(s.doc().unwrap().wb.active().unwrap().value(CellRef::new(0, 1)), Value::Empty);
    run(&mut s, "edit.redo", json!({"steps": 80}));
    assert_eq!(s.doc().unwrap().wb.active().unwrap().value(CellRef::new(0, 1)), Value::Number(80.0));
    assert_eq!(s.doc().unwrap().wb.active().unwrap().comments, original.active().unwrap().comments);
    for edit in 81..=140 {
        run(&mut s, "cell.set", json!({"cell": "B1", "input": edit.to_string()}));
    }
    let d = s.doc().unwrap();
    assert_eq!(d.undo.len(), 100);
    for wb in std::iter::once(&d.wb).chain(d.undo.iter().map(|h| &h.wb)) {
        assert!(Arc::ptr_eq(&wb.active().unwrap().comments, &original.active().unwrap().comments));
    }
}

#[test]
fn note_edit_isolated_from_history() {
    let mut s = with_notes(2, "original");
    let original = s.doc().unwrap().wb.clone();
    run(&mut s, "review.newNote", json!({"cell": "A1", "text": "changed"}));
    let changed = s.doc().unwrap().wb.clone();
    let assert_versions = |s: &Session, active_text: &str| {
        assert_eq!(original.active().unwrap().comments[&CellRef::new(0, 0)].text, "original");
        assert_eq!(changed.active().unwrap().comments[&CellRef::new(0, 0)].text, "changed");
        let sh = s.doc().unwrap().wb.active().unwrap();
        assert_eq!(sh.comments[&CellRef::new(0, 0)].text, active_text);
        assert_eq!(sh.comments[&CellRef::new(1, 0)].text, "original");
    };
    assert_versions(&s, "changed");
    run(&mut s, "edit.undo", json!({}));
    assert_versions(&s, "original");
    run(&mut s, "edit.redo", json!({}));
    assert_versions(&s, "changed");
}

#[test]
fn operations_outside_notes_keep_payload_shared() {
    let operations = [
        ("edit.clearAll", json!({"range": "B1:C2"})),
        ("edit.clearComments", json!({"range": "B1:C2"})),
        ("review.deleteComment", json!({"range": "B1:C2"})),
        ("review.resolveComment", json!({"cell": "B1"})),
        ("review.resolveComment", json!({"cell": "A1", "resolved": false})),
        ("review.showNote", json!({"cell": "B1"})),
        ("home.insertRows", json!({"rows": "10:10"})),
        ("home.insertCells", json!({"range": "B1", "shift": "down"})),
        ("data.sortAscending", json!({"range": "B1:B2", "header": false})),
        ("data.removeDuplicates", json!({"range": "B1:B2", "header": false})),
        ("edit.copy", json!({"range": "B1,B3"})),
        ("edit.cut", json!({"range": "B1:B2"})),
    ];
    for (command, params) in operations {
        let mut s = with_notes(1, "untouched note");
        run(&mut s, "range.setValues", json!({"range": "B1", "values": [[2], [1], [3]]}));
        let before = s.doc().unwrap().wb.clone();
        run(&mut s, command, params);
        if command == "edit.cut" {
            run(&mut s, "edit.paste", json!({"at": "D1"}));
        }
        let wb = if command == "edit.copy" { &s.clipboard.as_ref().unwrap().wb } else { &s.doc().unwrap().wb };
        assert!(Arc::ptr_eq(&wb.active().unwrap().comments, &before.active().unwrap().comments), "{command}");
        assert_eq!(
            wb.active().unwrap().comments[&CellRef::new(0, 0)].text.as_ptr(),
            before.active().unwrap().comments[&CellRef::new(0, 0)].text.as_ptr(),
            "{command} duplicated unaffected notes"
        );
    }
}
