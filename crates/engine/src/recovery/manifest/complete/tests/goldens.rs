//! Initial synthetic literals require Root's complete byte/field review; never regenerate here.
use super::*;
mod fields;
mod source;
const MINIMAL: &[u8] = include_bytes!("goldens/fixtures/minimal.gcr");

fn check(input: &[u8], workbook: &Workbook, metadata: &Metadata, total: usize, manifest: usize, frames: &[(usize, &[u8])]) -> ValidatedRecovery {
    assert_eq!(input.len(), total);
    assert_eq!(&input[..12], b"GCRCV001\x01\0\0\0");
    assert_eq!(u64::from_le_bytes(input[12..20].try_into().unwrap()), total as u64);
    assert_eq!(u64::from_le_bytes(input[20..28].try_into().unwrap()), manifest as u64);
    let mut end = 28 + manifest;
    for (offset, body) in frames {
        assert_eq!(end, *offset);
        assert_eq!(u64::from_le_bytes(input[end..end + 8].try_into().unwrap()), body.len() as u64);
        assert_eq!(&input[end + 8..end + 8 + body.len()], *body);
        end += 8 + body.len();
    }
    assert_eq!(end, total);
    let prepared = RecoveryCapture { workbook, metadata }.prepare().unwrap();
    let mut actual = Vec::new();
    prepared.write_to(&mut actual).unwrap();
    assert_eq!(actual.len(), input.len(), "encoded literal length");
    assert!(actual == input, "first literal mismatch at {:?}", actual.iter().zip(input).position(|(a, b)| a != b));
    assert_eq!(prepared.len(), total);
    let decoded = decode_recovery(input).unwrap();
    fields::same(workbook, metadata, &decoded);
    assert_eq!(bytes(&decoded.workbook, &decoded.metadata), input);
    decoded
}

#[test]
fn literal_minimal_complete_envelope_has_independent_header_and_default_fields() {
    let (workbook, metadata) = source::minimal();
    check(MINIMAL, &workbook, &metadata, 3204, 3176, &[]);
    assert_eq!(&MINIMAL[..28], &[71, 67, 82, 67, 86, 48, 48, 49, 1, 0, 0, 0, 132, 12, 0, 0, 0, 0, 0, 0, 104, 12, 0, 0, 0, 0, 0, 0]);
}

#[test]
fn literal_small_general_context_counts_and_real_depth_boundaries_are_independent() {
    const SMALL: &str = r#"{"general":[null,true,{"metadata":"é\n","n":-0.0}]}"#;
    let limits = RecoveryLimits { metadata: 1, ..RecoveryLimits::default() };
    let raw = preflight::lexical_audit(SMALL.as_bytes(), &limits).unwrap();
    let value: serde_json::Value = serde_json::from_str(SMALL).unwrap();
    let borrowed = observe::observe(&value, &limits).unwrap();
    for count in [raw, borrowed] {
        assert_eq!((count.nodes, count.keys, count.max_depth, count.decoded_bytes, count.max_string), (10, 3, 3, 19, 8));
        assert_eq!((count.native_bytes, count.groups, count.parts, count.cells), (0, 0, 0, 0));
    }
    assert_eq!((raw.number_bytes, raw.max_number), (4, 4));
}

#[test]
fn literal_ordinary_inventory_and_exact_selection_memory_cover_source_fields() {
    let workbook = source::ordinary();
    for (input, memory, total, manifest) in [
        (include_bytes!("goldens/fixtures/ordinary-1.gcr").as_slice(), 1, 39651, 39623),
        (include_bytes!("goldens/fixtures/ordinary-3.gcr").as_slice(), 3, 39905, 39877),
        (include_bytes!("goldens/fixtures/ordinary-5.gcr").as_slice(), 5, 40169, 40141),
    ] {
        let decoded = check(input, &workbook, &source::full_metadata(memory), total, manifest, &[]);
        fields::ordinary(&decoded);
    }
}
#[test]
fn literal_all_nine_file_kinds_are_explicit_and_extension_independent() {
    use crate::recovery::RecoveryFileKind::*;
    for (input, kind, total, manifest) in [
        (include_bytes!("goldens/fixtures/kind-Xlsx.gcr").as_slice(), Xlsx, 3206, 3178),
        (include_bytes!("goldens/fixtures/kind-Xls.gcr").as_slice(), Xls, 3205, 3177),
        (include_bytes!("goldens/fixtures/kind-Xlsb.gcr").as_slice(), Xlsb, 3206, 3178),
        (include_bytes!("goldens/fixtures/kind-Ods.gcr").as_slice(), Ods, 3205, 3177),
        (include_bytes!("goldens/fixtures/kind-Csv.gcr").as_slice(), Csv, 3205, 3177),
        (include_bytes!("goldens/fixtures/kind-Tsv.gcr").as_slice(), Tsv, 3205, 3177),
        (include_bytes!("goldens/fixtures/kind-Json.gcr").as_slice(), Json, 3206, 3178),
        (include_bytes!("goldens/fixtures/kind-Html.gcr").as_slice(), Html, 3206, 3178),
        (include_bytes!("goldens/fixtures/kind-Sylk.gcr").as_slice(), Sylk, 3206, 3178),
    ] {
        let (workbook, mut metadata) = source::minimal();
        metadata.file_kind = Some(kind);
        check(input, &workbook, &metadata, total, manifest, &[]);
    }
}

#[test]
fn literal_preserved_owners_cover_every_record_field_and_occurrence_frame_offset() {
    let input = include_bytes!("goldens/fixtures/owners.gcr");
    let w = source::owners();
    let (_, m) = source::minimal();
    let decoded = check(
        input,
        &w,
        &m,
        6655,
        6125,
        &[
            (6153, b"<root/>"),
            (6168, b"<root/>"),
            (6183, source::CHART.as_bytes()),
            (6462, source::DRAWING.as_bytes()),
            (6561, b""),
            (6569, &[7, 8]),
            (6579, b"<queryTable/>"),
            (6600, &png()),
            (6641, b"<xml/>"),
        ],
    );
    let parts = &decoded.workbook.preserved.as_ref().unwrap().parts;
    assert!(!Arc::ptr_eq(&parts[0].data, &parts[1].data));
    assert_eq!(parts[0].data, parts[1].data);
    let empty = source::empty_owners();
    check(include_bytes!("goldens/fixtures/empty-owners.gcr"), &empty, &m, 3350, 3322, &[]);
}
#[test]
fn literal_global_picture_groups_preserve_aliases_and_equal_distinct_arcs() {
    let (_, m) = source::minimal();
    let decoded = check(include_bytes!("goldens/fixtures/pictures.gcr"), &source::pictures(false), &m, 4890, 4862, &[]);
    let a = decoded.workbook.sheets[0].cell_pictures.values().collect::<Vec<_>>();
    let cross = decoded.workbook.sheets[1].cell_pictures.values().next().unwrap();
    assert!(Arc::ptr_eq(a[0], a[2]) && Arc::ptr_eq(a[0], cross));
    assert!(!Arc::ptr_eq(a[0], a[1]));
    assert_eq!(a[0], a[1]);
    assert_eq!(picture_groups(&decoded.workbook, &RecoveryLimits::default()).unwrap(), [0, 1, 0, 0]);
    let witness =
        check(include_bytes!("goldens/fixtures/picture-witness.gcr"), &source::pictures(true), &m, 3831, 3779, &[(3807, b"retained-witness")]);
    assert_eq!(witness.workbook.preserved.as_ref().unwrap().parts[0].name, "xl/media/cellImage2.png/child.bin");
}
#[test]
fn literal_native_units_preserve_context_and_have_explicit_platform_expectations() {
    let unix = include_bytes!("goldens/fixtures/native-unix.gcr");
    let windows = include_bytes!("goldens/fixtures/native-windows.gcr");
    #[cfg(unix)]
    {
        let (w, m) = source::native();
        check(unix, &w, &m, 3247, 3219, &[]);
        assert!(matches!(decode_recovery(windows), Err(RecoveryError::UnsupportedRepresentation("native_path_platform"))));
    }
    #[cfg(windows)]
    {
        let (w, m) = source::native();
        check(windows, &w, &m, 3258, 3230, &[]);
        assert!(matches!(decode_recovery(unix), Err(RecoveryError::UnsupportedRepresentation("native_path_platform"))));
    }
}
#[test]
fn literal_raw_guards_reach_base64_pair_and_numeric_map_contexts() {
    let input = include_bytes!("goldens/fixtures/ordinary-5.gcr");
    for (from, to) in [
        ("\"vba_project\":\"AP8=\"", "\"vba_project\":\"AP9=\""),
        ("\"cell_pictures\":[[", "\"cell_pictures\":[[null,"),
        ("\"cols\":{\"16383\":", "\"cols\":{\"016383\":"),
    ] {
        let bad = replace_manifest(input, from, to);
        let mut stats = Stats::default();
        assert!(decode_run(&bad, &RecoveryLimits::default(), &mut stats).is_err());
        assert_eq!((stats.d2_decode, stats.payload_copies, stats.final_compare), (0, 0, 0));
    }
    assert!(decode_recovery(include_bytes!("goldens/fixtures/toy.gcr")).is_err());
    let d64 = include_bytes!("goldens/fixtures/depth64.json");
    let d65 = include_bytes!("goldens/fixtures/depth65.json");
    let counted = preflight::lexical_audit(d64, &RecoveryLimits::default()).unwrap();
    assert_eq!((d64.len(), counted.nodes, counted.max_depth), (129, 65, 64));
    assert!(preflight::preflight_manifest(d65, &RecoveryLimits::default()).is_err());
    for (raw, accept) in [(d64.as_slice(), true), (d65.as_slice(), false)] {
        let value: serde_json::Value = serde_json::from_slice(raw).unwrap();
        assert_eq!(observe::observe(&value, &RecoveryLimits::default()).is_ok(), accept);
    }
    const NATIVE: &str = r#"{"metadata":{"original_path_hint":{"windows":[55296,0]}}}"#;
    for counts in [
        preflight::lexical_audit(NATIVE.as_bytes(), &RecoveryLimits::default()).unwrap(),
        observe::observe(&serde_json::from_str::<serde_json::Value>(NATIVE).unwrap(), &RecoveryLimits::default()).unwrap(),
    ] {
        assert_eq!((counts.nodes, counts.keys, counts.max_depth, counts.decoded_bytes, counts.native_bytes), (9, 3, 4, 33, 4));
    }
}
