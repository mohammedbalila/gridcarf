use gridcraft_core::{CellRef, RangeRef};
use gridcraft_engine::{Selection, Session, Target};
use serde_json::json;
use std::sync::Arc;

fn selection(cell: &str) -> Selection {
    let at = CellRef::parse(cell).unwrap();
    Selection { active: at, anchor: CellRef::new(20, 8), ranges: vec![RangeRef::cell(at), RangeRef::parse("I21:K24").unwrap()] }
}
fn setup() -> (Session, u64, u64) {
    let mut s = Session::new();
    s.new_workbook();
    s.execute("home.insertSheet", json!({"name":"B"})).unwrap();
    s.doc_mut().unwrap().sheet_selections = vec![selection("A3"), selection("D4"), selection("Z99"), Selection::default()];
    s.doc_mut().unwrap().selection = selection("B7");
    let first = s.active_window().unwrap();
    let second = s.new_window().unwrap();
    s.doc_mut().unwrap().selection = selection("C9");
    (s, first, second)
}
fn sheet(s: &mut Session, index: usize) {
    s.execute("sheet.activate", json!({"sheet":index})).unwrap();
}

#[test]
fn vanished_captured_b_keeps_b7_for_undo_without_contaminating_fallback_a() {
    let (mut s, first, second) = setup();
    let b = s.doc().unwrap().wb.sheets[1].uid.get();
    s.execute("home.deleteSheet", json!({"sheet":1})).unwrap();
    s.activate_window(first);
    assert_eq!(s.doc().unwrap().selection, selection("A3"), "fallback A uses its own remembered selection");
    s.activate_window(second);
    s.execute("edit.undo", json!({})).unwrap();
    sheet(&mut s, 1);
    assert_eq!(s.doc().unwrap().selection, selection("C9"));
    s.activate_window(first);
    sheet(&mut s, 1);
    assert_eq!(s.doc().unwrap().wb.sheets[1].uid.get(), b);
    assert_eq!(s.doc().unwrap().selection, selection("B7"), "parked live B7 belongs only to captured B");
    assert_eq!(&s.doc().unwrap().sheet_selections[s.doc().unwrap().sheet_selections.len() - 2..], &[selection("Z99"), Selection::default()]);
}

#[test]
fn vanished_b_without_roots_drops_only_known_payload_at_window_boundary() {
    let (mut s, first, _) = setup();
    s.execute("home.deleteSheet", json!({"sheet":1})).unwrap();
    let d = s.doc_mut().unwrap();
    d.undo.clear();
    d.redo.clear();
    d.saved = d.wb.clone();
    s.show_window(first);
    assert_eq!(s.doc().unwrap().selection, selection("A3"));
    assert_eq!(s.doc().unwrap().sheet_selections, vec![selection("A3"), selection("Z99"), Selection::default()]);
}

#[test]
fn renamed_reordered_uid_keeps_live_selection_and_inactive_prefix() {
    let (mut s, first, second) = setup();
    let b = s.doc().unwrap().wb.sheets[1].uid.get();
    {
        let wb = Arc::make_mut(&mut s.doc_mut().unwrap().wb);
        Arc::make_mut(&mut wb.sheets[1]).name = "Renamed".into();
        wb.sheets.swap(0, 1);
        wb.set_active_sheet(0);
    }
    s.show_window(first);
    assert_eq!(s.doc().unwrap().wb.active().unwrap().uid.get(), b);
    assert_eq!(s.doc().unwrap().selection, selection("B7"));
    assert_eq!(s.doc().unwrap().sheet_selections[1], selection("A3"), "RefEdit reads inactive current prefix");
    s.with_target(Target::Window(second), |s| assert_eq!(s.doc().unwrap().selection, selection("C9"))).unwrap();
    assert_eq!(s.active_window(), Some(first));
    assert_eq!(s.doc().unwrap().selection, selection("B7"));
}

#[test]
fn replacement_same_name_index_never_inherits_vanished_live() {
    let (mut s, first, _) = setup();
    let old = s.doc().unwrap().wb.sheets[1].uid.get();
    let wb = Arc::make_mut(&mut s.doc_mut().unwrap().wb);
    wb.sheets[1] = Arc::new(gridcraft_model::Sheet::new("B"));
    assert_ne!(wb.sheets[1].uid.get(), old);
    s.activate_window(first);
    assert_eq!(s.doc().unwrap().selection, Selection::default());
}

#[test]
fn paired_view_lifecycle_preserves_full_selection_and_workbook_arc() {
    let (mut s, first, second) = setup();
    let wb = s.doc().unwrap().wb.clone();
    let dirty = s.doc().unwrap().is_dirty();
    s.hide_window(second);
    assert_eq!(s.doc().unwrap().selection, selection("B7"));
    s.unhide_window(second);
    assert_eq!(s.doc().unwrap().selection, selection("C9"));
    s.close_window(second);
    assert_eq!(s.active_window(), Some(first));
    assert_eq!(s.doc().unwrap().selection, selection("B7"));
    assert!(Arc::ptr_eq(&wb, &s.doc().unwrap().wb));
    assert_eq!(s.doc().unwrap().is_dirty(), dirty);
}

#[test]
fn parked_b_keeps_captured_live_when_other_window_undoes_before_activation() {
    let (mut s, first, _) = setup();
    let b = s.doc().unwrap().wb.sheets[1].uid.get();
    s.execute("home.deleteSheet", json!({"sheet":1})).unwrap();
    s.execute("edit.undo", json!({})).unwrap();
    s.show_window(first);
    assert_eq!(s.doc().unwrap().wb.active().unwrap().uid.get(), b);
    assert_eq!(s.doc().unwrap().selection, selection("B7"));
}

#[test]
fn public_view_history_and_ctx_literals_remain_usable() {
    let mut s = Session::new();
    s.new_workbook();
    let view = gridcraft_engine::WindowView {
        selection: selection("D12"),
        sheet: 0,
        sheet_name: "Sheet1".into(),
        sheet_selections: vec![selection("A3"), Selection::default()],
    };
    let history = gridcraft_engine::HistoryEntry {
        label: "public history".into(),
        wb: s.doc().unwrap().wb.clone(),
        selection: view.selection.clone(),
        retained_bytes: 0,
    };
    let mut chosen = Selection::default();
    let cx = gridcraft_engine::cmd::Ctx {
        wb: gridcraft_model::Workbook::new(),
        changed: vec![],
        structural: false,
        sel: &mut chosen,
        fit_rows: vec![],
        protection_checked: false,
        edits: vec![],
    };
    *cx.sel = view.selection;
    assert_eq!(chosen, selection("D12"));
    s.doc_mut().unwrap().undo.push(history);
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(s.doc().unwrap().selection, chosen);
    s.doc_mut().unwrap().sheet_selections = view.sheet_selections;
    s.new_window().unwrap();
    assert_eq!(s.doc().unwrap().sheet_selections.last(), Some(&Selection::default()));
}

#[test]
fn e2_final_group_view_only_rollback_restores_captured_uid() {
    for failure in [false, true] {
        let mut s = Session::new();
        s.new_workbook();
        s.execute("home.insertSheet", json!({"name":"B"})).unwrap();
        let d = s.doc_mut().unwrap();
        d.selection = selection("B7");
        d.sheet_selections = vec![selection("A3"), selection("D4"), selection("Z99"), Selection::default()];
        let (a, b) = (d.wb.sheets[0].uid.get(), d.wb.sheets[1].uid.get());
        let wb = d.wb.clone();
        let history = format!("{:?}", (&d.undo, &d.redo));
        if failure {
            let error = s
                .grouped("view failure", |s| {
                    s.execute("sheet.activate", json!({"sheet":0}))?;
                    s.execute("cell.set", json!({"cell":"?", "input":"bad"}))
                })
                .unwrap_err();
            assert!(matches!(error, gridcraft_engine::EngineError::BadParams { .. }));
        } else {
            let token = s.group_begin("view only").unwrap();
            sheet(&mut s, 0);
            assert_eq!(s.doc().unwrap().wb.active().unwrap().uid.get(), a);
            assert_eq!(s.doc().unwrap().selection, selection("A3"));
            assert!(Arc::ptr_eq(&wb, &s.doc().unwrap().wb));
            s.group_rollback(token).unwrap();
        }
        assert_eq!(s.doc().unwrap().wb.active().unwrap().uid.get(), b, "captured B selection belongs to B");
        assert_eq!(s.doc().unwrap().selection, selection("B7"));
        assert!(Arc::ptr_eq(&wb, &s.doc().unwrap().wb));
        assert_eq!(format!("{:?}", (&s.doc().unwrap().undo, &s.doc().unwrap().redo)), history);
        sheet(&mut s, 0);
        assert_eq!(s.doc().unwrap().selection, selection("A3"));
        sheet(&mut s, 1);
        assert_eq!(s.doc().unwrap().wb.active().unwrap().uid.get(), b);
        assert_eq!(s.doc().unwrap().selection, selection("B7"));
        assert_eq!(&s.doc().unwrap().sheet_selections[2..], &[selection("Z99"), Selection::default()]);
    }
}
