use super::*;
use std::sync::Arc;

fn selected(n: u32) -> Selection {
    Selection {
        active: CellRef::new(n, 1),
        anchor: CellRef::new(n, 4),
        ranges: vec![RangeRef::cell(CellRef::new(n, 1)), RangeRef::cell(CellRef::new(n, 4))],
    }
}
fn workbook() -> Workbook {
    let mut wb = Workbook::new();
    wb.sheets.push(Arc::new(gridcraft_model::Sheet::new("B")));
    wb
}
fn adopt(values: &mut Vec<Selection>, wb: &Workbook, live: &mut Selection) -> SelectionMemory {
    let mut owner = SelectionMemory::default();
    owner.prepare(values, wb, wb, live, Transition::Refresh).unwrap().apply(values, &mut owner, live);
    owner
}
#[test]
fn exact_same_target_restore_never_pads_or_publishes_stale_slot() {
    for len in [0, 1, 2, 5] {
        let wb = workbook();
        let mut values: Vec<_> = (0..len).map(selected).collect();
        let mut live = selected(70);
        let mut owner = adopt(&mut values, &wb, &mut live);
        let before = (values.clone(), owner.clone(), values.as_ptr(), values.capacity());
        let plan = owner.prepare(&values, &wb, &wb, &live, Transition::Restore { sheet: 0, captured: Some(wb.sheets[0].uid.get()) }).unwrap();
        plan.apply(&mut values, &mut owner, &mut live);
        assert_eq!((values.clone(), owner, values.as_ptr(), values.capacity()), before);
        assert_eq!(live, selected(70));
    }
}
#[test]
fn captured_uid_not_current_workbook_active_owns_outgoing_live() {
    let mut wb = workbook();
    let (a, b) = (wb.sheets[0].uid.get(), wb.sheets[1].uid.get());
    wb.set_active_sheet(1);
    let mut values = vec![selected(3), selected(4), selected(90), Selection::default()];
    let mut live = selected(7);
    let mut owner = adopt(&mut values, &wb, &mut live);
    wb.sheets.remove(1);
    wb.set_active_sheet(0);
    owner.prepare(&values, &wb, &wb, &live, Transition::Restore { sheet: 0, captured: Some(b) }).unwrap().apply(&mut values, &mut owner, &mut live);
    assert_eq!(owner.labels, vec![a, b]);
    assert_eq!(values, vec![selected(3), selected(7), selected(90), Selection::default()]);
    assert_eq!(live, selected(3));
    owner.prepare_prune(&values, &wb, &live, &[a]).unwrap().apply(&mut values, &mut owner, &mut live);
    assert_eq!(values, vec![selected(3), selected(90), Selection::default()]);
}
#[test]
fn restore_remaps_uid_and_never_promotes_anonymous_suffix() {
    let mut wb = workbook();
    let b = wb.sheets[1].uid.get();
    wb.set_active_sheet(1);
    let mut values = vec![selected(3), selected(4), selected(90)];
    let mut live = selected(7);
    let mut owner = adopt(&mut values, &wb, &mut live);
    wb.sheets.swap(0, 1);
    owner.prepare(&values, &wb, &wb, &live, Transition::Restore { sheet: 0, captured: Some(b) }).unwrap().apply(&mut values, &mut owner, &mut live);
    assert_eq!(values, vec![selected(4), selected(3), selected(90)]);
    assert_eq!(live, selected(7));
    wb.sheets[0] = Arc::new(gridcraft_model::Sheet::new("B"));
    owner.prepare(&values, &wb, &wb, &live, Transition::Restore { sheet: 0, captured: Some(b) }).unwrap().apply(&mut values, &mut owner, &mut live);
    assert_eq!(live, Selection::default());
    assert_eq!(values, vec![Selection::default(), selected(3), selected(7), selected(90)]);
}
#[test]
fn empty_and_duplicate_restore_preserve_payload_without_identity_claim() {
    let mut wb = workbook();
    let a = wb.sheets[0].uid.get();
    let mut values = vec![selected(3), selected(4), selected(90)];
    let mut live = selected(7);
    let mut owner = adopt(&mut values, &wb, &mut live);
    wb.sheets.clear();
    owner.prepare(&values, &wb, &wb, &live, Transition::Restore { sheet: 0, captured: Some(a) }).unwrap().apply(&mut values, &mut owner, &mut live);
    assert_eq!(live, Selection::default());
    assert_eq!(values, vec![selected(7), selected(4), selected(90)]);
    let mut wb = workbook();
    wb.sheets[1] = wb.sheets[0].clone();
    let before = values.clone();
    live = selected(8);
    owner.prepare(&values, &wb, &wb, &live, Transition::Restore { sheet: 1, captured: None }).unwrap().apply(&mut values, &mut owner, &mut live);
    assert_eq!(values, before);
    assert_eq!(live, selected(8));
    assert_eq!(owner.published_len, None);
}
#[test]
fn every_owned_restore_preparation_failure_keeps_input_buffers_and_labels() {
    let mut wb = workbook();
    let b = wb.sheets[1].uid.get();
    wb.set_active_sheet(1);
    let mut values = vec![selected(3), selected(4), selected(90)];
    let mut live = selected(7);
    let owner = adopt(&mut values, &wb, &mut live);
    wb.sheets.remove(1);
    wb.set_active_sheet(0);
    let before = (values.clone(), owner.clone(), live.clone(), values.as_ptr(), values.capacity(), owner.labels.as_ptr());
    let mut failures = 0;
    for remaining in 0..30 {
        let result = owner.prepare_with(
            &values,
            &wb,
            &wb,
            &live,
            Transition::Restore { sheet: 0, captured: Some(b) },
            &mut Preparation { keep: None, remaining: Some(remaining) },
        );
        assert_eq!((values.clone(), owner.clone(), live.clone(), values.as_ptr(), values.capacity(), owner.labels.as_ptr()), before);
        match result {
            Err(_) => failures += 1,
            Ok(plan) => {
                let mut output = values.clone();
                let mut next_owner = owner.clone();
                let mut next_live = live.clone();
                plan.apply(&mut output, &mut next_owner, &mut next_live);
                assert_eq!(output, vec![selected(3), selected(7), selected(90)]);
                assert_eq!(next_live, selected(3));
                assert!(failures > 0, "real owned reservations must be exercised");
                return;
            }
        }
    }
    panic!("no preparation succeeded within the test allowance");
}
