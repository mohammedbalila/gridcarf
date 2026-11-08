use super::{CellRef, Metadata, NativePath, RecoveryError, RecoveryLimits, RecoveryProvenance, Selection, Stats};
#[cfg(test)]
use super::{RangeRef, RecoveryFileKind, Workbook, frame, prepare, write};
use gridcraft_core::{MAX_COLS, MAX_ROWS};
fn cell(cell: CellRef) -> bool {
    cell.row < MAX_ROWS && cell.col < MAX_COLS
}
fn selection(value: &Selection) -> Result<(), RecoveryError> {
    if !cell(value.active)
        || !cell(value.anchor)
        || value.ranges.is_empty()
        || value.ranges.iter().any(|r| !cell(r.start) || !cell(r.end) || r.start.row > r.end.row || r.start.col > r.end.col)
        || !value.ranges.last().is_some_and(|r| r.contains(value.active))
    {
        return Err(RecoveryError::Malformed("selection"));
    }
    Ok(())
}
pub(super) fn native(path: &NativePath, limits: &RecoveryLimits) -> Result<(), RecoveryError> {
    let bytes = match path {
        NativePath::Unix(v) => Some(v.len()),
        NativePath::Windows(v) => v.len().checked_mul(2),
    };
    if bytes.is_none_or(|n| n > limits.native_units) {
        return Err(RecoveryError::TooLarge("native_units"));
    }
    Ok(())
}
pub(super) fn eligible(value: &Metadata) -> Result<(), RecoveryError> {
    if value.provenance != RecoveryProvenance::Plain {
        return Err(RecoveryError::Ineligible(value.provenance));
    }
    if value.import_issues.iter().any(|i| i.effect == gridcraft_model::CompatibilityEffect::EncryptionRemoved) {
        return Err(RecoveryError::Ineligible(RecoveryProvenance::EncryptedOrigin));
    }
    Ok(())
}
pub(super) fn check(value: &Metadata, limits: &RecoveryLimits) -> Result<(), RecoveryError> {
    eligible(value)?;
    if let Some(path) = &value.original_path_hint {
        native(path, limits)?;
    }
    for text in std::iter::once(value.title.as_str()).chain(
        value.import_issues.iter().flat_map(|i| [&i.code, &i.feature, &i.description].into_iter().map(String::as_str).chain(i.location.as_deref())),
    ) {
        if text.len() > limits.metadata {
            return Err(RecoveryError::TooLarge("metadata"));
        }
    }
    selection(&value.selection)?;
    for entry in &value.sheet_selections {
        selection(entry)?;
    }
    Ok(())
}
pub(super) fn path(path: Option<NativePath>, limits: &RecoveryLimits, stats: &mut Stats) -> Result<Option<std::path::PathBuf>, RecoveryError> {
    let Some(path) = path else { return Ok(None) };
    native(&path, limits)?;
    #[cfg(unix)]
    if let NativePath::Unix(bytes) = path {
        use std::os::unix::ffi::OsStringExt;
        stats.native_conversions += 1;
        return Ok(Some(std::ffi::OsString::from_vec(bytes).into()));
    }
    #[cfg(windows)]
    if let NativePath::Windows(units) = path {
        use std::os::windows::ffi::OsStringExt;
        stats.native_conversions += 1;
        return Ok(Some(std::ffi::OsString::from_wide(&units).into()));
    }
    Err(RecoveryError::UnsupportedRepresentation("native_path_platform"))
}

#[cfg(test)]
mod tests;
