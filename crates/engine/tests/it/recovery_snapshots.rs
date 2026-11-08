//! Public codec admission is inert data validation, never restore or provenance authority.
use gridcraft_engine::recovery::{PreparedRecovery, RecoveryCapture, RecoveryError, RecoveryId, RecoveryProvenance, ValidatedRecovery, decode};

const MINIMAL: &[u8] = include_bytes!("../../src/recovery/manifest/complete/tests/goldens/fixtures/minimal.gcr");
const ORDINARY: &[u8] = include_bytes!("../../src/recovery/manifest/complete/tests/goldens/fixtures/ordinary-5.gcr");
const OWNERS: &[u8] = include_bytes!("../../src/recovery/manifest/complete/tests/goldens/fixtures/owners.gcr");
const PICTURES: &[u8] = include_bytes!("../../src/recovery/manifest/complete/tests/goldens/fixtures/pictures.gcr");
const WITNESS: &[u8] = include_bytes!("../../src/recovery/manifest/complete/tests/goldens/fixtures/picture-witness.gcr");
const UNIX: &[u8] = include_bytes!("../../src/recovery/manifest/complete/tests/goldens/fixtures/native-unix.gcr");
const WINDOWS: &[u8] = include_bytes!("../../src/recovery/manifest/complete/tests/goldens/fixtures/native-windows.gcr");

fn good(input: &[u8]) -> ValidatedRecovery {
    match decode(input) {
        Ok(value) => value,
        Err(error) => panic!("positive literal: {error}"),
    }
}
fn error(input: &[u8]) -> RecoveryError {
    match decode(input) {
        Err(error) => error,
        Ok(_) => panic!("hostile input accepted"),
    }
}
fn malformed(input: &[u8], reason: &str) {
    match error(input) {
        RecoveryError::Malformed(actual) => assert_eq!(actual, reason),
        other => panic!("{other:?}"),
    }
}
fn unsupported(input: &[u8], reason: &str) {
    match error(input) {
        RecoveryError::UnsupportedRepresentation(actual) => assert_eq!(actual, reason),
        other => panic!("{other:?}"),
    }
}
fn large(input: &[u8], reason: &str) {
    match error(input) {
        RecoveryError::TooLarge(actual) => assert_eq!(actual, reason),
        other => panic!("{other:?}"),
    }
}
fn total(bytes: &mut [u8]) {
    let length = bytes.len() as u64;
    bytes[12..20].copy_from_slice(&length.to_le_bytes());
}
fn edit(input: &[u8], from: &str, to: &str) -> Vec<u8> {
    let length = u64::from_le_bytes(input[20..28].try_into().unwrap()) as usize;
    let text = std::str::from_utf8(&input[28..28 + length]).unwrap();
    assert_eq!(text.matches(from).count(), 1, "target must be unique: {from}");
    let new = text.replacen(from, to, 1);
    let mut out = input[..28].to_vec();
    out.extend_from_slice(new.as_bytes());
    out.extend_from_slice(&input[28 + length..]);
    total(&mut out);
    out[20..28].copy_from_slice(&(new.len() as u64).to_le_bytes());
    out
}

fn public_signatures<'a>(_: std::marker::PhantomData<RecoveryCapture<'a>>) {
    let _: fn(&RecoveryCapture<'a>) -> Result<PreparedRecovery<'a>, RecoveryError> = RecoveryCapture::<'a>::prepare;
    let _: fn(&PreparedRecovery<'a>) -> usize = PreparedRecovery::<'a>::len;
    let _: fn(&PreparedRecovery<'a>) -> bool = PreparedRecovery::<'a>::is_empty;
    let _: fn(&PreparedRecovery<'a>, &mut dyn std::io::Write) -> Result<(), RecoveryError> = PreparedRecovery::<'a>::write_to;
}

#[test]
fn public_types_and_complete_literals_expose_only_inert_identity() {
    public_signatures(std::marker::PhantomData);
    let minimal = good(MINIMAL);
    assert_eq!(minimal.id(), "00000000000000000000000000000000".parse::<RecoveryId>().unwrap());
    assert_eq!((minimal.sequence(), minimal.captured_revision()), (0, 0));
    let ordinary = good(ORDINARY);
    assert_eq!(ordinary.id(), "0123456789abcdef0123456789abcdef".parse::<RecoveryId>().unwrap());
    assert_eq!((ordinary.sequence(), ordinary.captured_revision()), (7, u64::MAX));
    assert_eq!(serde_json::to_string(&ordinary.id()).unwrap(), "\"0123456789abcdef0123456789abcdef\"");
    for input in
        [OWNERS, PICTURES, WITNESS, include_bytes!("../../src/recovery/manifest/complete/tests/goldens/fixtures/empty-owners.gcr").as_slice()]
    {
        assert_eq!(good(input).sequence(), 0);
    }
}

#[test]
fn public_kinds_and_native_units_follow_complete_platform_contract() {
    for input in [
        include_bytes!("../../src/recovery/manifest/complete/tests/goldens/fixtures/kind-Xlsx.gcr").as_slice(),
        include_bytes!("../../src/recovery/manifest/complete/tests/goldens/fixtures/kind-Xls.gcr").as_slice(),
        include_bytes!("../../src/recovery/manifest/complete/tests/goldens/fixtures/kind-Xlsb.gcr").as_slice(),
        include_bytes!("../../src/recovery/manifest/complete/tests/goldens/fixtures/kind-Ods.gcr").as_slice(),
        include_bytes!("../../src/recovery/manifest/complete/tests/goldens/fixtures/kind-Csv.gcr").as_slice(),
        include_bytes!("../../src/recovery/manifest/complete/tests/goldens/fixtures/kind-Tsv.gcr").as_slice(),
        include_bytes!("../../src/recovery/manifest/complete/tests/goldens/fixtures/kind-Json.gcr").as_slice(),
        include_bytes!("../../src/recovery/manifest/complete/tests/goldens/fixtures/kind-Html.gcr").as_slice(),
        include_bytes!("../../src/recovery/manifest/complete/tests/goldens/fixtures/kind-Sylk.gcr").as_slice(),
    ] {
        assert_eq!(good(input).captured_revision(), 0);
    }
    #[cfg(unix)]
    {
        good(UNIX);
        unsupported(WINDOWS, "native_path_platform");
    }
    #[cfg(windows)]
    {
        good(WINDOWS);
        unsupported(UNIX, "native_path_platform");
    }
    #[cfg(not(any(unix, windows)))]
    {
        unsupported(UNIX, "native_path_platform");
        unsupported(WINDOWS, "native_path_platform");
    }
}

#[test]
fn public_envelope_checks_actual_lengths_before_declared_allocations() {
    good(MINIMAL);
    let mut bad = MINIMAL.to_vec();
    bad[0] = 0;
    malformed(&bad, "envelope_header");
    let mut bad = MINIMAL.to_vec();
    bad[8..12].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(matches!(error(&bad), RecoveryError::UnsupportedVersion(u32::MAX)));
    let mut bad = MINIMAL.to_vec();
    bad[12..20].copy_from_slice(&u64::MAX.to_le_bytes());
    large(&bad, "envelope");
    let mut bad = MINIMAL.to_vec();
    bad[20..28].copy_from_slice(&u64::MAX.to_le_bytes());
    large(&bad, "manifest");
    for cut in [0, 7, 27, 28, MINIMAL.len() - 1] {
        malformed(&MINIMAL[..cut], if cut < 28 { "envelope_header" } else { "envelope_length" });
    }
    let mut bad = MINIMAL.to_vec();
    bad.push(0);
    malformed(&bad, "envelope_length");
    total(&mut bad);
    malformed(&bad, "frame_trailing");
    let mut bad = MINIMAL.to_vec();
    bad[20..28].copy_from_slice(&4000u64.to_le_bytes());
    malformed(&bad, "envelope_length");
    malformed(include_bytes!("../../src/recovery/manifest/complete/tests/goldens/fixtures/toy.gcr"), "manifest_schema");
}

#[test]
fn public_all_frame_references_prefixes_last_body_and_eof_are_checked() {
    good(OWNERS);
    malformed(&edit(OWNERS, "\"frame\":8", "\"frame\":7"), "frame_reference");
    malformed(&edit(OWNERS, "\"length\":6", "\"length\":7"), "frame_length");
    large(&edit(OWNERS, "\"frame\":0,\"length\":7", "\"frame\":0,\"length\":67108865"), "part_bytes");
    let mut bad = OWNERS.to_vec();
    bad[6641..6649].copy_from_slice(&7u64.to_le_bytes());
    malformed(&bad, "frame_length");
    for (cut, reason) in [(6648, "frame_length"), (6654, "frame_body")] {
        let mut bad = OWNERS[..cut].to_vec();
        total(&mut bad);
        malformed(&bad, reason);
    }
    let mut bad = OWNERS.to_vec();
    bad.push(0);
    total(&mut bad);
    malformed(&bad, "frame_trailing");
}

#[test]
fn public_required_wrappers_ordinals_and_decoded_duplicates_refuse_precisely() {
    good(MINIMAL);
    good(PICTURES);
    for (from, to) in [
        ("\"source_dir\":null,", ""),
        ("\"original_path_hint\":null,", ""),
        ("\"cell_picture_groups\":[],", ""),
        ("{\"workbook\":{\"sheets\":", "{\"unknown_wrapper\":0,\"workbook\":{\"sheets\":"),
    ] {
        malformed(&edit(MINIMAL, from, to), "manifest_schema");
    }
    malformed(&edit(MINIMAL, "\"ordinal\":0", "\"ordinal\":1"), "sheet_ordinals");
    const SHEET: &str = "[{\"ordinal\":0,\"file_id\":0,\"spill\":{},\"spill_ranges\":{},\"preserved\":null}]";
    malformed(&edit(MINIMAL, SHEET, "[]"), "sheet_ordinals");
    malformed(&edit(PICTURES, "\"ordinal\":1", "\"ordinal\":0"), "sheet_ordinals");
    malformed(&edit(MINIMAL, "\"metadata\":{\"title\":\"\"", "\"metadata\":{\"title\":\"\",\"\\u0074itle\":\"duplicate\""), "duplicate_key");
}

#[test]
fn public_raw_contexts_group_identity_and_bounds_do_not_acquire_general_roles() {
    good(ORDINARY);
    good(PICTURES);
    unsupported(&edit(ORDINARY, "\"vba_project\":\"AP8=\"", "\"vba_project\":\"AP9=\""), "base64");
    unsupported(&edit(ORDINARY, "\"cols\":{\"16383\":", "\"cols\":{\"016383\":"), "numeric_key");
    malformed(&edit(PICTURES, "{\"row\":0,\"col\":1}", "{\"row\":0,\"col\":0}"), "picture_coordinate");
    malformed(&edit(PICTURES, "\"cell_picture_groups\":[0,1,0,0]", "\"cell_picture_groups\":[0,2,0,0]"), "picture_groups");
    malformed(&edit(ORDINARY, "\"cell_pictures\":[[", "\"cell_pictures\":[[null,"), "picture_coordinate");
    let deep = format!("\"data\":{}0{}", "[".repeat(65), "]".repeat(65));
    large(&edit(ORDINARY, "\"data\":\"not-base64\"", &deep), "depth");
    let long = format!("\"metadata\":{{\"title\":\"{}\"", "x".repeat(32769));
    large(&edit(MINIMAL, "\"metadata\":{\"title\":\"\"", &long), "metadata");
    malformed(&edit(MINIMAL, "\"original_path_hint\":null", "\"original_path_hint\":{\"windows\":[65536]}"), "unsigned_scalar");
}

#[test]
fn public_model_graph_macro_and_provenance_gates_are_actual_complete_checks() {
    good(MINIMAL);
    good(ORDINARY);
    good(OWNERS);
    good(WITNESS);
    malformed(&edit(MINIMAL, "\"bands\":{}", "\"bands\":{\"0\":{\"rows\":{}}}"), "model_storage");
    malformed(&edit(OWNERS, "\"target\":\"customXml/book.xml\"", "\"target\":\"customXml/missing.xml\""), "preserved_missing_target");
    malformed(&edit(WITNESS, "\"cell_picture_groups\":[0,0]", "\"cell_picture_groups\":[0,1]"), "preserved_output_name");
    unsupported(&edit(ORDINARY, "\"v\":1,\"name\":\"M\"", "\"v\":1,\"name\":\" M \""), "macro_adoption");
    unsupported(&edit(ORDINARY, "\"v\":1,\"name\":\"M\"", "\"v\":2,\"name\":\"M\""), "macro_channel");
    for (name, expected) in [("EncryptedOrigin", RecoveryProvenance::EncryptedOrigin), ("Unknown", RecoveryProvenance::Unknown)] {
        let bad = edit(MINIMAL, "\"provenance\":\"Plain\"", &format!("\"provenance\":\"{name}\""));
        assert!(matches!(error(&bad),RecoveryError::Ineligible(p) if p==expected));
    }
    let bad = edit(ORDINARY, "\"effect\":\"static\"", "\"effect\":\"encryptionRemoved\"");
    assert!(matches!(error(&bad), RecoveryError::Ineligible(RecoveryProvenance::EncryptedOrigin)));
}

#[test]
fn public_final_writer_refuses_only_targeted_spelling_order_default_and_escape_loss() {
    good(MINIMAL);
    for (from, to) in [
        ("\"size\":11.0", "\"size\":11e0"),
        ("\"name\":\"Calibri\",\"size\":11.0", "\"size\":11.0,\"name\":\"Calibri\""),
        ("\"major_font_ea\":\"\",", ""),
        ("\"metadata\":{\"title\":\"\"", "\"metadata\":{\"title\":\"\\u0061\""),
        ("\"workbook\":{\"sheets\":", "\"workbook\":{\"ignored_nested\":0,\"sheets\":"),
    ] {
        unsupported(&edit(MINIMAL, from, to), "manifest_writer_form");
    }
}

#[test]
fn actual_public_errors_are_finite_std_errors_without_hostile_payload_text() {
    good(MINIMAL);
    good(OWNERS);
    const MARKER: &str = "INJECTED_DO_NOT_DISPLAY_/private/secret.grc";
    let unknown = edit(MINIMAL, "{\"workbook\":{\"sheets\":", &format!("{{\"{MARKER}\":0,\"workbook\":{{\"sheets\":"));
    let target = edit(OWNERS, "\"target\":\"customXml/book.xml\"", &format!("\"target\":\"{MARKER}\""));
    for input in [unknown, target] {
        let actual = error(&input);
        let standard: &dyn std::error::Error = &actual;
        assert!(standard.source().is_none());
        for text in [actual.to_string(), format!("{actual:?}")] {
            assert!(!text.contains(MARKER));
            assert!(!text.contains("secret.grc"));
            assert!(!text.contains("/private/"));
            assert!(text.len() < 160);
        }
    }
}

#[test]
fn public_restore_retains_literal_content_and_inert_identity() {
    use gridcraft_engine::{
        Trust,
        core::{CellRef, RangeRef, Value},
    };
    let input = edit(ORDINARY, "\"text\":\"bad(\"", &format!("\"text\":\"{}\"", "x".repeat(4097)));
    let doc = gridcraft_engine::recovery::restore(good(&input)).unwrap();
    assert_eq!(doc.trust, Trust::FromFile);
    assert!(!doc.scripts_trusted);
    assert!(doc.is_dirty());
    assert!(doc.path.is_none());
    assert_eq!(doc.recovery_id(), Some("0123456789abcdef0123456789abcdef".parse().unwrap()));
    assert_eq!(doc.recovery_captured_revision(), Some(u64::MAX));
    assert_eq!(doc.revision, 1);
    assert_eq!(doc.title, "golden 😀\n\"\\");
    assert_eq!(doc.import_issues().len(), 2);
    assert_eq!(doc.import_issues()[0].code, "raw.code");
    assert_eq!(doc.import_issues()[0].description, "kept");
    assert_eq!(doc.selection.active, CellRef::new(7, 3));
    assert_eq!(doc.selection.anchor, CellRef::new(9, 9));
    assert_eq!(doc.sheet_selections.len(), 5);
    assert_eq!(doc.sheet_selections[3].active, CellRef::new(200, 200));
    assert_eq!(doc.sheet_selections[4].ranges, [RangeRef::new(CellRef::default(), CellRef::new(1048575, 16383))]);
    let sh = doc.wb.sheet(0).unwrap();
    assert_eq!(sh.cells.get(CellRef::new(6, 0)).unwrap().value, Value::Number(42.));
    let f = sh.cells.get(CellRef::default()).unwrap().formula.as_ref().unwrap();
    assert_eq!(f.text, "=retained broken(");
    assert_eq!(f.eval_mode, gridcraft_model::FormulaEvalMode::Legacy);
    assert!(f.expr.is_none());
    assert_eq!(sh.cells.get(CellRef::new(1, 0)).unwrap().py.as_ref().unwrap().code, "x\r\n\"😀\"");
    assert_eq!(sh.spill.get(&CellRef::new(0, 1)), Some(&Value::Empty));
    assert_eq!(doc.calc.last_recalc_cells, 0);
    assert_eq!(doc.wb.calc.mode, gridcraft_model::CalcMode::Manual);
    assert_eq!(doc.content_kind, Some(gridcraft_engine::io::FileKind::Sylk));
    for input in [OWNERS, PICTURES, WITNESS] {
        let doc = gridcraft_engine::recovery::restore(good(input)).unwrap();
        assert!(doc.is_dirty());
        assert!(!doc.scripts_trusted);
    }
}
struct RestoreFiles(std::path::PathBuf);
impl RestoreFiles {
    fn new() -> Self {
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!("rcv-{}-{}", std::process::id(), NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)));
        std::fs::create_dir(&path).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        Self(path)
    }
}
impl Drop for RestoreFiles {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
#[test]
fn only_successful_real_json_xlsx_save_clears_recovery_dirty() {
    use gridcraft_engine::Session;
    use serde_json::json;
    let files = RestoreFiles::new();
    for extension in ["json", "xlsx"] {
        let doc = gridcraft_engine::recovery::restore(good(MINIMAL)).unwrap();
        let id = doc.recovery_id();
        let mut session = Session::new();
        session.add_document(doc);
        session.execute_headless("file.saveBytes", json!({"format":extension})).unwrap();
        assert!(session.doc().unwrap().is_dirty());
        assert!(session.execute_headless("file.save", json!({})).is_err());
        assert!(session.doc().unwrap().is_dirty());
        let unavailable = files.0.join("missing").join(format!("book.{extension}"));
        assert!(session.execute_headless("file.save", json!({"path":unavailable})).is_err());
        assert!(session.doc().unwrap().is_dirty());
        session.execute_headless("file.save", json!({"path":files.0.join(format!("export-{extension}.csv"))})).unwrap();
        assert!(session.doc().unwrap().is_dirty());
        assert!(session.doc().unwrap().path.is_none());
        let path = files.0.join(format!("book.{extension}"));
        session.execute_headless("file.save", json!({"path":path})).unwrap();
        assert!(!session.doc().unwrap().is_dirty());
        assert_eq!(session.doc().unwrap().recovery_id(), id);
        assert_eq!(session.doc().unwrap().path.as_deref(), path.to_str());
        assert!(std::fs::metadata(path).unwrap().len() > 0);
    }
}
#[test]
fn restore_never_adopts_original_hint_or_changes_original_bytes() {
    use gridcraft_engine::core::CellRef;
    let files = RestoreFiles::new();
    let original = files.0.join("original.json");
    std::fs::write(&original, b"inert original sentinel").unwrap();
    #[cfg(unix)]
    let hint = {
        use std::os::unix::ffi::OsStrExt;
        serde_json::json!({"unix":original.as_os_str().as_bytes()})
    };
    #[cfg(windows)]
    let hint = {
        use std::os::windows::ffi::OsStrExt;
        serde_json::json!({"windows":original.as_os_str().encode_wide().collect::<Vec<_>>()})
    };
    #[cfg(not(any(unix, windows)))]
    let hint = serde_json::json!({"unix":[1,2,3]});
    let input = edit(MINIMAL, "\"original_path_hint\":null", &format!("\"original_path_hint\":{hint}"));
    let mut doc = gridcraft_engine::recovery::restore(good(&input)).unwrap();
    assert!(doc.path.is_none());
    doc.store_view(&[(0, CellRef::new(5, 5))]);
    assert!(doc.is_dirty());
    assert_eq!(std::fs::read(original).unwrap(), b"inert original sentinel");
}
