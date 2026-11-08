use super::super::tests::{bytes, metadata as fixture, replace_manifest};
use super::*;
use crate::recovery::{RecoveryCapture, decode as decode_recovery};

#[test]
fn prepare_real_d1_node_refusal_precedes_metadata_selection_work() {
    let source = Workbook::new();
    let mut meta = fixture();
    meta.selection.ranges.clear();
    assert!(matches!(
        prepare(&RecoveryCapture { workbook: &source, metadata: &meta }, &RecoveryLimits { nodes: 0, ..RecoveryLimits::default() }),
        Err(RecoveryError::TooLarge("nodes"))
    ));
}
#[test]
fn standalone_selection_invariants_and_entire_memory_are_enforced_without_sheet_lookup() {
    let source = Workbook::new();
    for length in [0, 1, 4, 9] {
        let mut meta = fixture();
        meta.sheet_selections = vec![Selection::at(CellRef::new(100, 10)); length];
        meta.selection.anchor = CellRef::new(999, 3);
        let out = decode_recovery(&bytes(&source, &meta)).unwrap();
        assert_eq!(out.metadata.sheet_selections, meta.sheet_selections);
        assert_eq!(out.metadata.selection, meta.selection);
    }
    for case in 0..5 {
        let mut meta = fixture();
        match case {
            0 => meta.selection.ranges.clear(),
            1 => meta.selection.active = CellRef::new(1, 0),
            2 => meta.selection.anchor.row = MAX_ROWS,
            3 => meta.selection.ranges[0] = RangeRef { start: CellRef::new(2, 0), end: CellRef::default() },
            _ => meta.sheet_selections[3].ranges[0].end.col = MAX_COLS,
        }
        assert!(matches!(RecoveryCapture { workbook: &source, metadata: &meta }.prepare(), Err(RecoveryError::Malformed("selection"))));
        let manifest = write::manifest_bytes(&source, &meta, &RecoveryLimits::default()).unwrap().bytes;
        let mut input = Vec::new();
        frame::Framing::new(&manifest, std::iter::empty(), &RecoveryLimits::default()).unwrap().write_to(&mut input).unwrap();
        assert!(matches!(decode_recovery(&input), Err(RecoveryError::Malformed("selection"))));
    }
}
#[test]
fn typed_provenance_and_encryption_facts_refuse_even_with_plain_label() {
    let source = Workbook::new();
    let input = bytes(&source, &fixture());
    for (name, provenance) in [("EncryptedOrigin", RecoveryProvenance::EncryptedOrigin), ("Unknown", RecoveryProvenance::Unknown)] {
        let bad = replace_manifest(&input, "\"provenance\":\"Plain\"", &format!("\"provenance\":\"{name}\""));
        assert!(matches!(decode_recovery(&bad),Err(RecoveryError::Ineligible(p)) if p==provenance));
    }
    let mut meta = fixture();
    meta.import_issues.push(gridcraft_model::CompatibilityIssue::new(
        "inert",
        None,
        gridcraft_model::CompatibilityEffect::EncryptionRemoved,
        gridcraft_model::Editability::Unknown,
        "synthetic",
    ));
    assert!(matches!(
        RecoveryCapture { workbook: &source, metadata: &meta }.prepare(),
        Err(RecoveryError::Ineligible(RecoveryProvenance::EncryptedOrigin))
    ));
    let manifest = write::manifest_bytes(&source, &meta, &RecoveryLimits::default()).unwrap().bytes;
    let mut bad = Vec::new();
    frame::Framing::new(&manifest, std::iter::empty(), &RecoveryLimits::default()).unwrap().write_to(&mut bad).unwrap();
    assert!(matches!(decode_recovery(&bad), Err(RecoveryError::Ineligible(RecoveryProvenance::EncryptedOrigin))));
}
#[test]
fn all_nine_kinds_and_required_nullable_metadata_roundtrip() {
    let source = Workbook::new();
    for kind in [
        RecoveryFileKind::Xlsx,
        RecoveryFileKind::Xls,
        RecoveryFileKind::Xlsb,
        RecoveryFileKind::Ods,
        RecoveryFileKind::Csv,
        RecoveryFileKind::Tsv,
        RecoveryFileKind::Json,
        RecoveryFileKind::Html,
        RecoveryFileKind::Sylk,
    ] {
        let mut meta = fixture();
        meta.file_kind = Some(kind);
        meta.original_path_hint = None;
        let out = decode_recovery(&bytes(&source, &meta)).unwrap();
        assert_eq!(serde_json::to_vec(&out.metadata).unwrap(), serde_json::to_vec(&meta).unwrap());
    }
}
