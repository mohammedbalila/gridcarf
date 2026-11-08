//! Command producers preserve full selections by numeric runtime identity.
use gridcraft_core::{CellRef, RangeRef, Value};
use gridcraft_engine::{Selection, Session};
use serde_json::{Value as Json, json};

fn run(s: &mut Session, id: &str, p: Json) -> Json {
    s.execute(id, p).unwrap_or_else(|e| panic!("{id}: {e}"))
}
fn selection(n: u32) -> Selection {
    let anchor = CellRef::new(n, 1);
    let active = CellRef::new(n + 3, 5);
    Selection { anchor, active, ranges: vec![RangeRef::new(anchor, CellRef::new(n + 1, 2)), RangeRef::new(CellRef::new(n + 2, 4), active)] }
}
fn session() -> Session {
    let mut s = Session::new();
    s.new_workbook();
    s
}
fn activate(s: &mut Session, i: usize) {
    run(s, "sheet.activate", json!({"sheet": i}));
}
fn uid(s: &Session) -> u64 {
    s.doc().unwrap().wb.active().unwrap().uid.get()
}
fn remember(s: &mut Session, n: u32) {
    s.doc_mut().unwrap().selection = selection(n);
}
fn three() -> (Session, [u64; 3]) {
    let mut s = session();
    let a = uid(&s);
    remember(&mut s, 10);
    run(&mut s, "home.insertSheet", json!({"name": "B"}));
    let b = uid(&s);
    remember(&mut s, 20);
    run(&mut s, "home.insertSheet", json!({"name": "C"}));
    let c = uid(&s);
    remember(&mut s, 30);
    activate(&mut s, 0);
    (s, [a, b, c])
}
#[test]
fn delete_on_either_side_keeps_the_actual_active_uid_and_live_selection() {
    for (active, deleted, expected) in [(0, 1, 0), (2, 0, 1), (1, 1, 1), (2, 2, 1), (0, 0, 0)] {
        let (mut s, ids) = three();
        activate(&mut s, active);
        let incoming = if active == deleted { if deleted == 2 { 1 } else { deleted + 1 } } else { active };
        run(&mut s, "home.deleteSheet", json!({"sheet": deleted}));
        assert_eq!(s.doc().unwrap().wb.active_sheet(), expected);
        assert_eq!(uid(&s), ids[incoming]);
        assert_eq!(s.doc().unwrap().selection, selection(10 + incoming as u32 * 10));
        run(&mut s, "edit.undo", json!({}));
        for (i, id) in ids.iter().enumerate() {
            activate(&mut s, i);
            assert_eq!(uid(&s), *id);
            assert_eq!(s.doc().unwrap().selection, selection(10 + i as u32 * 10));
        }
    }
}
#[test]
fn move_copy_insert_hide_unhide_and_name_reuse_do_not_borrow_slots() {
    let (mut s, ids) = three();
    run(&mut s, "sheet.move", json!({"sheet": 1, "to": 0}));
    assert_eq!(uid(&s), ids[1]);
    assert_eq!(s.doc().unwrap().selection, selection(20));
    run(&mut s, "sheet.move", json!({"sheet": 0, "to": 0, "copy": true}));
    let copy = uid(&s);
    assert!(!ids.contains(&copy));
    assert_eq!(s.doc().unwrap().selection, Selection::default());
    run(&mut s, "home.insertSheet", json!({"name": "Before", "before": 0}));
    assert_eq!(s.doc().unwrap().selection, Selection::default());
    activate(&mut s, 2);
    assert_eq!(uid(&s), ids[1]);
    assert_eq!(s.doc().unwrap().selection, selection(20));
    run(&mut s, "sheet.hide", json!({"sheet": 2}));
    assert_eq!(s.doc().unwrap().selection, Selection::default());
    run(&mut s, "sheet.unhide", json!({"sheet": 2}));
    assert_eq!(uid(&s), ids[1]);
    assert_eq!(s.doc().unwrap().selection, selection(20));
    run(&mut s, "sheet.hide", json!({"sheet": 4}));
    assert_eq!(uid(&s), ids[1]);
    assert_eq!(s.doc().unwrap().selection, selection(20));
    run(&mut s, "sheet.rename", json!({"sheet": 2, "name": "Renamed"}));
    run(&mut s, "home.insertSheet", json!({"name": "B"}));
    assert_ne!(uid(&s), ids[1]);
    assert_eq!(s.doc().unwrap().selection, Selection::default());
}
#[test]
fn history_retains_edit_cell_policy_with_full_multi_area_selection() {
    let mut s = session();
    run(&mut s, "selection.set", json!({"range":"B7:C8,E10:F11","active":"F11"}));
    let edited = s.doc().unwrap().selection.clone();
    run(&mut s, "cell.set", json!({"input":"42"}));
    run(&mut s, "selection.set", json!({"range":"H20:I21,K23:L24","active":"L24"}));
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(s.doc().unwrap().selection, edited);
    assert_eq!(s.doc().unwrap().wb.active().unwrap().value(edited.active), Value::Empty);
    run(&mut s, "edit.redo", json!({}));
    assert_eq!(s.doc().unwrap().selection, edited);
    assert_eq!(s.doc().unwrap().wb.active().unwrap().value(edited.active), Value::Number(42.0));
}
#[test]
fn reports_publish_the_new_sheet_and_preserve_source_selection() {
    for scenario in [false, true] {
        let mut s = session();
        run(&mut s, "range.setValues", json!({"range":"A1","values":[[1],[2],[3],[4],[5]]}));
        if scenario {
            run(&mut s, "data.scenarioAdd", json!({"name":"High","changing":"A1","values":[9]}));
        }
        remember(&mut s, 40);
        let source = uid(&s);
        let undo = s.doc().unwrap().undo.len();
        let result = if scenario {
            run(&mut s, "data.scenarioSummary", json!({}))
        } else {
            run(&mut s, "data.analysisToolPak", json!({"tool":"descriptive","range":"A1:A5","newSheet":"Statistics"}))
        };
        assert_eq!(s.doc().unwrap().wb.active_sheet(), 1);
        assert_ne!(uid(&s), source);
        assert_eq!(s.doc().unwrap().selection, Selection::default());
        assert_eq!(s.doc().unwrap().undo.len(), undo + 1);
        let sh = s.doc().unwrap().wb.active().unwrap();
        if scenario {
            assert_eq!(result["scenarios"], 1);
            assert_eq!(sh.value(CellRef::new(1, 1)), Value::text("Scenario Summary"));
        } else {
            assert_eq!(result["tool"], "descriptive");
            assert_eq!(result["sheet"], "Statistics");
            assert_eq!(result["range"], "A1:B15");
            assert_eq!(sh.value(CellRef::new(2, 1)), Value::Number(3.0));
        }
        activate(&mut s, 0);
        assert_eq!(s.doc().unwrap().selection, selection(40));
        run(&mut s, "edit.undo", json!({}));
        assert_eq!(uid(&s), source);
        assert_eq!(s.doc().unwrap().selection, selection(40));
        run(&mut s, "edit.redo", json!({}));
        activate(&mut s, 0);
        assert_eq!(s.doc().unwrap().selection, selection(40));
    }
}
#[test]
fn dropping_saved_history_and_direct_group_roots_releases_only_dead_payloads() {
    let (mut s, ids) = three();
    let saved = s.doc().unwrap().wb.clone();
    s.doc_mut().unwrap().saved = saved;
    run(&mut s, "home.deleteSheet", json!({"sheet":2}));
    s.doc_mut().unwrap().undo.clear();
    s.doc_mut().unwrap().redo.clear();
    run(&mut s, "selection.set", json!({"range":"A1"}));
    assert_eq!(s.doc().unwrap().sheet_selections.len(), 3, "saved retains C");
    s.doc_mut().unwrap().saved = s.doc().unwrap().wb.clone();
    let g = s.group_begin("root").unwrap();
    run(&mut s, "home.deleteSheet", json!({"sheet":1}));
    s.doc_mut().unwrap().undo.clear();
    run(&mut s, "selection.set", json!({"range":"A1"}));
    assert_eq!(s.doc().unwrap().sheet_selections.len(), 2, "group and saved retain B, C is dead");
    s.group_rollback(g).unwrap();
    activate(&mut s, 1);
    assert_eq!(uid(&s), ids[1]);
    assert_eq!(s.doc().unwrap().selection, selection(20));
}
