//! Runtime identity owns remembered selections, including sheets restored by history.

use gridcraft_core::{CellRef, RangeRef};
use gridcraft_engine::{Selection, Session};
use serde_json::{Value, json};

fn session() -> Session {
    let mut s = Session::new();
    s.new_workbook();
    s
}

fn command(s: &mut Session, id: &str, params: Value) {
    s.execute(id, params).unwrap_or_else(|error| panic!("{id}: {error}"));
    assert_eq!(s.depth(), 0);
}

fn selected(range: &str, active: &str) -> Selection {
    let ranges: Vec<_> = range.split(',').map(|r| RangeRef::parse(r).unwrap()).collect();
    Selection { active: CellRef::parse(active).unwrap(), anchor: ranges.last().unwrap().start, ranges }
}

fn select(s: &mut Session, range: &str, active: &str) {
    command(s, "selection.set", json!({"range": range, "active": active}));
    assert_eq!(s.doc().unwrap().selection, selected(range, active));
}

fn activate(s: &mut Session, index: usize) {
    command(s, "sheet.activate", json!({"sheet": index}));
    assert_eq!(s.doc().unwrap().wb.active_sheet(), index);
}

fn roots(s: &Session) {
    let mut maximum = 0;
    s.visit_document_workbooks(s.doc().unwrap().uid, |wb| maximum = maximum.max(wb.sheets.len()));
    assert_eq!(maximum, 2, "borrowed restoration roots stay finite");
}

fn cycle(s: &mut Session, range: &str, active: &str) -> u64 {
    let before = s.doc().unwrap().selection.clone();
    command(s, "home.insertSheet", json!({"name": "Transient"}));
    let d = s.doc().unwrap();
    assert_eq!((d.wb.sheets.len(), d.wb.active_sheet()), (2, 1));
    let uid = d.wb.sheets[1].uid.get();
    select(s, range, active);
    activate(s, 0);
    assert_eq!(s.doc().unwrap().sheet_selections[1], selected(range, active));
    command(s, "edit.undo", json!({}));
    let d = s.doc().unwrap();
    assert_eq!((d.wb.sheets.len(), d.undo.len(), d.redo.len(), d.saved.sheets.len()), (1, 0, 1, 1));
    assert_eq!(d.redo[0].wb.sheets.len(), 2);
    assert_eq!(d.redo[0].wb.sheets[1].uid.get(), uid);
    assert_eq!(d.selection, before);
    roots(s);
    uid
}

#[test]
fn uid_restore_keeps_latest_sheet_selection() {
    let mut s = session();
    let a = cycle(&mut s, "B7", "B7");
    let b = cycle(&mut s, "C9", "C9");
    assert_ne!(a, b);
    command(&mut s, "edit.redo", json!({}));
    command(&mut s, "home.deleteSheet", json!({"sheet": 0}));
    command(&mut s, "edit.undo", json!({}));
    activate(&mut s, 1);
    let d = s.doc().unwrap();
    assert_eq!(d.wb.active().unwrap().uid.get(), b);
    assert_eq!(d.selection, Selection::at(CellRef::parse("C9").unwrap()), "restored B owns C9, never A's B7");
}

#[test]
fn captured_redo_keeps_its_sheet_selection() {
    let mut s = session();
    let a = cycle(&mut s, "B7", "B7");
    let group = s.group_begin("selection rollback").unwrap();
    command(&mut s, "home.insertSheet", json!({"name": "Grouped"}));
    let grouped = s.doc().unwrap().wb.active().unwrap().uid.get();
    assert_ne!(a, grouped);
    select(&mut s, "D12:E13,G15:H16", "H16");
    activate(&mut s, 0);
    let mut captured_a = false;
    s.visit_document_workbooks(s.doc().unwrap().uid, |wb| captured_a |= wb.sheets.iter().any(|sh| sh.uid.get() == a));
    assert!(captured_a, "the direct group retains the earlier redo identity");
    s.group_rollback(group).unwrap();
    let d = s.doc().unwrap();
    assert_eq!((d.wb.sheets.len(), d.undo.len(), d.redo.len()), (1, 0, 1));
    assert_eq!(d.redo[0].wb.sheets[1].uid.get(), a);
    command(&mut s, "edit.redo", json!({}));
    activate(&mut s, 1);
    let d = s.doc().unwrap();
    assert_eq!(d.wb.active().unwrap().uid.get(), a);
    assert_eq!(d.selection, Selection::at(CellRef::parse("B7").unwrap()), "captured redo owns A's B7");
}

fn sample(s: &Session, completed: usize) {
    let values = &s.doc().unwrap().sheet_selections;
    let ranges_len: usize = values.iter().map(|v| v.ranges.len()).sum();
    let ranges_cap: usize = values.iter().map(|v| v.ranges.capacity()).sum();
    println!(
        "selection_memory cycles={completed} outer_len={} outer_cap={} ranges_len={ranges_len} ranges_cap={ranges_cap} outer_bytes={} nested_bytes={} (containers, not RSS)",
        values.len(),
        values.capacity(),
        values.capacity() * std::mem::size_of::<Selection>(),
        ranges_cap * std::mem::size_of::<RangeRef>()
    );
}

#[test]
fn repeated_sheet_cycles_keep_fixed_legacy_bound() {
    let mut s = session();
    let legacy = [selected("J20:K21,M23:N24", "N24"), Selection::default(), selected("P40:Q41,S43:T44", "T44")];
    s.doc_mut().unwrap().sheet_selections = std::iter::once(Selection::default()).chain(legacy.iter().cloned()).collect();
    let mut previous = None;
    for n in 1..=1016 {
        let uid = cycle(&mut s, &format!("B{n}:C{},E{n}:F{}", n + 1, n + 1), &format!("F{}", n + 1));
        assert_ne!(previous, Some(uid), "fresh insert must have a fresh runtime identity");
        previous = Some(uid);
        let values = &s.doc().unwrap().sheet_selections;
        assert_eq!(&values[values.len() - legacy.len()..], &legacy, "anonymous payload and order are exact at cycle {n}");
        assert!(values.len() <= legacy.len() + 2, "cycle {n}: {} slots exceed fixed legacy L + two restorable identities", values.len());
        if n == 16 || n == 1016 {
            sample(&s, n);
        }
    }
    println!("selection_memory completed=1016 successful_commands=4064");
}
