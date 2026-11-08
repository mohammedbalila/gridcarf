use super::super::tests::{bytes, metadata, replace_manifest};
use super::*;
use crate::recovery::{RecoveryCapture, decode as decode_recovery};
const REL: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
pub(in super::super) fn retained() -> Workbook {
    let mut source = Workbook::new();
    let data: Arc<[u8]> = Arc::from(&b"same"[..]);
    let parts = [("customXml/zero.bin", Arc::from(&b""[..])), ("customXml/a.bin", data.clone()), ("customXml/b.bin", data)]
        .into_iter()
        .map(|(name, data)| PreservedPart { name: name.into(), content_type: "application/octet-stream".into(), data, rels: vec![] })
        .collect::<Vec<_>>();
    let rels = parts
        .iter()
        .enumerate()
        .map(|(i, p)| PreservedRel { id: format!("rId{i}"), rel_type: format!("{REL}/customXml"), target: p.name.clone(), external: false })
        .collect();
    source.preserved = Some(Arc::new(Preserved { parts, rels, ..Default::default() }));
    Arc::make_mut(&mut source.sheets[0]).preserved = Some(Arc::new(Preserved::default()));
    source
}
#[test]
fn occurrence_payloads_copy_separately_and_none_empty_owner_distinction_survives() {
    let source = retained();
    let input = bytes(&source, &metadata());
    let mut stats = Stats::default();
    let out = decode_run(&input, &RecoveryLimits::default(), &mut stats).unwrap();
    let parts = &out.workbook.preserved.as_ref().unwrap().parts;
    assert_eq!((stats.payload_copies, stats.payload_bytes), (3, 8));
    assert!(!Arc::ptr_eq(&parts[1].data, &parts[2].data));
    assert_eq!(parts[1].data, parts[2].data);
    assert!(out.workbook.sheets[0].preserved.as_ref().unwrap().is_empty());
    assert_eq!(bytes(&out.workbook, &out.metadata), input);
    use std::io::Write;
    writeln!(std::io::stdout(), "E3_MATERIALIZATION {:?}", stats).unwrap();
}
#[test]
fn last_prefix_all_descriptors_eof_and_limits_refuse_before_first_copy() {
    let input = bytes(&retained(), &metadata());
    let length = u64::from_le_bytes(input[20..28].try_into().unwrap()) as usize;
    let mut bad = input.clone();
    bad[28 + length + 8 + 8 + 4] = 5;
    let mut stats = Stats::default();
    assert!(decode_run(&bad, &RecoveryLimits::default(), &mut stats).is_err());
    assert_eq!((stats.payload_copies, stats.native_conversions), (0, 0));
    for (from, to) in [("\"frame\":2", "\"frame\":1"), ("\"length\":4", "\"length\":5"), ("\"ordinal\":0", "\"ordinal\":1")] {
        let bad = replace_manifest(&input, from, to);
        let mut stats = Stats::default();
        assert!(decode_run(&bad, &RecoveryLimits::default(), &mut stats).is_err());
        assert_eq!(stats.payload_copies, 0);
    }
    for limits in [
        RecoveryLimits { parts: 2, ..RecoveryLimits::default() },
        RecoveryLimits { part_bytes: 3, ..RecoveryLimits::default() },
        RecoveryLimits { envelope: input.len() - 1, ..RecoveryLimits::default() },
        RecoveryLimits { manifest: length - 1, ..RecoveryLimits::default() },
    ] {
        let mut stats = Stats::default();
        assert!(decode_run(&input, &limits, &mut stats).is_err());
        assert_eq!((stats.payload_copies, stats.native_conversions), (0, 0));
    }
    let exact = RecoveryLimits { parts: 3, part_bytes: 4, envelope: input.len(), manifest: length, ..RecoveryLimits::default() };
    assert!(decode(&input, &exact).is_ok());
    let mut trailing = input.clone();
    trailing.push(0);
    let total = trailing.len() as u64;
    trailing[12..20].copy_from_slice(&total.to_le_bytes());
    let mut stats = Stats::default();
    assert!(decode_run(&trailing, &RecoveryLimits::default(), &mut stats).is_err());
    assert_eq!(stats.payload_copies, 0);
    for limits in [RecoveryLimits { parts: 2, ..RecoveryLimits::default() }, RecoveryLimits { part_bytes: 3, ..RecoveryLimits::default() }] {
        let source = retained();
        assert!(prepare(&RecoveryCapture { workbook: &source, metadata: &metadata() }, &limits).is_err());
    }
}
#[test]
fn native_source_paths_and_opaque_other_platform_hint_are_independent() {
    let mut source = Workbook::new();
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStringExt;
        source.source_dir = Some(std::ffi::OsString::from_vec(vec![b'/', 0xff]).into());
        source.source_path = Some(std::ffi::OsString::from_vec(vec![b'/', 0xfe]).into());
    }
    let input = bytes(&source, &metadata());
    let out = decode_recovery(&input).unwrap();
    assert_eq!(out.workbook.source_dir, source.source_dir);
    assert_eq!(out.workbook.source_path, source.source_path);
    assert_eq!(serde_json::to_vec(&out.metadata.original_path_hint).unwrap(), serde_json::to_vec(&metadata().original_path_hint).unwrap());
    let mut stats = Stats::default();
    assert!(decode_run(&input, &RecoveryLimits { native_units: 3, ..RecoveryLimits::default() }, &mut stats).is_err());
    assert_eq!((stats.native_conversions, stats.payload_copies), (0, 0));
    assert!(decode(&input, &RecoveryLimits { native_units: 4, ..RecoveryLimits::default() }).is_ok());
    #[cfg(unix)]
    {
        let bad = replace_manifest(&input, "\"source_dir\":{\"unix\":[47,255]}", "\"source_dir\":{\"windows\":[55296]}");
        let mut stats = Stats::default();
        assert!(matches!(
            decode_run(&bad, &RecoveryLimits::default(), &mut stats),
            Err(RecoveryError::UnsupportedRepresentation("native_path_platform"))
        ));
        assert_eq!(stats.native_conversions, 0);
    }
}

#[test]
fn repeated_zero_length_frames_still_copy_distinct_owner_occurrences() {
    let mut source = retained();
    let preserved = Arc::make_mut(source.preserved.as_mut().unwrap());
    let mut zero = preserved.parts[0].clone();
    zero.name = "customXml/another-zero.bin".into();
    preserved.rels.push(PreservedRel { id: "rIdZero".into(), rel_type: format!("{REL}/customXml"), target: zero.name.clone(), external: false });
    preserved.parts.push(zero);
    let mut stats = Stats::default();
    let out = decode_run(&bytes(&source, &metadata()), &RecoveryLimits::default(), &mut stats).unwrap();
    assert_eq!((stats.payload_copies, stats.payload_bytes), (4, 8));
    let parts = &out.workbook.preserved.as_ref().unwrap().parts;
    assert!(!Arc::ptr_eq(&parts[0].data, &parts[3].data));
}
