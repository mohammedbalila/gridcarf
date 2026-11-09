use super::*;
use crate::Selection;
use crate::recovery::*;
use gridcraft_core::{CellRef, RangeRef};
use gridcraft_model::{CellPicture, Preserved, PreservedPart, Sheet, Workbook};
use std::sync::Arc;

fn metadata() -> Metadata {
    Metadata {
        frozen_active_sheet: None,
        title: "Untitled".into(),
        original_path_hint: None,
        file_kind: None,
        dirty_at_capture: true,
        import_issues: vec![],
        selection: Selection::default(),
        sheet_selections: vec![],
        captured_revision: 0,
        id: "000102030405060708090a0b0c0d0e0f".parse().unwrap(),
        sequence: 0,
        provenance: RecoveryProvenance::Plain,
    }
}

const EMPTY_WORKBOOK_SIDECAR: &str = r#"{"source_dir":null,"source_path":null,"macros_unread":[],"cell_picture_groups":[],"preserved":null}"#;
const EMPTY_SHEET_SIDECAR: &str = r#"{"ordinal":0,"file_id":0,"spill":{},"spill_ranges":{},"preserved":null}"#;

#[test]
fn projection_fences_binary_before_allocating_groups_or_serializing() {
    let mut workbook = Workbook::new();
    workbook.vba_project = Some(Arc::new(vec![1; 4]));
    let metadata = metadata();
    let limits = RecoveryLimits { manifest: 7, ..RecoveryLimits::default() };
    assert!(matches!(ManifestRef::new(&workbook, &metadata, &limits), Err(RecoveryError::TooLarge("base64"))));
}

#[test]
fn actual_projection_observer_matches_raw_counts_and_pending_parent_bridge() {
    let mut workbook = Workbook::new();
    let metadata = metadata();
    let limits = RecoveryLimits::default();
    workbook.macros_unread = Arc::new(vec![serde_json::json!({"v":3,"future":{"data":"?"}})]);
    let projection = ManifestRef::new(&workbook, &metadata, &limits).unwrap();
    let wire = serde_json::to_vec(&projection).unwrap();
    let mut raw = preflight::lexical_audit(&wire, &limits).unwrap();
    raw.number_bytes = 0;
    raw.max_number = 0;
    assert_eq!(observe_manifest(&workbook, &metadata, &limits, false).unwrap(), raw);
    workbook.macros_read = vec![serde_json::json!({"name":"pending","steps":[{"type":"stop"}]})];
    assert!(matches!(observe_manifest(&workbook, &metadata, &limits, false), Err(RecoveryError::UnsupportedRepresentation("pending_macros"))));
    let pending = observe_manifest(&workbook, &metadata, &limits, true).unwrap();
    let mut expected: serde_json::Value = serde_json::from_slice(&wire).unwrap();
    expected["workbook"]["macros"] = serde_json::json!(workbook.macros_read);
    let bytes = serde_json::to_vec(&expected).unwrap();
    let mut counted = preflight::lexical_audit(&bytes, &limits).unwrap();
    counted.number_bytes = 0;
    counted.max_number = 0;
    preflight::preflight_manifest(&bytes, &limits).unwrap();
    assert_eq!(pending, counted);
    assert_eq!(pending.sheets, 1);
    let typed: gridcraft_model::Macro = serde_json::from_value(serde_json::json!({"v":1,"name":"typed","steps":[]})).unwrap();
    workbook.macros = vec![typed].into();
    assert!(matches!(observe_manifest(&workbook, &metadata, &limits, true), Err(RecoveryError::UnsupportedRepresentation("pending_macros"))));
    workbook.macros_read.clear();
    assert!(observe_manifest(&workbook, &metadata, &limits, false).is_ok());
}

#[test]
fn actual_sidecar_wrapper_offsets_admit_depth64_and_refuse65() {
    let mut workbook = Workbook::new();
    let metadata = metadata();
    let limits = RecoveryLimits::default();
    let mut value = serde_json::Value::Null;
    for _ in 0..60 {
        value = serde_json::Value::Array(vec![value]);
    }
    workbook.macros_unread = Arc::new(vec![value]);
    let observed = observe_manifest(&workbook, &metadata, &limits, false).unwrap();
    assert_eq!(observed.max_depth, 64);
    let raw = serde_json::to_vec(&ManifestRef::new(&workbook, &metadata, &limits).unwrap()).unwrap();
    assert_eq!(preflight::lexical_audit(&raw, &limits).unwrap().max_depth, 64);
    let value = std::mem::replace(&mut Arc::make_mut(&mut workbook.macros_unread)[0], serde_json::Value::Null);
    Arc::make_mut(&mut workbook.macros_unread)[0] = serde_json::Value::Array(vec![value]);
    assert!(matches!(observe_manifest(&workbook, &metadata, &limits, false), Err(RecoveryError::TooLarge("depth"))));
}

#[test]
fn actual_projection_observes_repeated_picture_arcs_and_native_sidecars_once() {
    let mut workbook = Workbook::new();
    let metadata = metadata();
    let limits = RecoveryLimits::default();
    let picture = Arc::new(CellPicture { data: vec![1; 2], mime: "x".into(), alt: "é".into() });
    let sheet = Arc::make_mut(&mut workbook.sheets[0]);
    sheet.cell_pictures.insert(CellRef::new(0, 0), picture.clone());
    sheet.cell_pictures.insert(CellRef::new(0, 1), picture);
    sheet.spill.insert(CellRef::new(0, 2), Value::Empty);
    workbook.source_path = Some("é.xlsx".into());
    let raw = serde_json::to_vec(&ManifestRef::new(&workbook, &metadata, &limits).unwrap()).unwrap();
    preflight::preflight_manifest(&raw, &limits).unwrap();
    let mut counted = preflight::lexical_audit(&raw, &limits).unwrap();
    counted.number_bytes = 0;
    counted.max_number = 0;
    let observed = observe_manifest(&workbook, &metadata, &limits, false).unwrap();
    assert_eq!(observed, counted);
    assert_eq!((observed.groups, observed.cells), (2, 1));
    assert!(observed.native_bytes > 0);
    use std::io::Write;
    writeln!(std::io::stdout(), "D1_PROJECTION_CALLBACKS {:?} writer_bytes={}", observed, raw.len()).unwrap();
}

#[test]
fn added_wrappers_require_nullable_and_empty_fields() {
    let workbook: WorkbookSidecar = serde_json::from_str(EMPTY_WORKBOOK_SIDECAR).unwrap();
    assert_eq!(serde_json::to_string(&workbook).unwrap(), EMPTY_WORKBOOK_SIDECAR);
    for field in ["source_dir", "source_path", "macros_unread", "cell_picture_groups", "preserved"] {
        let mut wire: serde_json::Value = serde_json::from_str(EMPTY_WORKBOOK_SIDECAR).unwrap();
        wire.as_object_mut().unwrap().remove(field);
        assert!(serde_json::from_value::<WorkbookSidecar>(wire).is_err(), "missing {field}");
    }
    let sheet: SheetSidecar = serde_json::from_str(EMPTY_SHEET_SIDECAR).unwrap();
    assert_eq!(serde_json::to_string(&sheet).unwrap(), EMPTY_SHEET_SIDECAR);
    for field in ["ordinal", "file_id", "spill", "spill_ranges", "preserved"] {
        let mut wire: serde_json::Value = serde_json::from_str(EMPTY_SHEET_SIDECAR).unwrap();
        wire.as_object_mut().unwrap().remove(field);
        assert!(serde_json::from_value::<SheetSidecar>(wire).is_err(), "missing {field}");
    }
    for groups in ["null", "[-1]", "[4294967296]", "[1.0]"] {
        let wire = EMPTY_WORKBOOK_SIDECAR.replace("\"cell_picture_groups\":[]", &format!("\"cell_picture_groups\":{groups}"));
        assert!(serde_json::from_str::<WorkbookSidecar>(&wire).is_err(), "{groups}");
    }
}

#[test]
fn wrapper_unknown_duplicate_and_missing_fields_are_refused() {
    let cases = [
        (r#"{"frame":0,"length":0,"extra":0}"#, false),
        (r#"{"frame":0,"frame":1,"length":0}"#, false),
        (r#"{"frame":0}"#, false),
        (r#"{"frame":0,"length":0}"#, true),
    ];
    for (wire, accepted) in cases {
        assert_eq!(serde_json::from_str::<FrameRef>(wire).is_ok(), accepted);
    }
    let extra = EMPTY_WORKBOOK_SIDECAR.replacen('{', "{\"extra\":0,", 1);
    assert!(serde_json::from_str::<WorkbookSidecar>(&extra).is_err());
    assert!(serde_json::from_str::<Sidecars>(r#"{"workbook":null,"sheets":[]}"#).is_err());
    let minimal = r#"{"package_rels":[],"rels":[],"parts":[],"elements":[],"vml":null,"sheets":[]}"#;
    let preserved: PreservedData = serde_json::from_str(minimal).unwrap();
    assert_eq!(serde_json::to_string(&preserved).unwrap(), minimal);
    assert!(serde_json::from_str::<PreservedData>(&minimal.replace("\"vml\":null,", "")).is_err());
    assert!(serde_json::from_str::<PreservedData>(&minimal.replacen('{', "{\"extra\":0,", 1)).is_err());
}

#[test]
fn metadata_preserves_complete_selection_memory_and_required_nulls() {
    let mut data = metadata();
    let mut tail = Selection::at(CellRef::new(8, 2));
    tail.anchor = CellRef::new(0, 0);
    tail.ranges.push(RangeRef::cell(CellRef::new(8, 2)));
    for values in [vec![], vec![Selection::default()], vec![tail.clone(); 4]] {
        data.sheet_selections = values.clone();
        let wire = serde_json::to_string(&data).unwrap();
        let restored: Metadata = serde_json::from_str(&wire).unwrap();
        assert_eq!(restored.sheet_selections, values);
        assert_eq!(restored.selection, data.selection);
        assert!(wire.starts_with(
            r#"{"title":"Untitled","original_path_hint":null,"file_kind":null,"dirty_at_capture":true,"import_issues":[],"selection":"#
        ));
        for field in ["original_path_hint", "file_kind"] {
            let mut value: serde_json::Value = serde_json::from_str(&wire).unwrap();
            value.as_object_mut().unwrap().remove(field);
            assert!(serde_json::from_value::<Metadata>(value).is_err());
        }
        assert!(serde_json::from_str::<Metadata>(&wire.replacen('{', "{\"uid\":1,", 1)).is_err());
    }
}

#[test]
fn identity_and_file_kind_wire_values_are_exact() {
    let id: RecoveryId = "000102030405060708090a0b0c0d0e0f".parse().unwrap();
    assert_eq!(serde_json::to_string(&id).unwrap(), "\"000102030405060708090a0b0c0d0e0f\"");
    assert_eq!(serde_json::from_str::<RecoveryId>("\"000102030405060708090a0b0c0d0e0f\"").unwrap(), id);
    for invalid in ["", "00", "000102030405060708090A0b0c0d0e0f", "000102030405060708090g0b0c0d0e0f"] {
        assert!(invalid.parse::<RecoveryId>().is_err());
    }
    for (kind, literal) in [
        (crate::io::FileKind::Xlsx, "Xlsx"),
        (crate::io::FileKind::Xls, "Xls"),
        (crate::io::FileKind::Xlsb, "Xlsb"),
        (crate::io::FileKind::Ods, "Ods"),
        (crate::io::FileKind::Csv, "Csv"),
        (crate::io::FileKind::Tsv, "Tsv"),
        (crate::io::FileKind::Json, "Json"),
        (crate::io::FileKind::Html, "Html"),
        (crate::io::FileKind::Sylk, "Sylk"),
    ] {
        let wire = format!("\"{literal}\"");
        assert_eq!(serde_json::to_string(&RecoveryFileKind::from(kind)).unwrap(), wire);
        assert_eq!(crate::io::FileKind::from(serde_json::from_str::<RecoveryFileKind>(&wire).unwrap()), kind);
    }
    assert!(serde_json::from_str::<RecoveryFileKind>("\"Xlsm\"").is_err());
}

#[test]
fn native_dtos_preserve_nonunicode_storage_and_reject_ambiguous_tags() {
    for literal in [r#"{"unix":[47,255]}"#, r#"{"windows":[67,58,92,55296]}"#, r#"{"unix":[]}"#] {
        let path: NativePath = serde_json::from_str(literal).unwrap();
        assert_eq!(serde_json::to_string(&path).unwrap(), literal);
    }
    for literal in [r#"{}"#, r#"{"unix":[],"windows":[]}"#, r#"{"unix":[256]}"#, r#"{"windows":[65536]}"#, r#"{"other":[]}"#] {
        assert!(serde_json::from_str::<NativePath>(literal).is_err());
    }
}

#[test]
fn projection_keeps_required_empty_order_and_some_empty_preserved() {
    let mut wb = Workbook::new();
    let data = metadata();
    let empty = serde_json::to_string(&ManifestRef::new(&wb, &data, &RecoveryLimits::default()).unwrap()).unwrap();
    assert!(empty.starts_with("{\"workbook\":"));
    assert!(empty.contains(&format!("\"metadata\":{},\"sidecars\":", serde_json::to_string(&data).unwrap())));
    assert!(empty.ends_with(&format!("{{\"workbook\":{EMPTY_WORKBOOK_SIDECAR},\"sheets\":[{EMPTY_SHEET_SIDECAR}]}}}}")));
    wb.preserved = Some(Arc::new(Preserved::default()));
    let wire = serde_json::to_string(&ManifestRef::new(&wb, &data, &RecoveryLimits::default()).unwrap()).unwrap();
    let decoded: Manifest = serde_json::from_str(&wire).unwrap();
    assert!(decoded.sidecars.workbook.preserved.is_some());
    assert_eq!(decoded.sidecars.sheets[0].file_id, 0);
    assert_eq!(decoded.workbook.sheets.len(), 1);
    assert_eq!(decoded.metadata.sheet_selections.len(), 0);
    assert!(serde_json::from_str::<Manifest>(&wire.replacen('{', "{\"extra\":0,", 1)).is_err());
}

fn part(name: &str, bytes: &[u8]) -> PreservedPart {
    PreservedPart { name: name.into(), content_type: "application/synthetic".into(), data: Arc::from(bytes), rels: vec![] }
}

#[test]
fn frame_descriptors_follow_every_owner_occurrence_without_deduplication() {
    let mut wb = Workbook::new();
    let repeated = part("same.bin", &[1, 2]);
    wb.preserved =
        Some(Arc::new(Preserved { parts: vec![repeated.clone(), part("empty", &[])], vml: Some(part("book.vml", &[3])), ..Preserved::default() }));
    Arc::make_mut(&mut wb.sheets[0]).preserved =
        Some(Arc::new(Preserved { parts: vec![repeated], vml: Some(part("sheet.vml", &[5, 6])), ..Preserved::default() }));
    let mut second = Sheet::new("Second");
    second.preserved = Some(Arc::new(Preserved { parts: vec![part("last", &[7])], ..Preserved::default() }));
    wb.sheets.push(Arc::new(second));
    let data = metadata();
    let wire = serde_json::to_string(&ManifestRef::new(&wb, &data, &RecoveryLimits::default()).unwrap()).unwrap();
    let decoded: Manifest = serde_json::from_str(&wire).unwrap();
    let book = decoded.sidecars.workbook.preserved.unwrap();
    let first = decoded.sidecars.sheets[0].preserved.as_ref().unwrap();
    let second = decoded.sidecars.sheets[1].preserved.as_ref().unwrap();
    let frames: Vec<_> = book
        .parts
        .iter()
        .chain(book.vml.iter())
        .chain(first.parts.iter())
        .chain(first.vml.iter())
        .chain(second.parts.iter())
        .map(|p| (p.data.frame, p.data.length))
        .collect();
    assert_eq!(frames, [(0, 2), (1, 0), (2, 1), (3, 2), (4, 2), (5, 1)]);
    assert_eq!(decoded.sidecars.sheets.iter().map(|s| s.ordinal).collect::<Vec<_>>(), [0, 1]);
}

fn picture() -> Arc<CellPicture> {
    Arc::new(CellPicture { data: vec![1, 2], mime: "image/synthetic".into(), alt: "same".into() })
}

#[test]
fn picture_groups_preserve_global_allocation_identity_and_first_encounter_order() {
    let mut wb = Workbook::new();
    let a = picture();
    let b = picture();
    let sheet = Arc::make_mut(&mut wb.sheets[0]);
    sheet.cell_pictures.insert(CellRef::new(1, 0), a.clone());
    sheet.cell_pictures.insert(CellRef::new(0, 1), b.clone());
    sheet.cell_pictures.insert(CellRef::new(0, 0), a.clone());
    let mut second = Sheet::new("Second");
    second.cell_pictures.insert(CellRef::new(0, 0), a);
    second.cell_pictures.insert(CellRef::new(0, 1), b);
    wb.sheets.push(Arc::new(second));
    let data = metadata();
    let projected = ManifestRef::new(&wb, &data, &RecoveryLimits::default()).unwrap();
    let decoded: Manifest = serde_json::from_str(&serde_json::to_string(&projected).unwrap()).unwrap();
    assert_eq!(decoded.sidecars.workbook.cell_picture_groups, [0, 1, 0, 0, 1]);
    // Ordinary serde still creates separate Arcs. Reconstruction belongs to a later checkpoint.
    assert!(!Arc::ptr_eq(
        &decoded.workbook.sheets[0].cell_pictures[&CellRef::new(0, 0)],
        &decoded.workbook.sheets[1].cell_pictures[&CellRef::new(0, 0)]
    ));
}

#[test]
fn projection_count_limits_bound_group_storage() {
    let mut wb = Workbook::new();
    Arc::make_mut(&mut wb.sheets[0]).cell_pictures.insert(CellRef::default(), picture());
    let limits = RecoveryLimits { nodes: 15, ..RecoveryLimits::default() };
    assert!(matches!(ManifestRef::new(&wb, &metadata(), &limits), Err(RecoveryError::TooLarge("nodes"))));
    let limits = RecoveryLimits { nodes: 16, ..RecoveryLimits::default() };
    assert!(ManifestRef::new(&wb, &metadata(), &limits).is_ok());
    wb.preserved = Some(Arc::new(Preserved { parts: vec![part("p", &[1])], ..Preserved::default() }));
    let limits = RecoveryLimits { parts: 0, ..RecoveryLimits::default() };
    assert!(matches!(ManifestRef::new(&wb, &metadata(), &limits), Err(RecoveryError::TooLarge("parts"))));
}

#[cfg(unix)]
#[test]
fn borrowed_native_paths_stream_exact_bytes_and_check_storage_bound() {
    use std::os::unix::ffi::OsStringExt;
    let mut wb = Workbook::new();
    wb.source_dir = Some(std::ffi::OsString::from_vec(vec![47, 255]).into());
    wb.source_path = Some(std::path::PathBuf::from(""));
    let mut data = metadata();
    data.original_path_hint = Some(NativePath::Unix(vec![111]));
    let wire = serde_json::to_string(&ManifestRef::new(&wb, &data, &RecoveryLimits::default()).unwrap()).unwrap();
    assert!(wire.contains(r#""original_path_hint":{"unix":[111]}"#));
    assert!(wire.contains(r#""source_dir":{"unix":[47,255]},"source_path":{"unix":[]}"#));
    wb.source_dir = Some(std::ffi::OsString::from_vec(vec![1; 32768]).into());
    assert!(ManifestRef::new(&wb, &data, &RecoveryLimits::default()).is_ok());
    wb.source_dir = Some(std::ffi::OsString::from_vec(vec![1; 32769]).into());
    assert!(matches!(ManifestRef::new(&wb, &data, &RecoveryLimits::default()), Err(RecoveryError::TooLarge("native_units"))));
}

#[test]
fn sheet_sidecar_keeps_a1_spills_ranges_and_full_file_id() {
    let mut wb = Workbook::new();
    let sheet = Arc::make_mut(&mut wb.sheets[0]);
    sheet.file_id = u32::MAX;
    sheet.spill.insert(CellRef::new(0, 1), gridcraft_core::Value::Number(-0.0));
    sheet.spill_ranges.insert(CellRef::new(0, 0), RangeRef::parse("A1:B1").unwrap());
    let data = metadata();
    let wire = serde_json::to_string(&ManifestRef::new(&wb, &data, &RecoveryLimits::default()).unwrap()).unwrap();
    assert!(wire.contains(
        r#""file_id":4294967295,"spill":{"B1":{"t":"Number","v":-0.0}},"spill_ranges":{"A1":{"start":{"row":0,"col":0},"end":{"row":0,"col":1}}}"#
    ));
    let decoded: Manifest = serde_json::from_str(&wire).unwrap();
    assert_eq!(decoded.sidecars.sheets[0].file_id, u32::MAX);
    let gridcraft_core::Value::Number(value) = decoded.sidecars.sheets[0].spill[&CellRef::new(0, 1)] else { panic!("number") };
    assert_eq!(value.to_bits(), (-0.0_f64).to_bits());
}

fn strict_record<T: serde::de::DeserializeOwned>(literal: &str) {
    assert!(serde_json::from_str::<T>(literal).is_ok());
    assert!(serde_json::from_str::<T>(&literal.replacen('{', "{\"extra\":null,", 1)).is_err());
    let fields: serde_json::Map<String, serde_json::Value> = serde_json::from_str(literal).unwrap();
    for key in fields.keys() {
        let mut missing = fields.clone();
        missing.remove(key);
        assert!(serde_json::from_value::<T>(serde_json::Value::Object(missing)).is_err(), "missing {key}");
    }
}

#[test]
fn each_added_record_has_a_closed_required_field_contract() {
    strict_record::<Relationship<String>>(r#"{"id":"r1","rel_type":"urn:r","target":"p","external":false}"#);
    strict_record::<Part<String, Vec<Relationship<String>>>>(r#"{"name":"p","content_type":"c","data":{"frame":0,"length":0},"rels":[]}"#);
    strict_record::<Fragment<String>>(r#"{"slot":"ext","xml":"<e/>"}"#);
    strict_record::<KeptSheet<String>>(r#"{"name":"C","visibility":"Hidden","rel_id":"r2","before":1,"sheet_id":7}"#);
    strict_record::<Sidecars>(&format!("{{\"workbook\":{EMPTY_WORKBOOK_SIDECAR},\"sheets\":[]}}"));
    strict_record::<SheetSidecar>(EMPTY_SHEET_SIDECAR);
    strict_record::<Metadata>(&serde_json::to_string(&metadata()).unwrap());
}

#[test]
fn preserved_projection_retains_each_nonpayload_record_field() {
    let rel = gridcraft_model::PreservedRel { id: "r1".into(), rel_type: "urn:synthetic".into(), target: "part".into(), external: true };
    let mut wb = Workbook::new();
    let mut payload = part("part", &[9]);
    payload.rels.push(rel.clone());
    wb.preserved = Some(Arc::new(Preserved {
        package_rels: vec![rel.clone()],
        rels: vec![rel],
        parts: vec![payload],
        elements: vec![gridcraft_model::PreservedXml { slot: "fileVersion".into(), xml: " <e/> ".into() }],
        sheets: vec![gridcraft_model::PreservedSheet {
            name: "Kept".into(),
            visibility: gridcraft_model::Visibility::Hidden,
            rel_id: "r2".into(),
            before: 1,
            sheet_id: 7,
        }],
        ..Preserved::default()
    }));
    let data = metadata();
    let wire = serde_json::to_string(&ManifestRef::new(&wb, &data, &RecoveryLimits::default()).unwrap()).unwrap();
    let decoded: Manifest = serde_json::from_str(&wire).unwrap();
    let retained = decoded.sidecars.workbook.preserved.unwrap();
    assert_eq!(retained.package_rels[0].target, "part");
    assert_eq!(retained.rels[0].rel_type, "urn:synthetic");
    assert!(retained.parts[0].rels[0].external);
    assert_eq!(retained.parts[0].content_type, "application/synthetic");
    assert_eq!(retained.elements[0].xml, " <e/> ");
    assert_eq!((retained.sheets[0].before, retained.sheets[0].sheet_id), (1, 7));
    assert_eq!(retained.sheets[0].rel_id, "r2");
}

#[test]
fn projection_retains_future_json_hidden_names_and_pending_macro_channel() {
    let mut wb = Workbook::new();
    wb.names.push(gridcraft_model::DefinedName {
        name: "_gc_macro_0_0".into(),
        scope: None,
        formula: "kept".into(),
        comment: "inert".into(),
        hidden: true,
    });
    wb.macros_unread = Arc::new(vec![serde_json::json!({"v":u64::MAX,"rows":{"01":"kept"},"data":"not base64"})]);
    wb.macros = [gridcraft_model::Macro {
        name: "Keep".into(),
        steps: vec![gridcraft_model::Step::command("cell.set", serde_json::json!({"A1":"ordinary content"}))],
        ..gridcraft_model::Macro::default()
    }]
    .into_iter()
    .collect();
    let data = metadata();
    let wire = serde_json::to_string(&ManifestRef::new(&wb, &data, &RecoveryLimits::default()).unwrap()).unwrap();
    let decoded: Manifest = serde_json::from_str(&wire).unwrap();
    assert_eq!(decoded.sidecars.workbook.macros_unread, *wb.macros_unread);
    assert_eq!(decoded.workbook.names, wb.names);
    assert!(decoded.workbook.macros.is_empty());
    assert_eq!(decoded.workbook.macros_read.len(), 1);
    assert_eq!(decoded.workbook.macros_read[0]["name"], "Keep");
    // DTO decoding is not strict macro adoption or complete recovery acceptance.
}
