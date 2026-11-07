use gridcraft_core::CellRef;
use gridcraft_model::{Comment, Workbook};
use std::sync::Arc;

fn notes(text: String) -> Arc<std::collections::BTreeMap<CellRef, Comment>> {
    Arc::new(
        [(
            CellRef::new(0, 0),
            Comment { author: String::new(), text, replies: vec![], threaded: false, resolved: false, visible: false, dates: vec![], note_box: None },
        )]
        .into(),
    )
}

#[test]
fn large_note_edits_trim_history() {
    let mut s = super::s();
    let mut current = s.doc().unwrap().wb.clone();
    let mut history = Vec::new();
    for version in 0..8 {
        let previous = current.clone();
        let mut next = (*current).clone();
        next.sheet_mut(0).unwrap().comments = notes(format!("{version}{}", "n".repeat(32 * 1024)));
        current = Arc::new(next);
        crate::push_history_with_budget(&mut history, version.to_string(), previous, &current, crate::Selection::default(), 48 * 1024);
    }
    assert_eq!(history.len(), 1, "a second old note exceeds the injected byte budget");
    assert!(history[0].retained_bytes >= 32 * 1024);
    let d = s.doc_mut().unwrap();
    d.wb = current;
    d.undo = history;
    s.execute("edit.undo", serde_json::json!({})).unwrap();
    assert!(s.doc().unwrap().wb.active().unwrap().comments[&CellRef::new(0, 0)].text.starts_with('6'));
    s.execute("edit.redo", serde_json::json!({})).unwrap();
    assert!(s.doc().unwrap().wb.active().unwrap().comments[&CellRef::new(0, 0)].text.starts_with('7'));
}

#[test]
fn nonadjacent_shared_payload_is_charged_once_across_sheets() {
    let payload = notes("n".repeat(128 * 1024));
    let mut history = Vec::new();
    let mut current = Arc::new(Workbook::new());
    for version in 0..6 {
        let previous = current.clone();
        let mut next = Workbook::new();
        if version % 2 == 0 {
            next.sheet_mut(0).unwrap().comments = payload.clone();
            let mut second = gridcraft_model::Sheet::new("Other");
            second.comments = payload.clone();
            next.sheets.push(Arc::new(second));
        }
        current = Arc::new(next);
        crate::push_history_with_budget(&mut history, version.to_string(), previous, &current, crate::Selection::default(), 512 * 1024);
    }
    let bytes: usize = history.iter().map(|h| h.retained_bytes).sum();
    assert!(bytes >= 128 * 1024, "history-only shared map must be charged: {bytes}");
    assert!(bytes < 256 * 1024, "same allocation in multiple sheets/nonadjacent steps counted repeatedly: {bytes}");
    assert_eq!(history.len(), 6);
}

#[test]
fn live_shared_payload_larger_than_budget_keeps_small_edits() {
    let mut current = Workbook::new();
    current.sheet_mut(0).unwrap().comments = notes("n".repeat(256 * 1024));
    let mut current = Arc::new(current);
    let mut history = Vec::new();
    for version in 0..8 {
        let previous = current.clone();
        let mut next = (*current).clone();
        next.sheet_mut(0).unwrap().set_value(CellRef::new(0, 1), gridcraft_core::Value::Number(version as f64));
        current = Arc::new(next);
        crate::push_history_with_budget(&mut history, version.to_string(), previous, &current, crate::Selection::default(), 96 * 1024);
    }
    assert_eq!(history.len(), 8);
    assert!(history.iter().map(|h| h.retained_bytes).sum::<usize>() < 96 * 1024);
}

#[test]
fn an_oversized_latest_note_step_remains_undoable() {
    let mut old = Workbook::new();
    old.sheet_mut(0).unwrap().comments = notes("n".repeat(128 * 1024));
    let current = Workbook::new();
    let mut history = Vec::new();
    crate::push_history_with_budget(&mut history, "remove note".into(), Arc::new(old), &current, crate::Selection::default(), 1024);
    assert_eq!(history.len(), 1);
    assert!(history[0].retained_bytes > 128 * 1024);
}

#[test]
fn saturated_estimates_still_trim_to_latest_step() {
    let mut history = Vec::new();
    let mut current = Arc::new(Workbook::new());
    for version in 0..4 {
        let previous = current;
        let mut next = Workbook::new();
        let mut value = serde_json::Value::Null;
        for _ in 0..300 {
            value = serde_json::Value::Array(vec![value]);
        }
        next.macros_read.push(value);
        current = Arc::new(next);
        // Delay trimming until multiple saturating entries are present.
        crate::push_entry(&mut history, version.to_string(), previous, &current, crate::Selection::default());
    }
    crate::trim_to_budget(&mut history, &current, 1024);
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].label, "3");
}
