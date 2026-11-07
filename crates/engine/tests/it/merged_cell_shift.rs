use gridcraft_core::{CellRef, RangeRef};
use gridcraft_engine::Session;
use serde_json::{Value, json};

// Check the entire public document state, including a populated redo stack, on refusal.
pub(super) fn assert_refused_unchanged(s: &mut Session, command: &str, params: Value) {
    s.execute("cell.set", json!({"cell":"Z30", "input":"redo sentinel"})).unwrap();
    s.execute("edit.undo", json!({})).unwrap();
    let before = s.doc().unwrap().clone();
    assert!(!before.redo.is_empty());
    let result = s.execute(command, params);
    let after = s.doc().unwrap();
    assert!(after.wb == before.wb, "failed command changed workbook: {result:?}");
    assert_eq!(after.selection, before.selection);
    assert_eq!(after.sheet_selections, before.sheet_selections);
    assert_eq!(after.revision, before.revision);
    for (actual, expected) in [(&after.undo, &before.undo), (&after.redo, &before.redo)] {
        assert_eq!(actual.len(), expected.len());
        for (a, b) in actual.iter().zip(expected) {
            assert_eq!(a.wb, b.wb);
            assert_eq!(a.selection, b.selection);
            assert_eq!(a.label, b.label);
            assert_eq!(a.retained_bytes, b.retained_bytes);
        }
    }
    assert!(result.is_err(), "unsafe command must refuse");
    s.execute("edit.redo", json!({})).unwrap();
    assert_eq!(s.doc().unwrap().wb.active().unwrap().value(CellRef::parse("Z30").unwrap()).display(), "redo sentinel");
}

fn blank() -> Session {
    let mut s = Session::new();
    s.new_workbook();
    s
}

fn seed70(header: Value) {
    let mut s = Session::new();
    s.execute("file.new", json!({"sample":"budget"})).unwrap();
    s.execute("home.mergeCenter", json!({"range":"C3:C10"})).unwrap();
    s.execute("home.mergeCells", json!({"range":"A11:E17"})).unwrap();
    assert_refused_unchanged(&mut s, "insert.table", json!({"header":header,"name":"Sales","range":"B1:D6","style":"TableStyleMedium2"}));
    let sh = s.doc().unwrap().wb.active().unwrap();
    assert_eq!(sh.merges, [RangeRef::parse("C3:C10").unwrap(), RangeRef::parse("A11:E17").unwrap()]);
    assert!(sh.tables.is_empty());
}

#[test]
fn exact_seed70_refuses_partial_merge_shift() {
    seed70(json!("North"));
}

#[test]
fn boolean_header_false_refuses_partial_merge_shift() {
    seed70(json!(false));
}

#[test]
fn partial_merge_shift_preserves_anchor_without_an_overlapping_merge() {
    let mut s = blank();
    s.execute("cell.set", json!({"cell":"C11","input":"anchor sentinel"})).unwrap();
    s.execute("home.mergeCells", json!({"range":"C11:E12"})).unwrap();
    assert_refused_unchanged(&mut s, "insert.table", json!({"range":"B1:D6","header":false}));
    let sh = s.doc().unwrap().wb.active().unwrap();
    assert_eq!(sh.value(CellRef::parse("C11").unwrap()).display(), "anchor sentinel");
    assert!(sh.value(CellRef::parse("C12").unwrap()).is_empty());
    assert_eq!(sh.merges, [RangeRef::parse("C11:E12").unwrap()]);
}

fn partial_shift(command: &str, shift: &str, range: &str, merge: &str, anchor: &str) {
    let mut s = blank();
    s.execute("cell.set", json!({"cell":anchor,"input":"anchor sentinel"})).unwrap();
    s.execute("home.mergeCells", json!({"range":merge})).unwrap();
    assert_refused_unchanged(&mut s, command, json!({"range":range,"shift":shift}));
    assert_eq!(s.doc().unwrap().wb.active().unwrap().value(CellRef::parse(anchor).unwrap()).display(), "anchor sentinel");
}

#[test]
fn insert_down_refuses_partial_merge() {
    partial_shift("home.insertCells", "down", "B1:D1", "C11:E12", "C11");
}
#[test]
fn delete_up_refuses_partial_merge() {
    partial_shift("home.deleteCells", "up", "B1:D1", "C11:E12", "C11");
}
#[test]
fn insert_right_refuses_partial_merge() {
    partial_shift("home.insertCells", "right", "A2:A4", "K3:L5", "K3");
}
#[test]
fn delete_left_refuses_partial_merge() {
    partial_shift("home.deleteCells", "left", "A2:A4", "K3:L5", "K3");
}

#[test]
fn full_span_and_unaffected_merges_shift_with_content_and_undo_redo() {
    for (command, params, merge, anchor, want_merge, want_anchor) in [
        ("insert.table", json!({"range":"B1:D6","header":false}), "C11:D12", "C11", "C12:D13", "C12"),
        ("home.insertCells", json!({"range":"A2:A4","shift":"right"}), "K3:L4", "K3", "L3:M4", "L3"),
        ("home.deleteCells", json!({"range":"B1:D1","shift":"up"}), "C11:D12", "C11", "C10:D11", "C10"),
        ("home.deleteCells", json!({"range":"A2:A4","shift":"left"}), "K3:L4", "K3", "J3:K4", "J3"),
        ("insert.table", json!({"range":"B4:D6","header":false}), "A1:E2", "A1", "A1:E2", "A1"),
        ("insert.table", json!({"range":"B1:D6","header":false}), "F11:G12", "F11", "F11:G12", "F11"),
    ] {
        let mut s = blank();
        s.execute("cell.set", json!({"cell":anchor,"input":"carried"})).unwrap();
        s.execute("home.mergeCells", json!({"range":merge})).unwrap();
        let before = s.doc().unwrap().wb.clone();
        s.execute(command, params).unwrap();
        let after = s.doc().unwrap().wb.clone();
        let sh = after.active().unwrap();
        assert_eq!(sh.merges, [RangeRef::parse(want_merge).unwrap()]);
        assert_eq!(sh.value(CellRef::parse(want_anchor).unwrap()).display(), "carried");
        s.execute("edit.undo", json!({})).unwrap();
        assert_eq!(s.doc().unwrap().wb, before);
        s.execute("edit.redo", json!({})).unwrap();
        assert_eq!(s.doc().unwrap().wb, after);
    }
}
