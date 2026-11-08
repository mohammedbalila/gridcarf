use super::{Arc, RecoveryError, RecoveryLimits, Stats, Workbook, reserve};
#[cfg(test)]
use super::{CellRef, Preserved, Value, decode_run, frame};
use gridcraft_model::CellPicture;
fn malformed() -> RecoveryError {
    RecoveryError::Malformed("picture_groups")
}
pub(super) fn restore(workbook: &mut Workbook, groups: &[u32], limits: &RecoveryLimits, stats: &mut Stats) -> Result<(), RecoveryError> {
    let count = workbook.sheets.iter().try_fold(0usize, |n, s| n.checked_add(s.cell_pictures.len())).ok_or_else(malformed)?;
    if groups.len() != count {
        return Err(malformed());
    }
    if count.checked_mul(14).and_then(|n| n.checked_add(2)).is_none_or(|n| n > limits.nodes) {
        return Err(RecoveryError::TooLarge("nodes"));
    }
    let mut first: Vec<Arc<CellPicture>> = reserve(count, stats)?;
    // Validate every occurrence before reassigning even the first repeated Arc.
    for (picture, group) in workbook.sheets.iter().flat_map(|s| s.cell_pictures.values()).zip(groups) {
        let group = usize::try_from(*group).map_err(|_| malformed())?;
        if group == first.len() {
            first.push(picture.clone());
        } else if first.get(group).is_none_or(|p| p.as_ref() != picture.as_ref()) {
            return Err(malformed());
        }
    }
    stats.picture_groups = first.len();
    for (picture, group) in workbook.sheets.iter_mut().flat_map(|s| Arc::make_mut(s).cell_pictures.values_mut()).zip(groups) {
        *picture = first.get(*group as usize).ok_or_else(malformed)?.clone();
        stats.picture_assignments += 1;
    }
    Ok(())
}
#[cfg(test)]
mod tests;
