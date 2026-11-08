use super::{
    Arc, Part, Preserved, PreservedData, RecoveryError, RecoveryLimits, Relationship, Sidecars, Stats, Workbook, frame, metadata, pictures, reserve,
};
#[cfg(test)]
use super::{decode, decode_run, prepare};
use gridcraft_model::{PreservedPart, PreservedRel, PreservedSheet, PreservedXml};
fn relationships(values: Vec<Relationship<String>>, stats: &mut Stats) -> Result<Vec<PreservedRel>, RecoveryError> {
    let mut out = reserve(values.len(), stats)?;
    for v in values {
        out.push(PreservedRel { id: v.id, rel_type: v.rel_type, target: v.target, external: v.external });
    }
    Ok(out)
}
fn part(value: Part<String, Vec<Relationship<String>>>, plan: &frame::FramePlan<'_>, stats: &mut Stats) -> Result<PreservedPart, RecoveryError> {
    let rels = relationships(value.rels, stats)?;
    let bytes = plan.payload(value.data.frame as usize).ok_or(RecoveryError::Malformed("frame_reference"))?;
    // The complete borrowed plan has already checked every prefix, descriptor and EOF.
    stats.payload_copies += 1;
    stats.payload_bytes += bytes.len();
    let data = Arc::from(bytes);
    Ok(PreservedPart { name: value.name, content_type: value.content_type, data, rels })
}
fn preserved(value: Option<PreservedData>, plan: &frame::FramePlan<'_>, stats: &mut Stats) -> Result<Option<Arc<Preserved>>, RecoveryError> {
    let Some(value) = value else { return Ok(None) };
    let package_rels = relationships(value.package_rels, stats)?;
    let rels = relationships(value.rels, stats)?;
    let mut parts = reserve(value.parts.len(), stats)?;
    let mut elements = reserve(value.elements.len(), stats)?;
    let mut sheets = reserve(value.sheets.len(), stats)?;
    for p in value.parts {
        parts.push(part(p, plan, stats)?);
    }
    for e in value.elements {
        elements.push(PreservedXml { slot: e.slot, xml: e.xml });
    }
    for s in value.sheets {
        sheets.push(PreservedSheet {
            name: s.name,
            visibility: s.visibility,
            rel_id: s.rel_id,
            before: usize::try_from(s.before).map_err(|_| RecoveryError::TooLarge("sheet_position"))?,
            sheet_id: s.sheet_id,
        });
    }
    let vml = value.vml.map(|p| part(p, plan, stats)).transpose()?;
    Ok(Some(Arc::new(Preserved { package_rels, rels, parts, elements, vml, sheets })))
}
pub(super) fn check(workbook: &Workbook, sidecars: &Sidecars, limits: &RecoveryLimits) -> Result<(), RecoveryError> {
    if sidecars.sheets.len() != workbook.sheets.len() || sidecars.sheets.iter().enumerate().any(|(i, s)| s.ordinal as usize != i) {
        return Err(RecoveryError::Malformed("sheet_ordinals"));
    }
    for p in sidecars.workbook.source_dir.iter().chain(sidecars.workbook.source_path.iter()) {
        metadata::native(p, limits)?;
    }
    Ok(())
}
pub(super) fn attach(
    mut workbook: Workbook,
    sidecars: Sidecars,
    plan: &frame::FramePlan<'_>,
    limits: &RecoveryLimits,
    stats: &mut Stats,
) -> Result<Workbook, RecoveryError> {
    let source = sidecars.workbook;
    workbook.source_dir = metadata::path(source.source_dir, limits, stats)?;
    workbook.source_path = metadata::path(source.source_path, limits, stats)?;
    workbook.macros_unread = Arc::new(source.macros_unread);
    workbook.preserved = preserved(source.preserved, plan, stats)?;
    for (sheet, data) in workbook.sheets.iter_mut().zip(sidecars.sheets) {
        let sheet = Arc::make_mut(sheet);
        sheet.file_id = data.file_id;
        sheet.spill = data.spill;
        sheet.spill_ranges = data.spill_ranges;
        sheet.preserved = preserved(data.preserved, plan, stats)?;
    }
    pictures::restore(&mut workbook, &source.cell_picture_groups, limits, stats)?;
    Ok(workbook)
}

#[cfg(test)]
mod tests;
