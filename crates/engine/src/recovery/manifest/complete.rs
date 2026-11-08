//! Complete private codec gates. Runtime provenance and installation remain separate.
use super::{
    CellRef, Manifest, Metadata, NativePath, Part, Preserved, PreservedData, RecoveryError, RecoveryId, RecoveryLimits, RecoveryProvenance,
    Relationship, Selection, Sidecars, Workbook, check_executable, descriptors, frame, preflight, rebuild, rebuild_owned, source_frames, write,
};
#[cfg(test)]
use super::{RangeRef, RecoveryFileKind, Value, observe, observe_manifest, picture_groups};
use crate::recovery::{PreparedRecovery, RecoveryCapture, ValidatedRecovery};
use std::sync::Arc;
mod metadata;
mod pictures;
mod sidecars;
#[derive(Debug, Default)]
#[cfg_attr(test, derive(serde::Serialize))]
struct Stats {
    payload_copies: usize,
    payload_bytes: usize,
    native_conversions: usize,
    vector_request: usize,
    vector_capacity: usize,
    picture_groups: usize,
    picture_assignments: usize,
    d2_decode: usize,
    p3_calls: usize,
    derived_calls: usize,
    final_compare: usize,
}
fn reserve<T>(count: usize, stats: &mut Stats) -> Result<Vec<T>, RecoveryError> {
    let bytes =
        count.checked_mul(std::mem::size_of::<T>()).filter(|n| *n <= isize::MAX as usize).ok_or(RecoveryError::TooLarge("materialization"))?;
    let mut out = Vec::new();
    out.try_reserve_exact(count).map_err(|_| RecoveryError::TooLarge("materialization"))?;
    stats.vector_request = stats.vector_request.max(bytes);
    stats.vector_capacity = stats.vector_capacity.max(out.capacity() * std::mem::size_of::<T>());
    Ok(out)
}
fn graph_error(error: gridcraft_xlsx::RecoveryPreservedError) -> RecoveryError {
    use gridcraft_xlsx::RecoveryPreservedError::*;
    match error {
        UnsupportedXml => RecoveryError::UnsupportedRepresentation("preserved_xml"),
        UnsupportedNative => RecoveryError::UnsupportedRepresentation("preserved_native"),
        UnsupportedPartName => RecoveryError::UnsupportedRepresentation("preserved_part_name"),
        XmlLimit => RecoveryError::TooLarge("preserved_xml"),
        VmlLimit => RecoveryError::TooLarge("preserved_vml"),
        WorkLimit => RecoveryError::TooLarge("preserved_work"),
        TextLimit => RecoveryError::TooLarge("preserved_text"),
        Allocation => RecoveryError::TooLarge("preserved_allocation"),
        ChainLimit => RecoveryError::TooLarge("preserved_chain"),
        PartLimit => RecoveryError::TooLarge("preserved_parts"),
        RelationshipLimit => RecoveryError::TooLarge("preserved_relationships"),
        ByteLimit => RecoveryError::TooLarge("preserved_bytes"),
        SheetLimit => RecoveryError::TooLarge("preserved_sheets"),
        Slot => RecoveryError::Malformed("preserved_slot"),
        Xml => RecoveryError::Malformed("preserved_xml"),
        Vml => RecoveryError::Malformed("preserved_vml"),
        ChartSheet => RecoveryError::Malformed("preserved_chartsheet"),
        OutputName => RecoveryError::Malformed("preserved_output_name"),
        PartName => RecoveryError::Malformed("preserved_part_name"),
        DuplicatePart => RecoveryError::Malformed("preserved_duplicate_part"),
        DuplicateRelationship => RecoveryError::Malformed("preserved_duplicate_relationship"),
        Relationship => RecoveryError::Malformed("preserved_relationship"),
        Owner => RecoveryError::Malformed("preserved_owner"),
        MissingTarget => RecoveryError::Malformed("preserved_missing_target"),
        UnreachablePart => RecoveryError::Malformed("preserved_unreachable_part"),
    }
}
fn graph(workbook: &Workbook, stats: &mut Stats) -> Result<(), RecoveryError> {
    stats.p3_calls += 1;
    gridcraft_xlsx::validate_recovery_preserved(workbook).map_err(graph_error)
}
pub(in crate::recovery) fn prepare<'a>(capture: &RecoveryCapture<'a>, limits: &RecoveryLimits) -> Result<PreparedRecovery<'a>, RecoveryError> {
    metadata::eligible(capture.metadata)?;
    check_executable(capture.workbook, capture.metadata, limits, false)?;
    metadata::check(capture.metadata, limits)?;
    gridcraft_model::validate_recovery_model(capture.workbook).map_err(rebuild::model_error)?;
    graph(capture.workbook, &mut Stats::default())?;
    let manifest = write::manifest_bytes(capture.workbook, capture.metadata, limits)?.bytes;
    let frames = frame::WriteParts::new(manifest.len(), source_frames(capture.workbook, limits)?, limits)?;
    Ok(PreparedRecovery { manifest, frames })
}
fn decode_run(bytes: &[u8], limits: &RecoveryLimits, stats: &mut Stats) -> Result<ValidatedRecovery, RecoveryError> {
    let envelope = frame::Envelope::parse(bytes, limits)?;
    preflight::preflight_manifest(envelope.manifest, limits)?;
    let Manifest { workbook, metadata, sidecars } =
        serde_json::from_slice(envelope.manifest).map_err(|_| RecoveryError::Malformed("manifest_schema"))?;
    metadata::check(&metadata, limits)?;
    sidecars::check(&workbook, &sidecars, limits)?;
    let plan = envelope.plan(descriptors(&sidecars, limits)?, limits)?;
    let mut workbook = sidecars::attach(workbook, sidecars, &plan, limits, stats)?;
    stats.d2_decode += 1;
    let staged = check_executable(&workbook, &metadata, limits, true)?;
    if let Some(macros) = staged.adopted {
        workbook.macros = macros.into();
        workbook.macros_read.clear();
    }
    // Only this owned staging candidate is mutated. Nothing escapes before the fixed point.
    gridcraft_model::validate_recovery_model(&workbook).map_err(rebuild::model_error)?;
    graph(&workbook, stats)?;
    stats.derived_calls += 1;
    let workbook = rebuild_owned(workbook, &metadata, limits)?;
    stats.final_compare += 1;
    write::compare_manifest(&workbook, &metadata, envelope.manifest, limits)?;
    plan.compare_payloads(source_frames(&workbook, limits)?)?;
    Ok(ValidatedRecovery { workbook, metadata })
}
pub(in crate::recovery) fn decode(bytes: &[u8], limits: &RecoveryLimits) -> Result<ValidatedRecovery, RecoveryError> {
    decode_run(bytes, limits, &mut Stats::default())
}
impl ValidatedRecovery {
    /// Recorded inert snapshot identifier.
    pub fn id(&self) -> RecoveryId {
        self.metadata.id
    }
    /// Recorded snapshot sequence; not a live session ordering authority.
    pub fn sequence(&self) -> u64 {
        self.metadata.sequence
    }
    /// Recorded revision; not proof of the current workbook revision.
    pub fn captured_revision(&self) -> u64 {
        self.metadata.captured_revision
    }
}
#[cfg(test)]
mod tests;

#[cfg(test)]
pub(super) fn measure(bytes: &[u8], limits: &RecoveryLimits) -> (Result<ValidatedRecovery, RecoveryError>, serde_json::Value) {
    let mut stats = Stats::default();
    let result = decode_run(bytes, limits, &mut stats);
    (result, serde_json::to_value(stats).unwrap())
}
