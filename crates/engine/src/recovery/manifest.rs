//! The private v1 projection, not an acceptance or raw-input validation entry point.

mod capture;
pub(super) mod complete;
mod context;
mod executable;
pub(super) mod frame;
mod frozen;
mod observe;
mod preflight;
mod rebuild;
mod write;
use super::{NativePath, RecoveryError, RecoveryFileKind, RecoveryId, RecoveryLimits, RecoveryProvenance};
use crate::Selection;
use gridcraft_core::{CellRef, RangeRef, Value};
use gridcraft_model::{CompatibilityIssue, Preserved, PreservedPart, PreservedRel, PreservedSheet, PreservedXml, Visibility, Workbook};
use serde::{
    Deserialize, Deserializer, Serialize, Serializer,
    ser::{SerializeSeq, SerializeStruct},
};
use std::{
    collections::{BTreeMap, HashMap},
    path::Path,
};

#[cfg(test)]
thread_local! { pub(in crate::recovery) static CAPTURE_WORK: std::cell::Cell<(usize,usize)> = const { std::cell::Cell::new((0,0)) }; }

// deserialize_with makes a missing field an error even though explicit null is supported.
fn required<'de, D: Deserializer<'de>, T: Deserialize<'de>>(deserializer: D) -> Result<Option<T>, D::Error> {
    Option::deserialize(deserializer)
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Metadata {
    #[serde(skip)]
    frozen_active_sheet: Option<usize>,
    title: String,
    #[serde(deserialize_with = "required")]
    original_path_hint: Option<NativePath>,
    #[serde(deserialize_with = "required")]
    file_kind: Option<RecoveryFileKind>,
    dirty_at_capture: bool,
    import_issues: Vec<CompatibilityIssue>,
    selection: Selection,
    sheet_selections: Vec<Selection>,
    captured_revision: u64,
    id: RecoveryId,
    sequence: u64,
    provenance: RecoveryProvenance,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    workbook: Workbook,
    metadata: Metadata,
    sidecars: Sidecars,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Sidecars {
    workbook: WorkbookSidecar,
    sheets: Vec<SheetSidecar>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct WorkbookSidecar {
    #[serde(deserialize_with = "required")]
    source_dir: Option<NativePath>,
    #[serde(deserialize_with = "required")]
    source_path: Option<NativePath>,
    macros_unread: Vec<serde_json::Value>,
    cell_picture_groups: Vec<u32>,
    #[serde(deserialize_with = "required")]
    preserved: Option<PreservedData>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SheetSidecar {
    ordinal: u32,
    file_id: u32,
    #[serde(with = "a1_map")]
    spill: BTreeMap<CellRef, Value>,
    #[serde(with = "a1_map")]
    spill_ranges: BTreeMap<CellRef, RangeRef>,
    #[serde(deserialize_with = "required")]
    preserved: Option<PreservedData>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PreservedData {
    package_rels: Vec<Relationship<String>>,
    rels: Vec<Relationship<String>>,
    parts: Vec<Part<String, Vec<Relationship<String>>>>,
    elements: Vec<Fragment<String>>,
    #[serde(deserialize_with = "required")]
    vml: Option<Part<String, Vec<Relationship<String>>>>,
    sheets: Vec<KeptSheet<String>>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Relationship<T> {
    id: T,
    rel_type: T,
    target: T,
    external: bool,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Part<T, R> {
    name: T,
    content_type: T,
    data: FrameRef,
    rels: R,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct FrameRef {
    frame: u32,
    length: u64,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Fragment<T> {
    slot: T,
    xml: T,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct KeptSheet<T> {
    name: T,
    visibility: Visibility,
    rel_id: T,
    before: u64,
    sheet_id: u32,
}

#[derive(Serialize)]
struct ManifestRef<'a> {
    workbook: frozen::FrozenWorkbook<'a>,
    metadata: &'a Metadata,
    sidecars: SidecarsRef<'a>,
}

impl<'a> ManifestRef<'a> {
    fn new(workbook: &'a Workbook, metadata: &'a Metadata, limits: &RecoveryLimits) -> Result<Self, RecoveryError> {
        observe::precheck(workbook, limits)?;
        if workbook.sheets.len() > limits.sheets {
            return Err(RecoveryError::TooLarge("sheets"));
        }
        for path in workbook.source_dir.iter().chain(workbook.source_path.iter()) {
            if native_bytes(path)? > limits.native_units {
                return Err(RecoveryError::TooLarge("native_units"));
            }
        }
        if let Some(path) = &metadata.original_path_hint {
            let bytes = match path {
                NativePath::Unix(v) => Some(v.len()),
                NativePath::Windows(v) => v.len().checked_mul(2),
            };
            if bytes.is_none_or(|n| n > limits.native_units) {
                return Err(RecoveryError::TooLarge("native_units"));
            }
        }
        let mut frames = 0usize;
        for preserved in workbook.preserved.iter().chain(workbook.sheets.iter().filter_map(|s| s.preserved.as_ref())) {
            frames = frames.checked_add(frame_count(preserved)).ok_or(RecoveryError::TooLarge("parts"))?;
            if frames > limits.parts || u32::try_from(frames).is_err() {
                return Err(RecoveryError::TooLarge("parts"));
            }
        }
        let groups = picture_groups(workbook, limits)?;
        Ok(Self {
            workbook: frozen::FrozenWorkbook { workbook, active_sheet: metadata.frozen_active_sheet.unwrap_or_else(|| workbook.active_sheet()) },
            metadata,
            sidecars: SidecarsRef { workbook, groups },
        })
    }
}

fn observe_manifest(
    workbook: &Workbook,
    metadata: &Metadata,
    limits: &RecoveryLimits,
    pending_decode: bool,
) -> Result<preflight::AuditStats, RecoveryError> {
    let pending = if workbook.macros_read.is_empty() {
        None
    } else {
        if !pending_decode || !workbook.macros.is_empty() {
            return Err(RecoveryError::UnsupportedRepresentation("pending_macros"));
        }
        Some(workbook.macros_read.as_slice())
    };
    let projection = ManifestRef::new(workbook, metadata, limits)?;
    observe::with_pending(&projection, limits, pending)
}

fn check_executable(workbook: &Workbook, metadata: &Metadata, limits: &RecoveryLimits, decode: bool) -> Result<executable::Staged, RecoveryError> {
    executable::check(workbook, metadata, limits, decode)
}

fn rebuild_owned(candidate: Workbook, metadata: &Metadata, limits: &RecoveryLimits) -> Result<Workbook, RecoveryError> {
    rebuild::owned(candidate, metadata, limits)
}

fn picture_groups(workbook: &Workbook, limits: &RecoveryLimits) -> Result<Vec<u32>, RecoveryError> {
    let count =
        workbook.sheets.iter().try_fold(0usize, |n, sheet| n.checked_add(sheet.cell_pictures.len())).ok_or(RecoveryError::TooLarge("nodes"))?;
    // Necessary node lower bound: each picture pair costs 13, each group ID one, plus key/array.
    if count.checked_mul(14).and_then(|n| n.checked_add(2)).is_none_or(|n| n > limits.nodes) {
        return Err(RecoveryError::TooLarge("nodes"));
    }
    let mut groups = Vec::new();
    groups.try_reserve_exact(count).map_err(|_| RecoveryError::TooLarge("picture_groups_allocation"))?;
    let mut identities = HashMap::new();
    identities.try_reserve(count).map_err(|_| RecoveryError::TooLarge("picture_groups_allocation"))?;
    for picture in workbook.sheets.iter().flat_map(|sheet| sheet.cell_pictures.values()) {
        let next = u32::try_from(identities.len()).map_err(|_| RecoveryError::TooLarge("picture_groups"))?;
        let group = identities.entry(std::sync::Arc::as_ptr(picture)).or_insert(next);
        groups.push(*group);
    }
    Ok(groups)
}

fn frame_count(preserved: &Preserved) -> usize {
    preserved.parts.len().saturating_add(usize::from(preserved.vml.is_some()))
}

fn source_frames<'a>(workbook: &'a Workbook, limits: &RecoveryLimits) -> Result<impl Iterator<Item = &'a [u8]> + Clone, RecoveryError> {
    if workbook.sheets.len() > limits.sheets {
        return Err(RecoveryError::TooLarge("sheets"));
    }
    Ok(workbook
        .preserved
        .iter()
        .map(|p| p.as_ref())
        .chain(workbook.sheets.iter().filter_map(|s| s.preserved.as_deref()))
        .flat_map(|p| p.parts.iter().chain(p.vml.iter()).map(|part| part.data.as_ref())))
}
fn descriptors<'a>(sidecars: &'a Sidecars, limits: &RecoveryLimits) -> Result<impl Iterator<Item = &'a FrameRef> + Clone, RecoveryError> {
    if sidecars.sheets.len() > limits.sheets {
        return Err(RecoveryError::TooLarge("sheets"));
    }
    Ok(sidecars
        .workbook
        .preserved
        .iter()
        .chain(sidecars.sheets.iter().filter_map(|s| s.preserved.as_ref()))
        .flat_map(|p| p.parts.iter().chain(p.vml.iter()).map(|part| &part.data)))
}

struct Sequence<I>(I);
impl<I: Iterator + Clone> Serialize for Sequence<I>
where
    I::Item: Serialize,
{
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_seq(self.0.clone())
    }
}
fn relationship(value: &PreservedRel) -> Relationship<&str> {
    Relationship { id: &value.id, rel_type: &value.rel_type, target: &value.target, external: value.external }
}
fn fragment(value: &PreservedXml) -> Fragment<&str> {
    Fragment { slot: &value.slot, xml: &value.xml }
}
fn kept_sheet(value: &PreservedSheet) -> KeptSheet<&str> {
    KeptSheet { name: &value.name, visibility: value.visibility, rel_id: &value.rel_id, before: value.before as u64, sheet_id: value.sheet_id }
}
fn part(value: &PreservedPart, frame: u32) -> Part<&str, Sequence<impl Iterator<Item = Relationship<&str>> + Clone>> {
    Part {
        name: &value.name,
        content_type: &value.content_type,
        data: FrameRef { frame, length: value.data.len() as u64 },
        rels: Sequence(value.rels.iter().map(relationship)),
    }
}

struct PreservedRef<'a> {
    value: &'a Preserved,
    first: u32,
}
impl Serialize for PreservedRef<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut output = serializer.serialize_struct("Preserved", 6)?;
        output.serialize_field("package_rels", &Sequence(self.value.package_rels.iter().map(relationship)))?;
        output.serialize_field("rels", &Sequence(self.value.rels.iter().map(relationship)))?;
        output.serialize_field("parts", &Sequence(self.value.parts.iter().enumerate().map(|(i, p)| part(p, self.first + i as u32))))?;
        output.serialize_field("elements", &Sequence(self.value.elements.iter().map(fragment)))?;
        output.serialize_field("vml", &self.value.vml.as_ref().map(|p| part(p, self.first + self.value.parts.len() as u32)))?;
        output.serialize_field("sheets", &Sequence(self.value.sheets.iter().map(kept_sheet)))?;
        output.end()
    }
}

struct SidecarsRef<'a> {
    workbook: &'a Workbook,
    groups: Vec<u32>,
}
impl Serialize for SidecarsRef<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut output = serializer.serialize_struct("Sidecars", 2)?;
        output.serialize_field("workbook", &WorkbookSidecarRef { workbook: self.workbook, groups: &self.groups })?;
        output.serialize_field("sheets", &SheetsRef(self.workbook))?;
        output.end()
    }
}
struct WorkbookSidecarRef<'a> {
    workbook: &'a Workbook,
    groups: &'a [u32],
}
impl Serialize for WorkbookSidecarRef<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut output = serializer.serialize_struct("WorkbookSidecar", 5)?;
        output.serialize_field("source_dir", &self.workbook.source_dir.as_deref().map(NativePathRef))?;
        output.serialize_field("source_path", &self.workbook.source_path.as_deref().map(NativePathRef))?;
        output.serialize_field("macros_unread", self.workbook.macros_unread.as_ref())?;
        output.serialize_field("cell_picture_groups", self.groups)?;
        output.serialize_field("preserved", &self.workbook.preserved.as_deref().map(|value| PreservedRef { value, first: 0 }))?;
        output.end()
    }
}
struct SheetsRef<'a>(&'a Workbook);
impl Serialize for SheetsRef<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut output = serializer.serialize_seq(Some(self.0.sheets.len()))?;
        let mut first = self.0.preserved.as_deref().map_or(0, frame_count) as u32;
        for (ordinal, sheet) in self.0.sheets.iter().enumerate() {
            #[derive(Serialize)]
            struct SheetRef<'a> {
                ordinal: u32,
                file_id: u32,
                #[serde(with = "a1_map")]
                spill: &'a BTreeMap<CellRef, Value>,
                #[serde(with = "a1_map")]
                spill_ranges: &'a BTreeMap<CellRef, RangeRef>,
                preserved: Option<PreservedRef<'a>>,
            }
            output.serialize_element(&SheetRef {
                ordinal: ordinal as u32,
                file_id: sheet.file_id,
                spill: &sheet.spill,
                spill_ranges: &sheet.spill_ranges,
                preserved: sheet.preserved.as_deref().map(|value| PreservedRef { value, first }),
            })?;
            first += sheet.preserved.as_deref().map_or(0, frame_count) as u32;
        }
        output.end()
    }
}

struct NativePathRef<'a>(&'a Path);
fn native_bytes(path: &Path) -> Result<usize, RecoveryError> {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        Ok(path.as_os_str().as_bytes().len())
    }
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        path.as_os_str().encode_wide().count().checked_mul(2).ok_or(RecoveryError::TooLarge("native_units"))
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = path;
        Err(RecoveryError::UnsupportedRepresentation("native_path_platform"))
    }
}
impl Serialize for NativePathRef<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStrExt;
            serializer.serialize_newtype_variant("NativePath", 0, "unix", self.0.as_os_str().as_bytes())
        }
        #[cfg(windows)]
        {
            use std::os::windows::ffi::OsStrExt;
            serializer.serialize_newtype_variant("NativePath", 1, "windows", &Sequence(self.0.as_os_str().encode_wide()))
        }
        #[cfg(not(any(unix, windows)))]
        {
            Err(serde::ser::Error::custom("native_path_platform"))
        }
    }
}

// These adapters mirror the model's plain A1 map shape without widening its private API.
mod a1_map {
    use super::{BTreeMap, CellRef, Deserialize, Deserializer, Serialize, Serializer};
    pub(super) fn serialize<S: Serializer, V: Serialize>(map: &BTreeMap<CellRef, V>, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_map(map.iter().map(|(key, value)| (key.a1(), value)))
    }
    pub(super) fn deserialize<'de, D: Deserializer<'de>, V: Deserialize<'de>>(deserializer: D) -> Result<BTreeMap<CellRef, V>, D::Error> {
        let raw = BTreeMap::<String, V>::deserialize(deserializer)?;
        raw.into_iter()
            .map(|(key, value)| CellRef::parse(&key).map(|cell| (cell, value)).ok_or_else(|| serde::de::Error::custom("cell_reference")))
            .collect()
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod measurement;
