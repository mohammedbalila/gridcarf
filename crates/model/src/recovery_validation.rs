//! Read-only typed recovery invariants. Validation does not certify raw JSON
//! budgets/duplicates, opaque graphs, executable adoption, provenance or rendering context.

use crate::{Cell, Color, Sheet, Style, StyleId, StyleTable};
use gridcraft_core::{Array, CellRef, MAX_COLS, RangeRef, Value};

mod features;
mod metadata;
mod objects;
mod structure;

/// Finite, content-free typed failures. Family variants describe malformed stored data;
/// Capacity is validation scratch failure, and UnsupportedRepresentation is ambiguity.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecoveryModelError {
    Storage,
    Coordinate,
    Range,
    Style,
    Value,
    Array,
    CellOwnership,
    Spill,
    Capacity,
    Workbook,
    Geometry,
    Table,
    Filter,
    WhatIf,
    Pivot,
    Control,
    Print,
    Phonetic,
    Python,
    UnsupportedRepresentation(RecoveryRepresentation),
}

/// Operationally ambiguous representations refused without normalizing or dropping data.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecoveryRepresentation {
    AmbiguousIdentity,
    OverlappingRectangles,
}

/// Validate the stored typed workbook without mutation, repair, cloning or rebuilding.
///
/// Checks raw cell storage before iteration/fallback access, then structural, cell/Python,
/// spill, feature, object and metadata invariants. Manual/stale/external and inert content
/// is preserved. Active-sheet state is sampled once; the caller owns capture coherence.
///
/// Success excludes raw JSON predecode budgets/duplicates, opaque graph validity, executable
/// adoption/consent, host metadata/provenance, and screen/page projection. In particular,
/// engine budgets must precede macro helpers, serialization and loaders. This function
/// never authorizes native recovery activation or ordinary destination fidelity.
///
/// Scratch reservations are checked/fallible; string comparison/counting work includes
/// actual bytes. This is not an RSS bound or immunity to failures before typed validation.
pub fn validate_recovery_model(workbook: &crate::Workbook) -> Result<(), RecoveryModelError> {
    // All raw stores are checked before any semantic pass can use fallback lookup/iteration.
    for sheet in &workbook.sheets {
        sheet.cells.validate_recovery_storage()?;
    }
    let style_count = structure::validate_workbook_structure(workbook)?;
    for sheet in &workbook.sheets {
        for (at, cell) in sheet.cells.iter() {
            validate_cell(at, cell, style_count)?;
            if let Some(py) = &cell.py {
                metadata::validate_python(py)?;
            }
        }
        validate_spills(sheet)?;
    }
    features::validate_workbook_features(workbook)?;
    objects::validate_workbook_objects(workbook)?;
    metadata::validate_workbook_metadata(workbook)
}

#[cfg(test)]
mod inventory_tests;

fn validate_cell_ref(at: CellRef) -> Result<(), RecoveryModelError> {
    if !at.is_valid() {
        return Err(RecoveryModelError::Coordinate);
    }
    Ok(())
}

/// Returns area without converting full-sheet metadata to a native allocation size.
fn validate_range(range: RangeRef) -> Result<u64, RecoveryModelError> {
    validate_cell_ref(range.start)?;
    validate_cell_ref(range.end)?;
    let height = range.end.row.checked_sub(range.start.row).and_then(|n| n.checked_add(1)).ok_or(RecoveryModelError::Range)?;
    let width = range.end.col.checked_sub(range.start.col).and_then(|n| n.checked_add(1)).ok_or(RecoveryModelError::Range)?;
    u64::from(height).checked_mul(u64::from(width)).ok_or(RecoveryModelError::Range)
}

fn validate_scalar(value: &Value) -> Result<(), RecoveryModelError> {
    match value {
        Value::Number(n) if !n.is_finite() => Err(RecoveryModelError::Value),
        Value::Array(_) => Err(RecoveryModelError::Array),
        Value::Empty | Value::Number(_) | Value::Text(_) | Value::Bool(_) | Value::Error(_) => Ok(()),
    }
}

fn validate_array(array: &Array) -> Result<(), RecoveryModelError> {
    if array.rows == 0 || array.cols == 0 || array.rows.checked_mul(array.cols) != Some(array.data.len()) {
        return Err(RecoveryModelError::Array);
    }
    for value in &array.data {
        validate_scalar(value)?;
    }
    Ok(())
}

fn validate_value(value: &Value) -> Result<(), RecoveryModelError> {
    match value {
        Value::Array(array) => validate_array(array),
        scalar => validate_scalar(scalar),
    }
}

fn validate_style_id(id: StyleId, count: usize) -> Result<(), RecoveryModelError> {
    if usize::try_from(id.0).map_err(|_| RecoveryModelError::Style)? >= count {
        return Err(RecoveryModelError::Style);
    }
    Ok(())
}

fn validate_color(color: Color) -> Result<(), RecoveryModelError> {
    match color {
        Color::Auto => Ok(()),
        Color::Rgb(rgb) if rgb <= 0xffffff => Ok(()),
        Color::Theme(slot, tint) if slot < 12 && (-1000..=1000).contains(&tint) => Ok(()),
        Color::Rgb(_) | Color::Theme(_, _) => Err(RecoveryModelError::Style),
    }
}

fn validate_style(style: &Style) -> Result<(), RecoveryModelError> {
    if !style.font.size.is_finite() || !((-90..=90).contains(&style.align.rotation) || style.align.rotation == 255) {
        return Err(RecoveryModelError::Style);
    }
    for color in [
        style.font.color,
        style.fill.fg,
        style.fill.bg,
        style.borders.left.color,
        style.borders.right.color,
        style.borders.top.color,
        style.borders.bottom.color,
        style.borders.diag_down.color,
        style.borders.diag_up.color,
    ] {
        validate_color(color)?;
    }
    Ok(())
}

fn validate_style_table(styles: &StyleTable) -> Result<usize, RecoveryModelError> {
    let count = styles.len();
    if count == 0 || u32::try_from(count - 1).is_err() {
        return Err(RecoveryModelError::Style);
    }
    // Iteration borrows the sequence, without fallback access, hashing or index rebuilding.
    for (_, style) in styles.iter() {
        validate_style(style)?;
    }
    Ok(count)
}

fn validate_cell(at: CellRef, cell: &Cell, style_count: usize) -> Result<(), RecoveryModelError> {
    validate_cell_ref(at)?;
    validate_style_id(cell.style, style_count)?;
    validate_value(&cell.value)?;
    if cell.formula.is_some() && cell.py.is_some() {
        return Err(RecoveryModelError::CellOwnership);
    }
    if let Some(formula) = &cell.formula
        && let Some(range) = formula.array
    {
        validate_range(range)?;
        if range.start != at {
            return Err(RecoveryModelError::CellOwnership);
        }
    }
    Ok(())
}

struct SpillOwner {
    anchor: CellRef,
    expected: u64,
    seen: u64,
}

#[derive(Clone, Copy)]
struct SpillEvent {
    row: u32,
    start: bool,
    owner: usize,
    first: u32,
    last: u32,
}

fn apply_spill_event(event: SpillEvent, columns: &mut [Option<usize>]) -> Result<(), RecoveryModelError> {
    let first = usize::try_from(event.first).map_err(|_| RecoveryModelError::Capacity)?;
    let end = usize::try_from(event.last).map_err(|_| RecoveryModelError::Capacity)?.checked_add(1).ok_or(RecoveryModelError::Capacity)?;
    for slot in columns.get_mut(first..end).ok_or(RecoveryModelError::Spill)? {
        if event.start {
            if slot.is_some() {
                return Err(RecoveryModelError::Spill);
            }
            *slot = Some(event.owner);
        } else {
            if *slot != Some(event.owner) {
                return Err(RecoveryModelError::Spill);
            }
            *slot = None;
        }
    }
    Ok(())
}

fn validate_spills(sheet: &Sheet) -> Result<(), RecoveryModelError> {
    sheet.cells.validate_recovery_storage()?;
    let mut expected = 0u64;
    for (anchor, range) in &sheet.spill_ranges {
        validate_cell_ref(*anchor)?;
        let area = validate_range(*range)?;
        let cell = sheet.cells.get(*anchor).ok_or(RecoveryModelError::Spill)?;
        if *anchor != range.start || cell.formula.is_some() == cell.py.is_some() {
            return Err(RecoveryModelError::Spill);
        }
        expected = expected.checked_add(area.checked_sub(1).ok_or(RecoveryModelError::Spill)?).ok_or(RecoveryModelError::Capacity)?;
    }
    // This gate MUST precede scratch reservation and column-span walks: sum(width) <= V+R.
    if expected != u64::try_from(sheet.spill.len()).map_err(|_| RecoveryModelError::Capacity)? {
        return Err(RecoveryModelError::Spill);
    }
    let count = sheet.spill_ranges.len();
    let event_count = count.checked_mul(2).ok_or(RecoveryModelError::Capacity)?;
    let column_count = usize::try_from(MAX_COLS).map_err(|_| RecoveryModelError::Capacity)?;
    for (length, size) in [
        (count, std::mem::size_of::<SpillOwner>()),
        (event_count, std::mem::size_of::<SpillEvent>()),
        (column_count, std::mem::size_of::<Option<usize>>()),
    ] {
        if length.checked_mul(size).is_none_or(|bytes| bytes > isize::MAX as usize) {
            return Err(RecoveryModelError::Capacity);
        }
    }
    let mut owners = Vec::new();
    owners.try_reserve_exact(count).map_err(|_| RecoveryModelError::Capacity)?;
    let mut events = Vec::new();
    events.try_reserve_exact(event_count).map_err(|_| RecoveryModelError::Capacity)?;
    let mut columns = Vec::new();
    columns.try_reserve_exact(column_count).map_err(|_| RecoveryModelError::Capacity)?;
    columns.resize(column_count, None);
    for (anchor, range) in &sheet.spill_ranges {
        let owner = owners.len();
        owners.push(SpillOwner { anchor: *anchor, expected: validate_range(*range)?.checked_sub(1).ok_or(RecoveryModelError::Spill)?, seen: 0 });
        events.push(SpillEvent { row: range.start.row, start: true, owner, first: range.start.col, last: range.end.col });
        events.push(SpillEvent {
            row: range.end.row.checked_add(1).ok_or(RecoveryModelError::Spill)?,
            start: false,
            owner,
            first: range.start.col,
            last: range.end.col,
        });
    }
    // end+1 can be MAX_ROWS; false sorts before true so adjacent rows can reuse columns.
    events.sort_unstable_by_key(|e| (e.row, e.start, e.owner));
    let mut events = events.into_iter().peekable();
    for (at, value) in &sheet.spill {
        validate_cell_ref(*at)?;
        while events.peek().is_some_and(|event| event.row <= at.row) {
            if let Some(event) = events.next() {
                apply_spill_event(event, &mut columns)?;
            }
        }
        let col = usize::try_from(at.col).map_err(|_| RecoveryModelError::Coordinate)?;
        let index = columns.get(col).copied().flatten().ok_or(RecoveryModelError::Spill)?;
        let owner = owners.get_mut(index).ok_or(RecoveryModelError::Spill)?;
        if *at == owner.anchor {
            return Err(RecoveryModelError::Spill);
        }
        validate_scalar(value)?;
        owner.seen = owner.seen.checked_add(1).ok_or(RecoveryModelError::Capacity)?;
    }
    // Zero-child owners and ranges beyond the last cached point still need overlap checks.
    for event in events {
        apply_spill_event(event, &mut columns)?;
    }
    if owners.iter().any(|owner| owner.seen != owner.expected) {
        return Err(RecoveryModelError::Spill);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Formula, FormulaEvalMode, PyCode, PyOutput, Workbook};
    use gridcraft_core::{CellError, MAX_ROWS};
    use std::sync::Arc;

    fn at(row: u32, col: u32) -> CellRef {
        CellRef::new(row, col)
    }
    fn rect(r0: u32, c0: u32, r1: u32, c1: u32) -> RangeRef {
        RangeRef { start: at(r0, c0), end: at(r1, c1) }
    }
    fn owner(sheet: &mut Sheet, range: RangeRef, python: bool) {
        let cell =
            if python { Cell { py: Some(Arc::new(PyCode::new("[1]"))), ..Cell::default() } } else { Cell::formula(Formula::new("=unparseable(")) };
        sheet.cells.set(range.start, cell);
        sheet.spill_ranges.insert(range.start, range);
    }
    fn complete(range: RangeRef, python: bool) -> Sheet {
        let mut sheet = Sheet::new("synthetic");
        owner(&mut sheet, range, python);
        // Fixtures are deliberately tiny; the production validator must never enumerate areas.
        for cell in range.iter().filter(|c| *c != range.start) {
            sheet.spill.insert(cell, Value::Empty);
        }
        sheet
    }

    #[test]
    fn recovery_default_and_sparse_edge_primitives_are_valid() {
        let mut wb = Workbook::new();
        let id = wb.styles.intern(Style { fill: crate::Fill::solid(Color::Rgb(0x123456)), ..Style::default() });
        let style_count = validate_style_table(&wb.styles).unwrap();
        let sheet = wb.sheet_mut(0).unwrap();
        sheet.cells.set(at(0, 0), Cell::styled(Value::Empty, id));
        sheet.cells.set(at(MAX_ROWS - 1, MAX_COLS - 1), Cell::value(Value::Number(-0.0)));
        assert_eq!(sheet.cells.validate_recovery_storage(), Ok(2));
        for (c, cell) in sheet.cells.iter() {
            assert_eq!(validate_cell(c, cell, style_count), Ok(()));
        }
        assert_eq!(validate_spills(sheet), Ok(()));
        assert_eq!(validate_range(RangeRef::all()), Ok(17_179_869_184));
        assert_eq!(validate_range(RangeRef::rows(0, 0)), Ok(16_384));
        assert_eq!(validate_range(RangeRef::cols(0, 0)), Ok(1_048_576));
    }

    #[test]
    fn recovery_coordinates_and_ranges_refuse_unchecked_endpoints_and_order() {
        assert_eq!(validate_cell_ref(at(MAX_ROWS - 1, MAX_COLS - 1)), Ok(()));
        for c in [at(MAX_ROWS, 0), at(0, MAX_COLS), at(u32::MAX, u32::MAX)] {
            assert_eq!(validate_cell_ref(c), Err(RecoveryModelError::Coordinate));
            assert_eq!(validate_range(RangeRef::cell(c)), Err(RecoveryModelError::Coordinate));
        }
        for r in [rect(1, 0, 0, 0), rect(0, 1, 0, 0), rect(1, 1, 0, 0)] {
            assert_eq!(validate_range(r), Err(RecoveryModelError::Range));
        }
        assert_eq!(validate_range(rect(2, 3, 2, 3)), Ok(1));
    }

    #[test]
    fn recovery_scalar_and_array_validation_preserves_payload_identity_and_bits() {
        let text: Arc<str> = Arc::from("inert text");
        let values = vec![Value::Empty, Value::Number(-0.0), Value::Text(text.clone()), Value::Bool(true), Value::Error(CellError::Circ)];
        for v in &values {
            assert_eq!(validate_scalar(v), Ok(()));
        }
        for e in CellError::ALL {
            assert_eq!(validate_scalar(&Value::Error(e)), Ok(()));
        }
        let array = Arc::new(Array { rows: 1, cols: 5, data: values });
        let shared = array.clone();
        let value = Value::Array(array);
        assert_eq!(validate_value(&value), Ok(()));
        assert_eq!(Arc::strong_count(&shared), 2);
        let Value::Array(after) = &value else {
            panic!("array");
        };
        assert!(Arc::ptr_eq(after, &shared));
        assert_eq!(after.data[1], Value::Number(-0.0));
        let Value::Number(n) = after.data[1] else {
            panic!("number");
        };
        assert_eq!(n.to_bits(), (-0.0f64).to_bits());
        let Value::Text(t) = &after.data[2] else {
            panic!("text");
        };
        assert!(Arc::ptr_eq(t, &text));
        assert_eq!(validate_value(&Value::Array(Arc::new(Array::scalar(Value::Empty)))), Ok(()));
        assert_eq!(validate_value(&Value::Array(Arc::new(Array { rows: 2, cols: 3, data: vec![Value::Bool(false); 6] }))), Ok(()));
    }

    #[test]
    fn recovery_malformed_arrays_and_nonfinite_leaves_are_refused_without_mutation() {
        for array in [
            Array { rows: 0, cols: 1, data: vec![] },
            Array { rows: 1, cols: 0, data: vec![] },
            Array { rows: usize::MAX, cols: 2, data: vec![] },
            Array { rows: 2, cols: 2, data: vec![Value::Empty] },
            Array::scalar(Value::Array(Arc::new(Array::scalar(Value::Empty)))),
        ] {
            assert_eq!(validate_array(&array), Err(RecoveryModelError::Array));
        }
        for n in [f64::from_bits(0x7ff8_0000_0000_0042), f64::INFINITY, f64::NEG_INFINITY] {
            let v = Value::Number(n);
            assert_eq!(validate_scalar(&v), Err(RecoveryModelError::Value));
            let array = Arc::new(Array { rows: 1, cols: 2, data: vec![Value::from("valid first"), v] });
            let count = Arc::strong_count(&array);
            assert_eq!(validate_value(&Value::Array(array.clone())), Err(RecoveryModelError::Value));
            assert_eq!(Arc::strong_count(&array), count);
            let Value::Number(after) = array.data[1] else {
                panic!("number");
            };
            assert_eq!(after.to_bits(), n.to_bits());
            let output = PyOutput { value: Value::Array(array), ..PyOutput::default() };
            assert_eq!(validate_value(&output.value), Err(RecoveryModelError::Value));
        }
    }

    #[test]
    fn recovery_style_domains_keep_custom_normal_duplicates_and_finite_signed_sizes() {
        for size in [-f32::MAX, -12.0, -0.0, 0.0, f32::MAX] {
            let mut style = Style::default();
            style.font.size = size;
            style.font.charset = Some(255);
            style.font.family = Some(255);
            style.dxf_stated.scheme = Some("unknown inert scheme".into());
            for rotation in [-90, 0, 90, 255] {
                style.align.rotation = rotation;
                assert_eq!(validate_style(&style), Ok(()));
                assert_eq!(style.font.size.to_bits(), size.to_bits());
            }
            let table: StyleTable = serde_json::from_value(serde_json::json!({"styles": [style.clone(), style]})).unwrap();
            assert_eq!(validate_style_table(&table), Ok(2));
            assert_eq!(validate_style_id(StyleId(1), 2), Ok(()));
        }
        for color in [Color::Auto, Color::Rgb(0), Color::Rgb(0xffffff), Color::Theme(0, -1000), Color::Theme(11, 1000)] {
            assert_eq!(validate_color(color), Ok(()));
        }
    }

    #[test]
    fn recovery_bad_style_table_ids_and_every_nested_color_are_refused() {
        let table: StyleTable = serde_json::from_str("{\"styles\":[]}").unwrap();
        assert_eq!(validate_style_table(&table), Err(RecoveryModelError::Style));
        for id in [StyleId(2), StyleId(u32::MAX)] {
            assert_eq!(validate_style_id(id, 2), Err(RecoveryModelError::Style));
        }
        for size in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            let mut style = Style::default();
            style.font.size = size;
            assert_eq!(validate_style(&style), Err(RecoveryModelError::Style));
            assert_eq!(style.font.size.to_bits(), size.to_bits());
        }
        for rotation in [-91, 91, 254, 256] {
            let mut style = Style::default();
            style.align.rotation = rotation;
            assert_eq!(validate_style(&style), Err(RecoveryModelError::Style));
        }
        for color in [Color::Rgb(0x1000000), Color::Theme(12, 0), Color::Theme(0, -1001), Color::Theme(11, 1001)] {
            for field in 0..9 {
                let mut s = Style::default();
                match field {
                    0 => s.font.color = color,
                    1 => s.fill.fg = color,
                    2 => s.fill.bg = color,
                    3 => s.borders.left.color = color,
                    4 => s.borders.right.color = color,
                    5 => s.borders.top.color = color,
                    6 => s.borders.bottom.color = color,
                    7 => s.borders.diag_down.color = color,
                    _ => s.borders.diag_up.color = color,
                }
                assert_eq!(validate_style(&s), Err(RecoveryModelError::Style), "field {field}");
            }
        }
    }

    #[test]
    fn recovery_formula_modes_flags_parse_caches_and_array_anchors_are_preserved() {
        for mode in [FormulaEvalMode::Array, FormulaEvalMode::Legacy] {
            for dynamic in [false, true] {
                for text in ["=SUM(1,2)", "=unparseable("] {
                    let mut formula = Formula::new(text);
                    formula.eval_mode = mode;
                    formula.dynamic = dynamic;
                    formula.array = Some(rect(2, 3, 4, 5));
                    let formula = Arc::new(formula);
                    let cell =
                        Cell { formula: Some(formula.clone()), value: Value::Array(Arc::new(Array::scalar(Value::Number(-0.0)))), ..Cell::default() };
                    assert_eq!(validate_cell(at(2, 3), &cell, 1), Ok(()));
                    assert!(Arc::ptr_eq(cell.formula.as_ref().unwrap(), &formula));
                    assert_eq!(Arc::strong_count(&formula), 2);
                    assert_eq!((formula.eval_mode, formula.dynamic), (mode, dynamic));
                    let mut absent = (*formula).clone();
                    absent.expr = None;
                    assert_eq!(validate_cell(at(2, 3), &Cell::formula(absent), 1), Ok(()));
                }
            }
        }
        assert_eq!(validate_cell(at(0, 0), &Cell { py: Some(Arc::new(PyCode::default())), ..Cell::default() }, 1), Ok(()));
    }

    #[test]
    fn recovery_cell_ownership_and_array_metadata_are_checked() {
        let mut cell = Cell::formula(Formula::new("=1"));
        cell.py = Some(Arc::new(PyCode::default()));
        assert_eq!(validate_cell(at(0, 0), &cell, 1), Err(RecoveryModelError::CellOwnership));
        cell.py = None;
        cell.style = StyleId(1);
        assert_eq!(validate_cell(at(0, 0), &cell, 1), Err(RecoveryModelError::Style));
        for r in [rect(1, 1, 2, 2), rect(1, 0, 0, 0), rect(0, 0, MAX_ROWS, 0)] {
            let mut f = Formula::new("=1");
            f.array = Some(r);
            assert!(validate_cell(at(0, 0), &Cell::formula(f), 1).is_err());
        }
    }

    #[test]
    fn recovery_complete_formula_python_empty_and_shadowed_spills_are_preserved() {
        for python in [false, true] {
            let mut sheet = complete(rect(2, 3, 3, 4), python);
            sheet.cells.set(at(2, 4), Cell::value(Value::Number(99.0)));
            sheet.cells.set(at(3, 3), Cell::formula(Formula::new("=42")));
            sheet.cells.set(at(3, 4), Cell::styled(Value::Empty, StyleId(1)));
            let before = sheet.clone();
            assert_eq!(validate_spills(&sheet), Ok(()));
            assert_eq!(sheet, before);
            assert_eq!(sheet.value_ref(at(2, 4)), Some(&Value::Number(99.0)));
            assert_eq!(sheet.value_ref(at(3, 3)), Some(&Value::Empty));
            assert_eq!(sheet.spill.len(), 3);
        }
    }

    #[test]
    fn recovery_spill_sweep_handoffs_adjacent_columns_rows_and_last_row_sentinel() {
        for ranges in
            [vec![rect(0, 0, 1, 0), rect(2, 0, 3, 0)], vec![rect(0, 0, 0, 1), rect(0, 2, 0, 3)], vec![rect(MAX_ROWS - 2, 0, MAX_ROWS - 1, 0)]]
        {
            let mut sheet = Sheet::new("synthetic");
            for range in ranges {
                let part = complete(range, false);
                sheet.cells.set(range.start, part.cells.get(range.start).unwrap().clone());
                sheet.spill_ranges.extend(part.spill_ranges);
                sheet.spill.extend(part.spill);
            }
            assert_eq!(validate_spills(&sheet), Ok(()));
        }
        let mut sheet = Sheet::new("zero children");
        owner(&mut sheet, rect(0, 0, 0, 0), false);
        owner(&mut sheet, rect(1, 0, 1, 0), true);
        owner(&mut sheet, rect(MAX_ROWS - 1, MAX_COLS - 1, MAX_ROWS - 1, MAX_COLS - 1), false);
        assert_eq!(validate_spills(&sheet), Ok(()));
        assert!(sheet.spill.is_empty());
    }

    #[test]
    fn recovery_spill_true_overlap_and_wrong_anchor_are_refused() {
        for (a, b, points) in [
            (rect(0, 0, 1, 1), rect(1, 1, 2, 2), vec![at(0, 1), at(1, 0), at(1, 1), at(1, 2), at(2, 1), at(2, 2)]),
            (rect(3, 3, 4, 4), rect(4, 4, 4, 4), vec![at(3, 4), at(4, 3), at(4, 4)]),
        ] {
            let mut sheet = Sheet::new("overlap");
            owner(&mut sheet, a, false);
            owner(&mut sheet, b, true);
            for point in points {
                sheet.spill.insert(point, Value::Empty);
            }
            assert_eq!(validate_spills(&sheet), Err(RecoveryModelError::Spill));
        }
        // A wrong top-left key is refused in the prepass, even with V=0.
        let mut sheet = Sheet::new("remaining");
        owner(&mut sheet, rect(2, 2, 2, 2), false);
        sheet.spill_ranges.insert(at(3, 3), rect(2, 2, 2, 2));
        assert_eq!(validate_spills(&sheet), Err(RecoveryModelError::Spill));
    }

    #[test]
    fn recovery_orphan_missing_anchor_wrong_owner_and_cache_keys_are_refused() {
        let good = complete(rect(0, 0, 1, 1), false);
        let mut bad = good.clone();
        bad.spill.remove(&at(1, 1));
        assert_eq!(validate_spills(&bad), Err(RecoveryModelError::Spill));
        bad.spill.insert(at(9, 9), Value::Empty); // Aggregate count balances; exact coverage does not.
        assert_eq!(validate_spills(&bad), Err(RecoveryModelError::Spill));
        for key in [at(0, 0), at(MAX_ROWS, 0), at(0, MAX_COLS)] {
            let mut bad = good.clone();
            bad.spill.remove(&at(1, 1));
            bad.spill.insert(key, Value::Empty);
            assert!(validate_spills(&bad).is_err());
        }
        for cell in [
            None,
            Some(Cell::value(Value::Number(1.0))),
            Some(Cell { formula: Some(Arc::new(Formula::new("=1"))), py: Some(Arc::new(PyCode::default())), ..Cell::default() }),
        ] {
            let mut bad = good.clone();
            bad.cells.remove(at(0, 0));
            if let Some(cell) = cell {
                bad.cells.set(at(0, 0), cell);
            }
            assert_eq!(validate_spills(&bad), Err(RecoveryModelError::Spill));
        }
        let mut orphan = Sheet::new("orphan");
        orphan.spill.insert(at(0, 0), Value::Empty);
        assert_eq!(validate_spills(&orphan), Err(RecoveryModelError::Spill));
        let mut wrong = good.clone();
        wrong.spill_ranges.clear();
        wrong.spill_ranges.insert(at(1, 1), rect(0, 0, 1, 1));
        assert_eq!(validate_spills(&wrong), Err(RecoveryModelError::Spill));
    }

    #[test]
    fn recovery_huge_spill_claim_and_invalid_ranges_refuse_before_coverage_work() {
        let mut sheet = Sheet::new("huge");
        owner(&mut sheet, RangeRef::all(), false);
        assert_eq!(validate_spills(&sheet), Err(RecoveryModelError::Spill));
        for range in [rect(0, 0, MAX_ROWS, 0), rect(1, 0, 0, 0), rect(0, 1, 0, 0)] {
            let mut sheet = Sheet::new("bad range");
            owner(&mut sheet, range, false);
            assert!(validate_spills(&sheet).is_err());
        }
    }

    #[test]
    fn recovery_late_nonfinite_and_nested_spill_leaves_preserve_sharing() {
        for value in
            [Value::Number(f64::from_bits(0x7ff8_0000_0000_0033)), Value::Number(f64::INFINITY), Value::Array(Arc::new(Array::scalar(Value::Empty)))]
        {
            let mut sheet = complete(rect(0, 0, 1, 1), false);
            sheet.spill.insert(at(1, 1), value);
            let formula = sheet.cells.get(at(0, 0)).unwrap().formula.as_ref().unwrap();
            let count = Arc::strong_count(formula);
            let pointer = Arc::as_ptr(formula);
            let before = sheet.spill.get(&at(1, 1)).unwrap();
            assert!(validate_spills(&sheet).is_err());
            assert_eq!(Arc::strong_count(formula), count);
            assert_eq!(Arc::as_ptr(formula), pointer);
            assert!(std::ptr::eq(before, sheet.spill.get(&at(1, 1)).unwrap()));
            if let Value::Number(n) = before {
                assert_eq!(
                    sheet.spill.get(&at(1, 1)).and_then(|v| if let Value::Number(n) = v { Some(n.to_bits()) } else { None }),
                    Some(n.to_bits())
                );
            }
        }
    }
}
