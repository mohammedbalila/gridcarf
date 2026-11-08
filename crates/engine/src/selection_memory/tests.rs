use super::*;
use gridcraft_model::{Sheet, SheetUid};
use std::sync::Arc;

fn selection(n: u32) -> Selection {
    let anchor = CellRef::new(n, 2);
    let active = CellRef::new(n + 1, 5);
    Selection { active, anchor, ranges: vec![RangeRef::cell(anchor), RangeRef::new(anchor, active)] }
}

fn workbook(count: usize) -> Workbook {
    let mut wb = Workbook::new();
    wb.sheets = (0..count).map(|_| Arc::new(Sheet::new("Same"))).collect();
    wb
}

fn apply(owner: &mut SelectionMemory, values: &mut Vec<Selection>, before: &Workbook, after: &Workbook, live: &mut Selection, t: Transition<'_>) {
    owner.prepare(values, before, after, live, t).unwrap().apply(values, owner, live);
}

fn adopt(owner: &mut SelectionMemory, values: &mut Vec<Selection>, wb: &Workbook, live: &mut Selection) {
    apply(owner, values, wb, wb, live, Transition::Refresh);
}

#[test]
fn adoption_changes_only_integer_metadata_for_every_legacy_length() {
    let wb = workbook(3);
    for len in 0..=6 {
        let mut values: Vec<_> = (0..len).map(|n| if n % 2 == 0 { Selection::default() } else { selection(7) }).collect();
        let original = values.clone();
        let buffers: Vec<_> = values.iter().map(|v| v.ranges.as_ptr()).collect();
        let ptr = values.as_ptr();
        let mut owner = SelectionMemory::default();
        let mut live = selection(20);
        adopt(&mut owner, &mut values, &wb, &mut live);
        assert_eq!(values, original);
        assert_eq!(ptr, values.as_ptr());
        assert_eq!(buffers, values.iter().map(|v| v.ranges.as_ptr()).collect::<Vec<_>>());
        assert_eq!(owner.labels, wb.sheets.iter().take(len).map(|s| s.uid.get()).collect::<Vec<_>>());
        assert_eq!(owner.published_len, Some(len));
        assert_eq!(live, selection(20));
    }
}

#[test]
fn remap_restores_dormant_uid_and_never_promotes_anonymous_values() {
    let before = workbook(2);
    let mut owner = SelectionMemory::default();
    let mut values = vec![selection(1), selection(2), Selection::default(), selection(90)];
    let mut live = selection(10);
    adopt(&mut owner, &mut values, &before, &mut live);
    let stored_ptr = values[1].ranges.as_ptr();
    let mut after = before.clone();
    after.sheets.remove(1);
    after.sheets.insert(0, Arc::new(Sheet::new("Same")));
    after.set_active_sheet(1);
    apply(&mut owner, &mut values, &before, &after, &mut live, Transition::Refresh);
    assert_eq!(values, [Selection::default(), selection(1), selection(2), Selection::default(), selection(90)]);
    assert_eq!(values[2].ranges.as_ptr(), stored_ptr, "move nested ownership");
    apply(&mut owner, &mut values, &after, &before, &mut live, Transition::Navigate(1));
    assert_eq!(live, selection(2));
    assert_eq!(values, [selection(10), selection(2), Selection::default(), Selection::default(), selection(90)]);
    owner.prepare_prune(&values, &before, &live, &before.sheets.iter().map(|s| s.uid.get()).collect::<Vec<_>>()).unwrap().apply(
        &mut values,
        &mut owner,
        &mut live,
    );
    assert_eq!(values, [selection(10), selection(2), Selection::default(), selection(90)]);
}

#[test]
fn topology_only_refresh_preserves_labels_and_external_live_selection() {
    let before = workbook(2);
    let mut owner = SelectionMemory::default();
    let mut values = vec![selection(1), selection(2), selection(99)];
    let mut live = selection(10);
    adopt(&mut owner, &mut values, &before, &mut live);
    let mut after = before.clone();
    after.sheets.swap(0, 1);
    after.set_active_sheet(0);
    apply(&mut owner, &mut values, &after, &after, &mut live, Transition::Refresh);
    assert_eq!(values, [selection(2), selection(1), selection(99)]);
    assert_eq!(live, selection(10), "untracked activation never assigns guessed outgoing live state");
    assert_eq!(owner.active, Some(after.sheets[0].uid.get()));
}

#[test]
fn same_length_edits_replacement_permutation_and_regrow_edit_published_slots() {
    for edit in 0..4 {
        let wb = workbook(2);
        let mut owner = SelectionMemory::default();
        let mut values = vec![selection(1), selection(2), selection(99)];
        let mut live = selection(10);
        adopt(&mut owner, &mut values, &wb, &mut live);
        match edit {
            0 => values[1] = selection(8),
            1 => values = vec![selection(3), selection(8), selection(99)],
            2 => values.swap(0, 1),
            _ => {
                values.truncate(1);
                values.extend([selection(8), selection(99)]);
            }
        }
        let target = values[1].clone();
        apply(&mut owner, &mut values, &wb, &wb, &mut live, Transition::Navigate(1));
        assert_eq!(live, target);
        assert_eq!(values[0], selection(10));
        assert_eq!(values[2], selection(99));
    }
}

#[test]
fn observed_length_reset_adopts_actual_prefix_even_with_changed_topology() {
    for length in [0, 1, 2, 4] {
        let mut wb = workbook(2);
        let mut owner = SelectionMemory::default();
        let mut values = vec![selection(1), selection(2), selection(99)];
        let mut live = selection(10);
        adopt(&mut owner, &mut values, &wb, &mut live);
        values = (0..length).map(|n| selection(n + 30)).collect();
        wb.sheets.swap(0, 1);
        let reset = values.clone();
        adopt(&mut owner, &mut values, &wb, &mut live);
        assert_eq!(values, reset);
        assert_eq!(owner.labels, wb.sheets.iter().take(length as usize).map(|s| s.uid.get()).collect::<Vec<_>>());
        apply(&mut owner, &mut values, &wb, &wb, &mut live, Transition::Navigate(1));
        assert_eq!(live, reset.get(1).cloned().unwrap_or_default());
        if length > 2 {
            assert_eq!(&values[2..], &reset[2..]);
        }
    }
}

#[test]
fn numeric_identity_ignores_name_file_id_and_content_equality() {
    for file_id in [0, 7] {
        let mut wb = workbook(2);
        for sh in &mut wb.sheets {
            Arc::make_mut(sh).file_id = file_id;
        }
        assert_eq!(wb.sheets[0].uid, wb.sheets[1].uid);
        assert_ne!(wb.sheets[0].uid.get(), wb.sheets[1].uid.get());
        let mut owner = SelectionMemory::default();
        let mut values = vec![selection(1), selection(2)];
        let mut live = selection(10);
        adopt(&mut owner, &mut values, &wb, &mut live);
        apply(&mut owner, &mut values, &wb, &wb, &mut live, Transition::Navigate(1));
        assert_eq!(live, selection(2));
        let chosen = selection(50);
        apply(&mut owner, &mut values, &wb, &wb, &mut live, Transition::Publish(&chosen));
        assert_eq!(live, chosen);
    }
}

#[test]
fn duplicate_runtime_uid_keeps_positional_payload_and_later_adopts_unique_state() {
    let mut wb = workbook(2);
    Arc::make_mut(&mut wb.sheets[1]).uid = wb.sheets[0].uid;
    let mut values = vec![selection(1), selection(2), selection(99)];
    let mut owner = SelectionMemory::default();
    let mut live = selection(10);
    apply(&mut owner, &mut values, &wb, &wb, &mut live, Transition::Navigate(1));
    assert_eq!(values, [selection(10), selection(2), selection(99)]);
    assert_eq!(live, selection(2));
    assert_eq!(owner.published_len, None);
    owner.prepare_prune(&values, &wb, &live, &[]).unwrap().apply(&mut values, &mut owner, &mut live);
    assert_eq!(values.len(), 3);
    Arc::make_mut(&mut wb.sheets[1]).uid = SheetUid::fresh();
    let original = values.clone();
    adopt(&mut owner, &mut values, &wb, &mut live);
    assert_eq!(values, original);
    assert_eq!(owner.labels.len(), 2);
}

#[test]
fn every_owned_preparation_failure_preserves_payload_live_labels_and_order() {
    for case in 0..7 {
        let mut before = workbook(2);
        if case == 4 {
            Arc::make_mut(&mut before.sheets[1]).uid = before.sheets[0].uid;
        }
        let mut owner = SelectionMemory::default();
        let mut values = vec![selection(1), selection(2), selection(99)];
        if case == 4 {
            values.truncate(1);
        }
        let mut live = selection(10);
        if case != 6 {
            adopt(&mut owner, &mut values, &before, &mut live);
        }
        let mut after = before.clone();
        if case < 3 {
            after.sheets.insert(0, Arc::new(Sheet::new("new")));
        }
        if case == 3 {
            after.sheets.pop();
        }
        let original = (values.clone(), owner.clone(), live.clone());
        let pointers: Vec<_> = values.iter().map(|v| v.ranges.as_ptr()).collect();
        let outer = values.as_ptr();
        let mut failures = 0;
        let mut succeeded = false;
        let chosen = selection(40);
        let keep = [before.sheets[0].uid.get()];
        for limit in 0..100 {
            let mut budget = Preparation { remaining: Some(limit), keep: (case == 3).then_some(keep.as_slice()) };
            let transition = match case {
                0 => Transition::Publish(&chosen),
                1 => Transition::Navigate(0),
                4 => Transition::Navigate(1),
                _ => Transition::Refresh,
            };
            match owner.prepare_with(&values, &before, &after, &live, transition, &mut budget) {
                Err(_) => {
                    failures += 1;
                    assert_eq!((&values, &owner, &live), (&original.0, &original.1, &original.2));
                    assert_eq!(values.as_ptr(), outer);
                    assert_eq!(values.iter().map(|v| v.ranges.as_ptr()).collect::<Vec<_>>(), pointers);
                    assert_eq!((before.active_sheet(), after.active_sheet()), (0, 0));
                }
                Ok(prepared) => {
                    prepared.apply(&mut values, &mut owner, &mut live);
                    succeeded = true;
                    break;
                }
            }
        }
        assert!(succeeded && failures >= 6, "case {case}: exhaust every actual reservation before success");
        match case {
            0 => {
                assert_eq!(live, chosen);
                assert_eq!(values[1], selection(10));
            }
            1 => {
                assert_eq!(live, Selection::default());
                assert_eq!(values[1], selection(10));
            }
            3 => assert_eq!(values, [selection(1), selection(99)]),
            4 => {
                assert_eq!(values, [selection(10), Selection::default()]);
                assert_eq!(live, Selection::default());
            }
            _ => assert_eq!(live, selection(10)),
        }
        println!("selection_memory case={case} owned fallible boundaries={failures}");
    }
    assert!(Preparation::default().vector::<Selection>(usize::MAX).is_err(), "capacity overflow is an error");
}

#[test]
fn finite_controls_bound_payload_and_integer_capacities_after_warmup() {
    let wb = workbook(1);
    let legacy = [Selection::default(), selection(90)];
    let mut values = vec![Selection::default(), legacy[0].clone(), legacy[1].clone()];
    let mut owner = SelectionMemory::default();
    let mut live = Selection::default();
    adopt(&mut owner, &mut values, &wb, &mut live);
    let mut warm = None;
    for n in 1..=1016 {
        let mut added = wb.clone();
        added.sheets.push(Arc::new(Sheet::new("Transient")));
        added.set_active_sheet(1);
        apply(&mut owner, &mut values, &wb, &added, &mut live, Transition::Publish(&selection(n)));
        apply(&mut owner, &mut values, &added, &wb, &mut live, Transition::Publish(&Selection::default()));
        let keep = [wb.sheets[0].uid.get(), added.sheets[1].uid.get()];
        owner.prepare_prune(&values, &wb, &live, &keep).unwrap().apply(&mut values, &mut owner, &mut live);
        assert_eq!(&values[2..], &legacy);
        assert_eq!(owner.labels.len(), 2);
        let capacities =
            (values.capacity(), values.iter().map(|v| v.ranges.capacity()).sum::<usize>(), owner.labels.capacity(), owner.current.capacity());
        if n == 16 {
            warm = Some(capacities);
        }
        if n >= 16 {
            assert_eq!(Some(capacities), warm);
        }
        if n == 16 || n == 1016 {
            println!("selection_memory private cycles={n} outer/ranges/labels/order capacities={capacities:?} (not RSS)");
        }
    }
}

#[test]
fn extracted_target_is_prepared_without_publication_and_same_uid_has_no_replacement() {
    let wb = workbook(2);
    let mut owner = SelectionMemory::default();
    let mut values = vec![selection(1), selection(2)];
    let mut live = selection(10);
    adopt(&mut owner, &mut values, &wb, &mut live);
    let original = (values.clone(), owner.clone(), live.clone());
    let target = owner.prepare(&values, &wb, &wb, &live, Transition::Navigate(1)).unwrap().into_selection();
    assert_eq!(target, Some(selection(2)));
    assert_eq!((&values, &owner, &live), (&original.0, &original.1, &original.2));
    assert!(owner.prepare(&values, &wb, &wb, &live, Transition::Navigate(0)).unwrap().into_selection().is_none());
}

#[test]
fn empty_topology_keeps_all_anonymous_payload_before_fresh_defaults() {
    let before = workbook(0);
    let after = workbook(1);
    let mut owner = SelectionMemory::default();
    let mut values = vec![selection(9), Selection::default()];
    let original = values.clone();
    let mut live = selection(20);
    adopt(&mut owner, &mut values, &before, &mut live);
    assert!(owner.labels.is_empty());
    apply(&mut owner, &mut values, &before, &after, &mut live, Transition::Navigate(0));
    assert_eq!(live, Selection::default());
    assert_eq!(values[0], Selection::default());
    assert_eq!(&values[1..], &original);
}
