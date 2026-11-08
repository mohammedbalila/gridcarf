use super::{RecoveryError, RecoveryLimits, preflight::charge};
use gridcraft_core::CellRef;
use gridcraft_model::{CellPicture, Workbook};
use std::{mem::size_of, sync::Arc};

#[derive(Debug, Default)]
pub(super) struct Requests {
    pub(super) raw: usize,
    pub(super) encoded: usize,
    pub(super) largest_b64: usize,
    pub(super) occurrences: usize,
    pub(super) picture_vector: usize,
    pub(super) group_vector: usize,
    pub(super) identity_entries: usize,
    pub(super) identity_payload: usize,
    pub(super) serialization_payload: usize,
}
fn large() -> RecoveryError {
    RecoveryError::TooLarge("serializer_requests")
}
fn bytes(count: usize, width: usize) -> Result<usize, RecoveryError> {
    count.checked_mul(width).filter(|n| *n <= isize::MAX as usize).ok_or_else(large)
}
fn encoded(raw: usize) -> Result<usize, RecoveryError> {
    raw.checked_add(2).and_then(|n| (n / 3).checked_mul(4)).filter(|n| *n <= isize::MAX as usize).ok_or_else(large)
}
fn coordinate(cell: &CellRef) -> Result<(), RecoveryError> {
    if cell.row >= 1_048_576 || cell.col >= 16_384 {
        return Err(RecoveryError::Malformed("a1_key"));
    }
    Ok(())
}
impl Requests {
    fn binary(&mut self, raw: usize, limits: &RecoveryLimits) -> Result<usize, RecoveryError> {
        let size = encoded(raw)?;
        charge(&mut self.raw, raw, limits.envelope, "base64")?;
        charge(&mut self.encoded, size, limits.manifest, "base64")?;
        self.largest_b64 = self.largest_b64.max(size);
        Ok(size)
    }
}

pub(super) fn check(workbook: &Workbook, limits: &RecoveryLimits) -> Result<Requests, RecoveryError> {
    if workbook.sheets.len() > limits.sheets {
        return Err(RecoveryError::TooLarge("sheets"));
    }
    let mut requests = Requests::default();
    let mut spills = 0;
    // Check collection lengths before any entry scan or existing serializer can allocate.
    for sheet in &workbook.sheets {
        charge(&mut spills, sheet.spill.len(), limits.cells, "cells")?;
        for count in
            [sheet.images.len(), sheet.cell_pictures.len(), sheet.comments.len(), sheet.hyperlinks.len(), sheet.spill.len(), sheet.spill_ranges.len()]
        {
            charge(&mut requests.occurrences, count, limits.nodes, "nodes")?;
        }
        charge(&mut requests.identity_entries, sheet.cell_pictures.len(), limits.nodes, "nodes")?;
        requests.picture_vector = requests.picture_vector.max(bytes(sheet.cell_pictures.len(), size_of::<(&CellRef, &Arc<CellPicture>)>())?);
    }
    // Existing source projection emits at least14 nodes per picture occurrence including its group.
    if requests.identity_entries.checked_mul(14).and_then(|n| n.checked_add(2)).is_none_or(|n| n > limits.nodes) {
        return Err(RecoveryError::TooLarge("nodes"));
    }
    requests.group_vector = bytes(requests.identity_entries, size_of::<u32>())?;
    requests.identity_payload = bytes(requests.identity_entries, size_of::<(*const CellPicture, u32)>())?;
    if let Some(value) = &workbook.vba_project {
        requests.serialization_payload = requests.binary(value.len(), limits)?;
    }
    for sheet in &workbook.sheets {
        for key in sheet
            .comments
            .keys()
            .chain(sheet.hyperlinks.keys())
            .chain(sheet.spill.keys())
            .chain(sheet.spill_ranges.keys())
            .chain(sheet.cell_pictures.keys())
        {
            coordinate(key)?;
        }
        for image in &sheet.images {
            let size = requests.binary(image.data.len(), limits)?;
            requests.serialization_payload = requests.serialization_payload.max(size);
        }
        let refs = bytes(sheet.cell_pictures.len(), size_of::<(&CellRef, &Arc<CellPicture>)>())?;
        requests.serialization_payload = requests.serialization_payload.max(refs);
        for picture in sheet.cell_pictures.values() {
            let size = requests.binary(picture.data.len(), limits)?;
            requests.serialization_payload = requests.serialization_payload.max(refs.checked_add(size).ok_or_else(large)?);
        }
    }
    requests.serialization_payload = requests.serialization_payload.checked_add(requests.group_vector).ok_or_else(large)?;
    Ok(requests)
}

#[cfg(test)]
mod tests;
