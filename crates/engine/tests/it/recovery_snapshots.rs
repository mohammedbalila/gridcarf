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
