use super::*;
use crate::{Session, cmd};
use serde_json::json;

fn session() -> Session {
    let mut s = Session::new();
    s.new_workbook();
    s
}
fn selected(n: u32) -> Selection {
    Selection {
        active: CellRef::new(n + 1, 5),
        anchor: CellRef::new(n, 1),
        ranges: vec![RangeRef::cell(CellRef::new(n, 1)), RangeRef::cell(CellRef::new(n + 1, 5))],
    }
}
fn nested(s: &mut Session, _: &serde_json::Value) -> crate::Result<serde_json::Value> {
    let _group = s.group_begin("nested")?;
    s.execute("home.insertSheet", json!({"name":"Temporary"}))?;
    s.doc_mut()?.selection = selected(30);
    s.execute("sheet.activate", json!({"sheet":0}))?;
    Err(crate::EngineError::Other("original error".into()))
}
fn escaped(s: &mut Session, p: &serde_json::Value) -> crate::Result<serde_json::Value> {
    let _ = nested(s, p);
    panic!("original panic")
}
#[test]
fn normal_error_and_panic_reconcile_labels_without_losing_anonymous_payload() {
    cmd::register_undoable_test_command("test.selectionMemory.error", nested);
    cmd::register_undoable_test_command("test.selectionMemory.panic", escaped);
    for id in ["test.selectionMemory.error", "test.selectionMemory.panic"] {
        let mut s = session();
        s.doc_mut().unwrap().sheet_selections = vec![selected(1), selected(90)];
        s.doc_mut().unwrap().selection = selected(10);
        let original = s.doc().unwrap().wb.sheets[0].uid.get();
        let error = s.execute(id, json!({})).unwrap_err();
        assert!(error.to_string().contains(if id.ends_with("panic") { "original panic" } else { "original error" }));
        let d = s.doc().unwrap();
        assert_eq!(d.wb.sheets.len(), 1);
        assert_eq!(d.wb.sheets[0].uid.get(), original);
        assert_eq!(d.selection, selected(10));
        assert_eq!(d.sheet_selections.last(), Some(&selected(90)));
        assert!(d.sheet_selections.len() <= 2);
        assert!(d.undo.is_empty() && d.redo.is_empty());
        assert!(s.groups.is_empty());
    }
}

// The failure budget belongs to this DocState. Each plan starts with that allowance;
// the real Preparation::vector decrements it only for non-empty reservations.
fn fingerprint(d: &crate::DocState) -> serde_json::Value {
    json!({"values":d.sheet_selections,"labels":d.selection_memory.labels,
        "current":d.selection_memory.current,"published":d.selection_memory.published_len,
        "active":d.selection_memory.active,"outer":[d.sheet_selections.as_ptr() as usize,d.sheet_selections.capacity()],
        "labelsBuffer":[d.selection_memory.labels.as_ptr() as usize,d.selection_memory.labels.capacity()],
        "currentBuffer":[d.selection_memory.current.as_ptr() as usize,d.selection_memory.current.capacity()],
        "nested":d.sheet_selections.iter().map(|v| (v.ranges.as_ptr() as usize,v.ranges.capacity())).collect::<Vec<_>>()})
}
fn ready() -> Session {
    let mut s = session();
    s.doc_mut().unwrap().sheet_selections = vec![selected(1), selected(90)];
    s.doc_mut().unwrap().selection = selected(10);
    s.doc_mut().unwrap().refresh_selection_memory().unwrap();
    s
}
fn fail_at_restoration(s: &mut Session, p: &serde_json::Value) -> crate::Result<serde_json::Value> {
    let _ = nested(s, p);
    let snapshot = fingerprint(s.doc()?);
    s.doc_mut()?.selection_preparation_budget = Some(0);
    let message = format!("original failure snapshot={snapshot}");
    if p["panic"].as_bool().unwrap_or(false) {
        panic!("{message}")
    }
    Err(crate::EngineError::Other(message))
}
fn assert_restored(s: &Session, original: u64) {
    let d = s.doc().unwrap();
    assert_eq!(d.wb.sheets.len(), 1);
    assert_eq!(d.wb.sheets[0].uid.get(), original);
    assert_eq!(d.selection, selected(10));
    assert_eq!(d.wb.active().unwrap().value(CellRef::default()), gridcraft_core::Value::Empty);
    assert!(d.undo.is_empty() && d.redo.is_empty());
    assert!(s.groups.is_empty());
    assert!(s.group_step.is_none());
}
#[test]
fn command_error_and_escaped_panic_restore_data_when_new_view_preparation_fails() {
    cmd::register_undoable_test_command("test.selectionMemory.restoreFailure", fail_at_restoration);
    for panic in [false, true] {
        let mut s = ready();
        s.start_recording();
        let original = s.doc().unwrap().wb.sheets[0].uid.get();
        let error = s.execute("test.selectionMemory.restoreFailure", json!({"panic":panic})).unwrap_err();
        assert!(matches!((&error, panic), (crate::EngineError::Internal(..), true) | (crate::EngineError::Other(_), false)));
        let text = error.to_string();
        let expected: serde_json::Value = serde_json::from_str(text.split_once("snapshot=").unwrap().1).unwrap();
        assert_restored(&s, original);
        assert!(s.doc().unwrap().selection_refresh_deferred);
        assert_eq!(fingerprint(s.doc().unwrap()), expected, "failed refresh leaves values and paired metadata buffers untouched");
        assert!(s.stop_recording().unwrap().steps.is_empty());
        let before = fingerprint(s.doc().unwrap());
        assert!(s.execute("home.insertSheet", json!({"name":"Refused"})).is_err());
        assert_eq!(fingerprint(s.doc().unwrap()), before);
        assert_restored(&s, original);
        s.doc_mut().unwrap().selection_preparation_budget = None;
        s.execute("selection.set", json!({"range":"B11,F12","active":"F12"})).unwrap();
        let d = s.doc().unwrap();
        assert!(!d.selection_refresh_deferred);
        assert_eq!(d.selection_memory.labels, [original]);
        assert_eq!(d.sheet_selections, [selected(10), selected(90)]);
    }
}
#[test]
fn direct_group_rollback_defers_only_view_refresh_then_recovers_exact_payload() {
    for failed in [false, true] {
        let mut s = ready();
        let original = s.doc().unwrap().wb.sheets[0].uid.get();
        s.start_recording();
        let group = s.group_begin("direct").unwrap();
        s.execute("home.insertSheet", json!({"name":"Temporary"})).unwrap();
        s.doc_mut().unwrap().selection = selected(30);
        s.execute("cell.set", json!({"input":"73"})).unwrap();
        s.execute("sheet.activate", json!({"sheet":0})).unwrap();
        let before = fingerprint(s.doc().unwrap());
        if failed {
            s.doc_mut().unwrap().selection_preparation_budget = Some(0);
        }
        s.group_rollback(group).unwrap();
        assert_restored(&s, original);
        assert!(s.stop_recording().unwrap().steps.is_empty());
        assert_eq!(s.doc().unwrap().selection_refresh_deferred, failed);
        if failed {
            assert_eq!(fingerprint(s.doc().unwrap()), before);
        }
        s.doc_mut().unwrap().selection_preparation_budget = None;
        s.execute("sheet.activate", json!({"sheet":0})).unwrap();
        let d = s.doc().unwrap();
        assert_eq!(d.sheet_selections, [selected(10), selected(90)]);
        assert_eq!(d.selection_memory.labels, [original]);
        assert_eq!(d.selection, selected(10));
    }
}
#[test]
fn switch_commit_and_history_refuse_before_publication_or_pop() {
    let mut s = ready();
    s.execute("home.insertSheet", json!({"name":"B"})).unwrap();
    let d = s.doc_mut().unwrap();
    d.selection = selected(30);
    let before = fingerprint(d);
    let wb = d.wb.clone();
    let live = d.selection.clone();
    d.selection_preparation_budget = Some(0);
    assert!(d.try_wb_switch_sheet(0).is_err());
    assert_eq!(fingerprint(d), before);
    assert_eq!(d.selection, live);
    assert_eq!(d.wb.active_sheet(), 1);
    d.wb_switch_sheet(0);
    assert_eq!(fingerprint(d), before);
    assert_eq!(d.wb.active_sheet(), 1);
    d.touched = Some(vec![]);
    d.edits = Some(vec![]);
    let calc_before = format!("{:?}", d.calc);
    let result = cmd::commit(d, |cx| {
        cx.wb.sheets.push(std::sync::Arc::new(gridcraft_model::Sheet::new("Refused")));
        cx.structural = true;
        cx.touch(0, CellRef::default());
        Ok(())
    });
    assert!(result.is_err());
    assert!(std::sync::Arc::ptr_eq(&d.wb, &wb));
    assert_eq!(format!("{:?}", d.calc), calc_before);
    assert_eq!(fingerprint(d), before);
    assert_eq!(d.selection, live);
    assert!(d.touched.as_ref().unwrap().is_empty() && d.edits.as_ref().unwrap().is_empty());
    // Nested entry bypasses outer Refresh, so this exercises history preparation itself.
    let (undo, redo) = (d.undo.len(), d.redo.len());
    assert!(s.execute_nested("edit.undo", json!({})).is_err());
    assert_eq!((s.doc().unwrap().undo.len(), s.doc().unwrap().redo.len()), (undo, redo));
    assert_eq!(fingerprint(s.doc().unwrap()), before);
    s.doc_mut().unwrap().selection_preparation_budget = None;
    s.execute("edit.undo", json!({})).unwrap();
    let d = s.doc_mut().unwrap();
    let before = fingerprint(d);
    let (undo, redo) = (d.undo.len(), d.redo.len());
    d.selection_preparation_budget = Some(0);
    assert!(s.execute_nested("edit.redo", json!({})).is_err());
    assert_eq!((s.doc().unwrap().undo.len(), s.doc().unwrap().redo.len()), (undo, redo));
    assert_eq!(fingerprint(s.doc().unwrap()), before);
}
#[test]
fn report_producers_refuse_before_new_sheet_or_logs_are_published() {
    for scenario in [false, true] {
        let mut s = ready();
        s.execute("range.setValues", json!({"range":"A1","values":[[1],[2],[3]]})).unwrap();
        if scenario {
            s.execute("data.scenarioAdd", json!({"name":"High","changing":"A1","values":[9]})).unwrap();
        }
        let d = s.doc_mut().unwrap();
        let wb = d.wb.clone();
        let before = fingerprint(d);
        let live = d.selection.clone();
        let (undo, redo) = (d.undo.len(), d.redo.len());
        d.touched = Some(vec![]);
        d.edits = Some(vec![]);
        d.selection_preparation_budget = Some(0);
        let calc_before = format!("{:?}", d.calc);
        let (id, p) =
            if scenario { ("data.scenarioSummary", json!({})) } else { ("data.analysisToolPak", json!({"tool":"descriptive","range":"A1:A3"})) };
        assert!(s.execute_nested(id, p).is_err());
        let d = s.doc().unwrap();
        assert!(std::sync::Arc::ptr_eq(&wb, &d.wb));
        assert_eq!(d.selection, live);
        assert_eq!(fingerprint(d), before);
        assert_eq!((d.undo.len(), d.redo.len()), (undo, redo));
        assert_eq!(format!("{:?}", d.calc), calc_before);
        assert!(d.touched.as_ref().unwrap().is_empty() && d.edits.as_ref().unwrap().is_empty());
    }
}
fn cleanup_failure(s: &mut Session, _: &serde_json::Value) -> crate::Result<serde_json::Value> {
    s.execute("home.insertSheet", json!({"name":"Transient"}))?;
    s.doc_mut()?.selection = selected(30);
    s.execute("sheet.activate", json!({"sheet":0}))?;
    s.execute("edit.undo", json!({}))?;
    let d = s.doc_mut()?;
    d.redo.clear();
    let expected = fingerprint(d);
    d.selection_preparation_budget = Some(0);
    Ok(expected)
}
#[test]
fn cleanup_failure_preserves_published_command_result_and_resumes_later() {
    cmd::register_test_command("test.selectionMemory.cleanupFailure", cleanup_failure);
    let mut s = ready();
    let expected = s.execute("test.selectionMemory.cleanupFailure", json!({})).unwrap();
    assert_eq!(fingerprint(s.doc().unwrap()), expected);
    assert!(s.doc().unwrap().selection_refresh_deferred);
    assert_eq!(s.doc().unwrap().sheet_selections.len(), 3, "no bound claimed while cleanup is deferred");
    s.doc_mut().unwrap().selection_preparation_budget = None;
    s.execute("sheet.activate", json!({"sheet":0})).unwrap();
    assert_eq!(s.doc().unwrap().sheet_selections, [selected(10), selected(90)]);
    assert!(!s.doc().unwrap().selection_refresh_deferred);
}

use std::sync::Arc;

#[test]
fn e2_final_group_view_only_rollback_defers_refresh_without_losing_state() {
    for write in [false, true] {
        let mut s = session();
        s.execute("home.insertSheet", json!({"name":"B"})).unwrap();
        s.execute("cell.set", json!({"cell":"A1", "input":"41"})).unwrap();
        s.execute("cell.set", json!({"cell":"A1", "input":"42"})).unwrap();
        s.execute("edit.undo", json!({})).unwrap();
        let d = s.doc_mut().unwrap();
        d.selection = selected(6);
        d.sheet_selections = vec![selected(2), selected(4), selected(90), Selection::default()];
        d.refresh_selection_memory().unwrap();
        let b = d.wb.sheets[1].uid.get();
        let wb = d.wb.clone();
        let history = format!("{:?}", (&d.undo, &d.redo));
        s.start_recording();
        let token = s.group_begin("budget rollback").unwrap();
        s.execute("sheet.activate", json!({"sheet":0})).unwrap();
        if write {
            s.execute("cell.set", json!({"cell":"C3", "input":"must roll back"})).unwrap();
        }
        let before = fingerprint(s.doc().unwrap());
        s.doc_mut().unwrap().selection_preparation_budget = Some(0);
        s.group_rollback(token).unwrap();
        let d = s.doc().unwrap();
        assert_eq!(d.wb.active().unwrap().uid.get(), b, "budget refusal must not prevent captured UID restoration");
        assert_eq!(d.selection, selected(6));
        assert!(Arc::ptr_eq(&wb, &d.wb));
        assert_eq!(d.wb.sheets[1].value(CellRef::default()), gridcraft_core::Value::Number(41.0));
        assert!(d.wb.sheets[0].cells.is_empty());
        assert_eq!(format!("{:?}", (&d.undo, &d.redo)), history);
        assert!(s.groups.is_empty());
        assert!(d.selection_refresh_deferred);
        assert_eq!(fingerprint(d), before, "failed refresh preserves all payload and metadata buffers");
        assert!(s.stop_recording().unwrap().steps.is_empty());
        s.doc_mut().unwrap().selection_preparation_budget = None;
        s.execute("sheet.activate", json!({"sheet":1})).unwrap();
        assert!(!s.doc().unwrap().selection_refresh_deferred);
        assert_eq!(s.doc().unwrap().selection_memory.active_uid(), Some(b));
        assert_eq!(s.doc().unwrap().selection, selected(6));
        s.execute("sheet.activate", json!({"sheet":0})).unwrap();
        assert_eq!(s.doc().unwrap().selection, selected(2));
        s.execute("sheet.activate", json!({"sheet":1})).unwrap();
        assert_eq!(s.doc().unwrap().selection, selected(6));
        assert_eq!(&s.doc().unwrap().sheet_selections[2..], &[selected(90), Selection::default()]);
    }
}

#[test]
fn e2_final_group_missing_or_ambiguous_identity_keeps_existing_fallback() {
    for mode in ["empty", "invalid", "duplicate"] {
        let mut s = session();
        let d = s.doc_mut().unwrap();
        let wb = Arc::make_mut(&mut d.wb);
        match mode {
            "empty" => wb.sheets.clear(),
            "invalid" => wb.set_active_sheet(99),
            _ => {
                wb.sheets.push(wb.sheets[0].clone());
                wb.set_active_sheet(1);
            }
        }
        d.selection = selected(6);
        d.sheet_selections = vec![selected(2), selected(4), selected(90)];
        let token = s.group_begin("no identity").unwrap();
        assert_eq!(s.doc().unwrap().selection_memory.active_uid(), None);
        let d = s.doc_mut().unwrap();
        d.wb.set_active_sheet(0);
        d.selection = selected(30);
        s.group_rollback(token).unwrap();
        assert_eq!(s.doc().unwrap().wb.active_sheet(), 0);
        assert_eq!(s.doc().unwrap().selection, selected(6));
        assert_eq!(s.doc().unwrap().sheet_selections, [selected(2), selected(4), selected(90)]);
        assert!(s.groups.is_empty());
    }
}
