//! Detached finishing in the descendant that can consume both private codec owners.
use super::{Arc, RecoveryError, ValidatedRecovery, Workbook};
#[cfg(test)]
use super::{CellRef, NativePath, Selection, Value};
use crate::selection_memory::{SelectionMemory, Transition};

fn scan_work(wb: &Workbook) -> Result<usize, RecoveryError> {
    // D3 validates CellStore directories and len twice. Every nonempty band/row is bounded
    // by its populated cells; zero len has no hidden directories. No raw AuditStats token.
    const WORK: usize = 16_777_216;
    let s = wb.sheets.len();
    let mut work = s;
    if work > WORK {
        return Err(RecoveryError::TooLarge("restore_work"));
    }
    let (mut cells, mut shapes) = (0usize, 0usize);
    for sheet in &wb.sheets {
        cells = cells.checked_add(sheet.cells.len()).ok_or(RecoveryError::TooLarge("restore_work"))?;
        shapes = shapes.checked_add(sheet.shapes.len()).ok_or(RecoveryError::TooLarge("restore_work"))?;
    }
    // Header inventory S above, then Calc/equation/refresh: 3S+3Cstore+2T.
    // Selection identity preparation adds <=10S visits and four fixed sort calls. It only
    // labels the existing prefix (fresh unique UIDs, Refresh), never copies selection ranges.
    for (count, multiple) in [(s, 3usize), (cells, 3), (shapes, 2), (s, 10), (1, 4)] {
        work = count.checked_mul(multiple).and_then(|n| work.checked_add(n)).filter(|n| *n <= WORK).ok_or(RecoveryError::TooLarge("restore_work"))?;
    }
    Ok(work)
}
pub(in crate::recovery) fn restore(validated: ValidatedRecovery) -> Result<crate::DocState, RecoveryError> {
    restore_owned(
        validated,
        #[cfg(test)]
        None,
    )
}
fn restore_owned(validated: ValidatedRecovery, #[cfg(test)] reservation_budget: Option<usize>) -> Result<crate::DocState, RecoveryError> {
    let ValidatedRecovery { mut workbook, mut metadata } = validated;
    let work = scan_work(&workbook)?;
    let calc = gridcraft_calc::Calc::from_recovery(&workbook, work).map_err(|e| match e {
        gridcraft_calc::RecoveryCalcError::Limit(reason) => RecoveryError::TooLarge(reason),
        gridcraft_calc::RecoveryCalcError::Allocation => RecoveryError::TooLarge("restore_allocation"),
    })?;
    // Whole candidate admission, BEFORE any refresh/layout. The shared cache answers sources
    // above 4096 bytes with SourceTooLong without copying, hashing, or calling math layout.
    for sheet in &workbook.sheets {
        for shape in &sheet.shapes {
            if shape.kind == gridcraft_model::ShapeKind::Equation && shape.text.len() <= 4096 {
                return Err(RecoveryError::UnsupportedRepresentation("restore_equation"));
            }
        }
    }
    let mut selection_memory = SelectionMemory::default();
    #[cfg(not(test))]
    let plan = selection_memory
        .prepare(&metadata.sheet_selections, &workbook, &workbook, &metadata.selection, Transition::Refresh)
        .map_err(|_| RecoveryError::TooLarge("restore_selection"))?;
    #[cfg(test)]
    let plan = match reservation_budget {
        Some(budget) => {
            selection_memory.prepare_with_budget(&metadata.sheet_selections, &workbook, &workbook, &metadata.selection, Transition::Refresh, budget)
        }
        None => selection_memory.prepare(&metadata.sheet_selections, &workbook, &workbook, &metadata.selection, Transition::Refresh),
    }
    .map_err(|_| RecoveryError::TooLarge("restore_selection"))?;
    plan.apply(&mut metadata.sheet_selections, &mut selection_memory, &mut metadata.selection);
    // For admitted shapes none is stale. Preserve the common no-layout cache semantics.
    workbook.refresh_equations();
    let context = crate::recovery::RestoreContext {
        original_path_hint: metadata.original_path_hint,
        id: metadata.id,
        captured_revision: metadata.captured_revision,
        sequence: metadata.sequence,
        dirty_at_capture: metadata.dirty_at_capture,
        provenance: metadata.provenance,
    };
    // Arc/saved identity is created only after fallible selection preparation and graph admission.
    let mut doc = crate::DocState::finish(workbook, calc, None, metadata.title, crate::Trust::FromFile, false);
    doc.selection = metadata.selection;
    doc.sheet_selections = metadata.sheet_selections;
    doc.selection_memory = selection_memory;
    doc.import_issues = metadata.import_issues;
    doc.content_kind = metadata.file_kind.map(Into::into);
    doc.recovery_context = Some(Arc::new(context));
    doc.recovery_needs_save = true;
    Ok(doc)
}
#[cfg(test)]
mod tests;
