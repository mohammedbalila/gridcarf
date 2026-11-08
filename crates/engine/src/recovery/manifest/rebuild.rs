//! Derived reconstruction of an owned detached candidate, not complete recovery acceptance.
use super::{Metadata, RecoveryError, RecoveryLimits, Workbook, check_executable};
use gridcraft_core::CellRef;
use gridcraft_model::{Formula, RecoveryModelError, RecoveryRepresentation};
use std::sync::Arc;

#[derive(Debug, Default)]
#[cfg_attr(test, derive(serde::Serialize))]
struct Stats {
    d2_calls: usize,
    model_before: usize,
    model_after: usize,
    sheet_detach: usize,
    store_calls: usize,
    style_calls: usize,
    style_entries: usize,
    actual_before: usize,
    actual_after: usize,
    drops: usize,
    formulas: usize,
    parsed: usize,
    text_bytes: usize,
    coordinate_request: usize,
    coordinate_capacity: usize,
}
pub(super) fn model_error(error: RecoveryModelError) -> RecoveryError {
    use RecoveryModelError::*;
    match error {
        Storage => RecoveryError::Malformed("model_storage"),
        Coordinate => RecoveryError::Malformed("model_coordinate"),
        Range => RecoveryError::Malformed("model_range"),
        Style => RecoveryError::Malformed("model_style"),
        Value => RecoveryError::Malformed("model_value"),
        Array => RecoveryError::Malformed("model_array"),
        CellOwnership => RecoveryError::Malformed("model_cell_ownership"),
        Spill => RecoveryError::Malformed("model_spill"),
        Capacity => RecoveryError::TooLarge("model_allocation"),
        Workbook => RecoveryError::Malformed("model_workbook"),
        Geometry => RecoveryError::Malformed("model_geometry"),
        Table => RecoveryError::Malformed("model_table"),
        Filter => RecoveryError::Malformed("model_filter"),
        WhatIf => RecoveryError::Malformed("model_what_if"),
        Pivot => RecoveryError::Malformed("model_pivot"),
        Control => RecoveryError::Malformed("model_control"),
        Print => RecoveryError::Malformed("model_print"),
        Phonetic => RecoveryError::Malformed("model_phonetic"),
        Python => RecoveryError::Malformed("model_python"),
        UnsupportedRepresentation(reason) => match reason {
            RecoveryRepresentation::AmbiguousIdentity => RecoveryError::UnsupportedRepresentation("model_identity"),
            RecoveryRepresentation::OverlappingRectangles => RecoveryError::UnsupportedRepresentation("model_rectangles"),
        },
    }
}
fn run(mut candidate: Workbook, metadata: &Metadata, limits: &RecoveryLimits, stats: &mut Stats) -> Result<Workbook, RecoveryError> {
    stats.d2_calls += 1;
    check_executable(&candidate, metadata, limits, false)?;
    stats.model_before += 1;
    gridcraft_model::validate_recovery_model(&candidate).map_err(model_error)?;
    for sheet in &mut candidate.sheets {
        // Only the complete readonly model gate permits physical iteration/counting here.
        let before = sheet.cells.iter().count();
        stats.actual_before += before;
        let formulas = sheet.cells.iter().filter(|(_, cell)| cell.formula.is_some()).count();
        let bytes = formulas
            .checked_mul(std::mem::size_of::<CellRef>())
            .filter(|n| *n <= isize::MAX as usize)
            .ok_or(RecoveryError::TooLarge("formula_coordinates"))?;
        let mut coordinates = Vec::new();
        coordinates.try_reserve_exact(formulas).map_err(|_| RecoveryError::TooLarge("formula_coordinates"))?;
        stats.coordinate_request = stats.coordinate_request.max(bytes);
        stats.coordinate_capacity = stats.coordinate_capacity.max(coordinates.capacity() * std::mem::size_of::<CellRef>());
        coordinates.extend(sheet.cells.iter().filter_map(|(at, cell)| cell.formula.as_ref().map(|_| at)));
        stats.sheet_detach += 1;
        let sheet = Arc::make_mut(sheet);
        stats.store_calls += 1;
        let dropped = sheet.cells.rebuild();
        stats.drops += dropped;
        let after = sheet.cells.iter().count();
        stats.actual_after += after;
        if dropped != 0 || after != before {
            return Err(RecoveryError::Malformed("model_storage"));
        }
        for at in coordinates {
            let cell = sheet.cells.get_mut(at).ok_or(RecoveryError::Malformed("model_storage"))?;
            let old = cell.formula.as_ref().ok_or(RecoveryError::Malformed("model_storage"))?;
            stats.formulas += 1;
            stats.text_bytes = stats.text_bytes.checked_add(old.text.len()).ok_or(RecoveryError::TooLarge("formula_text"))?;
            let expr = gridcraft_formula::parse(&old.text).ok().map(Arc::new);
            stats.parsed += usize::from(expr.is_some());
            // Construct from semantic fields: never clone/walk the old expr or print/canonicalize text.
            cell.formula = Some(Arc::new(Formula { text: old.text.clone(), expr, array: old.array, dynamic: old.dynamic, eval_mode: old.eval_mode }));
        }
    }
    stats.style_entries = candidate.styles.len();
    stats.style_calls += 1;
    candidate.styles.rebuild_index();
    stats.model_after += 1;
    gridcraft_model::validate_recovery_model(&candidate).map_err(model_error)?;
    Ok(candidate)
}
pub(super) fn owned(candidate: Workbook, metadata: &Metadata, limits: &RecoveryLimits) -> Result<Workbook, RecoveryError> {
    run(candidate, metadata, limits, &mut Stats::default())
}

#[cfg(test)]
mod tests;

#[cfg(test)]
pub(super) fn measure(candidate: Workbook, metadata: &Metadata, limits: &RecoveryLimits) -> (Result<Workbook, RecoveryError>, serde_json::Value) {
    let mut stats = Stats::default();
    let result = run(candidate, metadata, limits, &mut stats);
    (result, serde_json::to_value(stats).unwrap())
}
