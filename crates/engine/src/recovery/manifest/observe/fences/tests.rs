use super::*;
use std::{cell::Cell, sync::Arc};

#[test]
fn checked_expansion_and_occurrences_fence_actual_projection_entry() {
    let mut workbook = Workbook::new();
    workbook.vba_project = Some(Arc::new(vec![0; 4]));
    assert!(matches!(check(&workbook, &RecoveryLimits { manifest: 7, ..RecoveryLimits::default() }), Err(RecoveryError::TooLarge("base64"))));
    assert!(encoded(usize::MAX).is_err());
    let exact = check(&workbook, &RecoveryLimits { manifest: 8, part_bytes: 0, ..RecoveryLimits::default() }).unwrap();
    assert_eq!((exact.raw, exact.encoded, exact.largest_b64), (4, 8, 8));
    let entered = Cell::new(0);
    // Explicit proxy for the post-fence Serialize boundary, not a model allocator counter.
    if check(&workbook, &RecoveryLimits { manifest: 7, ..RecoveryLimits::default() }).is_ok() {
        entered.set(1);
    }
    assert_eq!(entered.get(), 0);
}

#[test]
fn picture_reference_groups_and_a1_requests_are_bounded_before_formatting() {
    let mut workbook = Workbook::new();
    let picture = Arc::new(gridcraft_model::CellPicture { data: vec![1; 2], mime: "x".into(), alt: "".into() });
    let sheet = Arc::make_mut(&mut workbook.sheets[0]);
    sheet.cell_pictures.insert(CellRef::new(0, 0), picture.clone());
    sheet.cell_pictures.insert(CellRef::new(0, 1), picture);
    sheet.spill.insert(CellRef::new(0, 2), gridcraft_core::Value::Empty);
    let stats = check(&workbook, &RecoveryLimits::default()).unwrap();
    assert_eq!((stats.raw, stats.encoded, stats.largest_b64, stats.picture_vector, stats.group_vector, stats.identity_entries), (4, 8, 4, 32, 8, 2));
    assert!(check(&workbook, &RecoveryLimits { nodes: 29, ..RecoveryLimits::default() }).is_err());
    assert!(matches!(check(&workbook, &RecoveryLimits { cells: 0, ..RecoveryLimits::default() }), Err(RecoveryError::TooLarge("cells"))));
    Arc::make_mut(&mut workbook.sheets[0]).spill.insert(CellRef::new(1_048_576, 0), gridcraft_core::Value::Empty);
    assert!(matches!(check(&workbook, &RecoveryLimits::default()), Err(RecoveryError::Malformed("a1_key"))));
    use std::io::Write;
    writeln!(std::io::stdout(), "D1_FENCE_REQUESTS {:?}", stats).unwrap();
}
