use super::*;
use gridcraft_core::{CellRef, RangeRef};
use std::sync::Arc;

fn selection(n: u32) -> Selection {
    Selection {
        active: CellRef::new(n, 2),
        anchor: CellRef::new(n, 4),
        ranges: vec![RangeRef::cell(CellRef::new(n, 2)), RangeRef::cell(CellRef::new(n, 4))],
    }
}
fn ready() -> (Session, u64, u64) {
    let mut s = Session::new();
    s.new_workbook();
    let d = s.doc_mut().unwrap();
    d.sheet_selections = vec![selection(4), selection(90), Selection::default()];
    d.selection = selection(7);
    let a = s.active_window().unwrap();
    let b = s.new_window().unwrap();
    s.doc_mut().unwrap().selection = selection(9);
    (s, a, b)
}
fn state(s: &Session) -> String {
    format!(
        "{:?}|{:?}|{}|{}|{}|{:?}",
        s.windows,
        s.active_window,
        s.active,
        s.activation,
        s.next_window,
        s.docs
            .iter()
            .map(|d| (
                d.wb.active_sheet(),
                &d.selection,
                &d.sheet_selections,
                &d.selection_memory,
                d.selection_refresh_deferred,
                d.sheet_selections.as_ptr() as usize,
                d.sheet_selections.capacity(),
                Arc::as_ptr(&d.wb) as usize
            ))
            .collect::<Vec<_>>()
    )
}

#[test]
fn reversible_window_failures_preserve_every_view_and_activation_field() {
    for action in ["activate", "show", "new", "close", "hide", "unhide", "target"] {
        let (mut s, a, b) = ready();
        if action == "unhide" {
            s.hide_window(a);
        }
        s.doc_mut().unwrap().selection_preparation_budget = Some(0);
        let before = state(&s);
        match action {
            "activate" => s.activate_window(a),
            "show" => s.show_window(a),
            "new" => assert!(s.new_window().is_err()),
            "close" => s.close_window(b),
            "hide" => s.hide_window(b),
            "unhide" => s.unhide_window(a),
            _ => {
                assert_eq!(action, "target");
                let mut called = false;
                assert!(s.with_target(Target::Window(a), |_| called = true).is_err());
                assert!(!called, "a failed entry must never run on the wrong window");
            }
        }
        assert_eq!(state(&s), before, "{action} published a failed reversible change");
    }
}

#[test]
fn mandatory_close_commands_do_not_fail_at_generic_entry_refresh() {
    for command in ["file.close", "window.close", "direct"] {
        let (mut s, a, b) = ready();
        s.activate_window(a); // MRU a; rendering b parks a, retaining one already-live survivor.
        s.show_window(b);
        let survivor = s.doc().unwrap().uid;
        s.new_workbook();
        let closed = s.doc().unwrap().uid;
        let token = s.group_begin("closed group").unwrap();
        for d in &mut s.docs {
            d.selection_preparation_budget = Some(0);
        }
        if command == "direct" {
            s.close_document(1);
        } else {
            assert_eq!(s.execute(command, json!({"force":true})).unwrap()["closed"], true);
        }
        assert_eq!(s.docs.len(), 1);
        assert_eq!(s.doc().unwrap().uid, survivor);
        assert_eq!(s.active_window(), Some(b));
        assert!(!s.windows.iter().any(|w| w.doc_uid == closed));
        assert!(!s.group_is_open(&token));
        assert!(s.windows.iter().find(|w| w.id == b).unwrap().view.is_none());
        assert!(s.doc().unwrap().selection_refresh_deferred);
    }
}

#[test]
fn public_result_window_wrappers_do_not_report_failed_noop_success() {
    for command in ["window.activate", "view.switchWindows", "view.unhideWindow", "view.hideWindow", "window.close", "file.close", "view.newWindow"] {
        let (mut s, a, b) = ready();
        s.doc_mut().unwrap().selection_preparation_budget = Some(0);
        let before = state(&s);
        let target = if matches!(command, "view.hideWindow" | "window.close" | "file.close") { b } else { a };
        assert!(s.execute(command, json!({"window":target})).is_err(), "{command}");
        assert_eq!(state(&s), before, "{command}");
    }
}

#[test]
fn close_nonactive_live_window_keeps_global_activation_and_stamps() {
    let (mut s, a, b) = ready();
    s.new_workbook();
    let other = s.active_window().unwrap();
    let stamps: Vec<_> = s.windows.iter().map(|w| (w.id, w.last_active)).collect();
    let activation = s.activation;
    s.try_close_window(b).unwrap();
    assert_eq!(s.active_window(), Some(other));
    assert_eq!(s.activation, activation);
    assert_eq!(s.document_view_window(s.window(a).unwrap().doc_uid), Some(a));
    for w in &s.windows {
        assert!(stamps.contains(&(w.id, w.last_active)));
    }
    s.show_window(a);
    assert_eq!(s.doc().unwrap().selection, selection(7));
}

#[test]
fn close_live_prepares_both_documents_before_any_publication() {
    let (mut s, a, b) = ready();
    s.new_workbook();
    let c = s.active_window().unwrap();
    let e = s.new_window().unwrap();
    s.activate_window(c);
    s.show_window(e);
    s.activate_window(b);
    s.docs[1].selection_preparation_budget = Some(0);
    let before = state(&s);
    assert!(s.try_close_window(b).is_err());
    assert_eq!(state(&s), before);
    s.docs[1].selection_preparation_budget = None;
    s.try_close_window(b).unwrap();
    assert_eq!(s.active_window(), Some(c));
    assert_eq!(s.document_view_window(s.window(a).unwrap().doc_uid), Some(a));
}

#[test]
fn pair_capture_preserves_empty_short_full_long_and_same_length_public_edits() {
    for len in [0, 1, 2, 5] {
        let mut s = Session::new();
        s.new_workbook();
        s.execute("home.insertSheet", json!({"name":"B"})).unwrap();
        let values: Vec<_> = (0..len).map(selection).collect();
        s.doc_mut().unwrap().sheet_selections = values.clone();
        let a = s.active_window().unwrap();
        let b = s.new_window().unwrap();
        assert_eq!(s.doc().unwrap().sheet_selections, values);
        let mut edited = values.clone();
        edited.reverse();
        if let Some(v) = edited.first_mut() {
            *v = selection(71);
        }
        s.doc_mut().unwrap().sheet_selections = edited.clone();
        s.show_window(a);
        assert_eq!(s.doc().unwrap().sheet_selections, values);
        s.show_window(b);
        assert_eq!(s.doc().unwrap().sheet_selections, edited);
        s.doc_mut().unwrap().sheet_selections.push(selection(99));
        let replacement = s.doc().unwrap().sheet_selections.clone();
        s.show_window(a);
        s.show_window(b);
        assert_eq!(s.doc().unwrap().sheet_selections, replacement);
    }
}

#[test]
fn deferred_snapshot_uses_actual_active_uid_and_duplicate_fallback() {
    let (mut s, _, _) = ready();
    s.execute("home.insertSheet", json!({"name":"B"})).unwrap();
    let d = s.doc_mut().unwrap();
    let old = d.selection_memory.active_uid();
    d.wb.set_active_sheet(0);
    d.selection = selection(42);
    d.selection_refresh_deferred = true;
    let pair = PairedView::snapshot(d);
    assert_ne!(pair.captured, old);
    assert_eq!(pair.captured, Some(d.wb.sheets[0].uid.get()));
    assert!(pair.deferred);
    assert_eq!(pair.memory, d.selection_memory);
    let wb = Arc::make_mut(&mut d.wb);
    wb.sheets[1] = wb.sheets[0].clone();
    assert_eq!(PairedView::snapshot(d).captured, None);
    let before = d.sheet_selections.clone();
    let pair = PairedView::snapshot(d);
    let plan = pair.prepare(d).unwrap();
    pair.apply(d, plan);
    assert_eq!(d.selection, selection(42));
    assert_eq!(d.sheet_selections, before);
    Arc::make_mut(&mut d.wb).sheets.clear();
    let pair = PairedView::snapshot(d);
    let plan = pair.prepare(d).unwrap();
    pair.apply(d, plan);
    assert_eq!(d.selection, Selection::default());
}

#[test]
fn close_releases_debugger_and_groups_even_when_survivor_refresh_fails() {
    let (mut s, _, _) = ready();
    s.new_workbook();
    let mac = crate::macros::parse_macro(
        &json!({"name":"Close", "output":{"onConflict":"overwrite"}, "steps":[{"command":"cell.set", "params":{"cell":"A1", "input":"x"}}]}),
    )
    .unwrap();
    Arc::make_mut(&mut s.doc_mut().unwrap().wb).macros.push(mac);
    s.execute("macro.debug", json!({"name":"Close"})).unwrap();
    let closed = s.doc().unwrap().uid;
    for d in &mut s.docs {
        d.selection_preparation_budget = Some(0);
    }
    assert_eq!(s.execute("file.close", json!({"force":true})).unwrap()["closed"], true);
    assert!(s.macro_debug.is_none());
    assert!(!s.groups.iter().any(|g| g.doc == closed));
    assert_eq!(s.docs.len(), 1);
    assert_eq!(s.window(s.active_window().unwrap()).unwrap().doc_uid, s.doc().unwrap().uid);
}

#[test]
fn every_registered_lifecycle_action_has_a_staged_wrapper_control() {
    let actual: Vec<_> = crate::cmd::command_specs()
        .iter()
        .filter(|c| (is_window_command(c.id) && c.id != "window.list") || c.id == "file.close")
        .map(|c| c.id)
        .collect();
    let mut expected =
        vec!["window.activate", "window.close", "view.newWindow", "view.hideWindow", "view.unhideWindow", "view.switchWindows", "file.close"];
    let mut actual = actual;
    actual.sort_unstable();
    expected.sort_unstable();
    assert_eq!(actual, expected);
}

#[test]
fn vanished_capture_with_invalid_actual_active_falls_back_to_first_sheet() {
    let (mut s, _, _) = ready();
    s.execute("home.insertSheet", json!({"name":"B"})).unwrap();
    let d = s.doc_mut().unwrap();
    d.sheet_selections = vec![selection(3), selection(4)];
    let pair = PairedView::capture(d).unwrap();
    let wb = Arc::make_mut(&mut d.wb);
    wb.sheets[1] = Arc::new(gridcraft_model::Sheet::new("B"));
    wb.set_active_sheet(usize::MAX);
    let prepared = pair.prepare(d).unwrap();
    pair.apply(d, prepared);
    assert_eq!(d.wb.active_sheet(), 0);
    assert_eq!(d.selection, selection(3));
}

#[test]
fn new_window_prepares_live_prefix_before_publication_at_every_owned_budget() {
    let mut succeeded = false;
    for budget in 0..32 {
        let (mut s, _, _) = ready();
        Arc::make_mut(&mut s.doc_mut().unwrap().wb).sheets.push(Arc::new(gridcraft_model::Sheet::new("B")));
        s.doc_mut().unwrap().selection_preparation_budget = Some(budget);
        let before = state(&s);
        match s.new_window() {
            Err(_) => assert_eq!(state(&s), before, "budget {budget}"),
            Ok(_) => {
                succeeded = true;
                assert_eq!(
                    s.doc().unwrap().sheet_selections,
                    vec![selection(4), Selection::default(), selection(90), Selection::default()],
                    "budget {budget}"
                );
            }
        }
    }
    assert!(succeeded);
}

#[test]
fn public_document_activation_failure_and_unknown_index_remain_noops() {
    let (mut s, _, _) = ready();
    s.new_workbook();
    s.docs[0].selection_preparation_budget = Some(0);
    let before = state(&s);
    s.set_active(0);
    assert_eq!(state(&s), before);
    s.set_active(usize::MAX);
    assert_eq!(state(&s), before);
    s.set_active(1);
    assert_eq!(state(&s), before);
}
