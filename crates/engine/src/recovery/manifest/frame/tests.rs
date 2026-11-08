use super::*;
use serde::Deserialize;
use std::{
    io::{self, Write},
    sync::Arc,
};

fn toy() -> Vec<u8> {
    let mut bytes = b"GCRCV001".to_vec();
    bytes.extend(1u32.to_le_bytes());
    bytes.extend(41u64.to_le_bytes());
    bytes.extend(2u64.to_le_bytes());
    bytes.extend(b"{}");
    bytes.extend(3u64.to_le_bytes());
    bytes.extend(b"abc");
    bytes
}
fn refs() -> Vec<FrameRef> {
    vec![FrameRef { frame: 0, length: 3 }]
}
#[test]
fn literal_header_endian_lengths_and_toy_plan_are_framing_only() {
    let limits = RecoveryLimits::default();
    let bytes = toy();
    let envelope = Envelope::parse(&bytes, &limits).unwrap();
    assert_eq!(envelope.manifest, b"{}");
    let plan = envelope.plan(refs().iter(), &limits).unwrap();
    assert_eq!(plan.payloads, [&b"abc"[..]]);
    assert_eq!(plan.requested, std::mem::size_of::<&[u8]>());
    assert!(super::super::Manifest::deserialize(&mut serde_json::Deserializer::from_slice(envelope.manifest)).is_err());
    let framing = Framing::new(b"{}", [&b"abc"[..]].into_iter(), &limits).unwrap();
    assert_eq!(framing.len(), 41);
    let mut written = Vec::new();
    framing.write_to(&mut written).unwrap();
    assert_eq!(written, bytes);
    let mut version = bytes.clone();
    version[8..12].copy_from_slice(&2u32.to_le_bytes());
    assert!(matches!(Envelope::parse(&version, &limits), Err(RecoveryError::UnsupportedVersion(2))));
    for size in 0..28 {
        assert!(matches!(Envelope::parse(&bytes[..size], &limits), Err(RecoveryError::Malformed("envelope_header"))));
    }
    let mut bad = bytes.clone();
    bad[0] = 0;
    assert!(Envelope::parse(&bad, &limits).is_err());
    assert!(Envelope::parse(&bytes, &RecoveryLimits { envelope: 41, manifest: 2, ..RecoveryLimits::default() }).is_ok());
    assert!(matches!(
        Envelope::parse(&bytes, &RecoveryLimits { envelope: 40, ..RecoveryLimits::default() }),
        Err(RecoveryError::TooLarge("envelope"))
    ));
    assert!(matches!(
        Envelope::parse(&bytes, &RecoveryLimits { manifest: 1, ..RecoveryLimits::default() }),
        Err(RecoveryError::TooLarge("manifest"))
    ));
}
#[test]
fn every_frame_truncation_reference_mismatch_and_trailing_byte_is_checked() {
    let limits = RecoveryLimits::default();
    let bytes = toy();
    for size in 28..bytes.len() {
        let mut cut = bytes[..size].to_vec();
        cut[12..20].copy_from_slice(&(size as u64).to_le_bytes());
        let result = Envelope::parse(&cut, &limits).and_then(|e| e.plan(refs().iter(), &limits));
        assert!(result.is_err(), "{size}");
    }
    for descriptors in [
        vec![],
        vec![FrameRef { frame: 1, length: 3 }],
        vec![FrameRef { frame: 0, length: 2 }],
        vec![FrameRef { frame: 0, length: 3 }, FrameRef { frame: 0, length: 0 }],
    ] {
        assert!(Envelope::parse(&bytes, &limits).unwrap().plan(descriptors.iter(), &limits).is_err());
    }
    let mut extra = bytes.clone();
    extra.push(0);
    extra[12..20].copy_from_slice(&42u64.to_le_bytes());
    assert!(matches!(Envelope::parse(&extra, &limits).unwrap().plan(refs().iter(), &limits), Err(RecoveryError::Malformed("frame_trailing"))));
    assert!(
        Envelope::parse(&bytes, &RecoveryLimits { parts: 0, ..RecoveryLimits::default() })
            .unwrap()
            .plan(refs().iter(), &RecoveryLimits { parts: 0, ..RecoveryLimits::default() })
            .is_err()
    );
    assert!(Envelope::parse(&bytes, &limits).unwrap().plan(refs().iter(), &RecoveryLimits { part_bytes: 2, ..RecoveryLimits::default() }).is_err());
    let mut huge = bytes.clone();
    huge[20..28].copy_from_slice(&u64::MAX.to_le_bytes());
    assert!(matches!(Envelope::parse(&huge, &limits), Err(RecoveryError::TooLarge(_))));
    assert!(
        total_length(usize::MAX, [0].into_iter(), &RecoveryLimits { manifest: usize::MAX, envelope: usize::MAX, ..RecoveryLimits::default() })
            .is_err()
    );
}
#[test]
fn last_descriptor_failure_precedes_borrowed_plan_reservation_and_all_payload_copy() {
    let limits = RecoveryLimits::default();
    let framing = Framing::new(b"{}", [&b""[..], &b"same"[..], &b"same"[..]].into_iter(), &limits).unwrap();
    let mut bytes = Vec::new();
    framing.write_to(&mut bytes).unwrap();
    let descriptors = vec![FrameRef { frame: 0, length: 0 }, FrameRef { frame: 1, length: 4 }, FrameRef { frame: 2, length: 4 }];
    let envelope = Envelope::parse(&bytes, &limits).unwrap();
    let mut visits = 0;
    let mut invalid = descriptors;
    invalid[2].length = 3;
    assert!(
        scan(envelope.body, invalid.iter(), &limits, |_| {
            visits += 1;
            Ok(())
        })
        .is_err()
    );
    assert_eq!(visits, 2);
    assert!(envelope.plan(invalid.iter(), &limits).is_err()); // No copy accessor exists; E3 owns payload copies.
    invalid[2].length = 4;
    let plan = envelope.plan(invalid.iter(), &limits).unwrap();
    assert_eq!(plan.payloads.len(), 3);
    plan.compare_payloads([&b""[..], &b"same"[..], &b"same"[..]].into_iter()).unwrap();
    assert!(matches!(
        plan.compare_payloads([&b""[..], &b"same"[..], &b"sAme"[..]].into_iter()),
        Err(RecoveryError::UnsupportedRepresentation("frame_payload"))
    ));
}
#[test]
fn real_source_occurrences_follow_owner_storage_order_without_arc_dedup() {
    use super::super::*;
    let data: Arc<[u8]> = Arc::from([1, 2]);
    let part = |label: &str| gridcraft_model::PreservedPart { name: label.into(), content_type: "x".into(), data: data.clone(), rels: vec![] };
    let mut workbook = Workbook::new();
    workbook.preserved = Some(Arc::new(Preserved { parts: vec![part("a")], vml: Some(part("v")), ..Default::default() }));
    Arc::make_mut(&mut workbook.sheets[0]).preserved =
        Some(Arc::new(Preserved { parts: vec![part("b")], vml: Some(part("w")), ..Default::default() }));
    let limits = RecoveryLimits::default();
    let values: Vec<_> = source_frames(&workbook, &limits).unwrap().collect();
    assert_eq!(values.len(), 4);
    assert!(values.iter().all(|v| v.as_ptr() == data.as_ptr()));
    let framing = Framing::new(b"{}", source_frames(&workbook, &limits).unwrap(), &limits).unwrap();
    let mut bytes = Vec::new();
    framing.write_to(&mut bytes).unwrap();
    let refs: Vec<_> = (0..4).map(|frame| FrameRef { frame, length: 2 }).collect();
    let plan = Envelope::parse(&bytes, &limits).unwrap().plan(refs.iter(), &limits).unwrap();
    assert_eq!(plan.payloads.len(), 4);
    use std::io::Write;
    writeln!(std::io::stdout(), "E2_PLAN_REQUESTS entries={} requested={} capacity={}", plan.payloads.len(), plan.requested, plan.capacity).unwrap();
}
#[test]
fn low_level_sink_partial_writes_zero_errors_and_remaining_are_finite() {
    struct Sink {
        bytes: Vec<u8>,
        fail: Option<usize>,
        zero: bool,
        calls: usize,
    }
    impl Write for Sink {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.calls += 1;
            if self.fail == Some(self.bytes.len()) {
                return if self.zero { Ok(0) } else { Err(io::ErrorKind::BrokenPipe.into()) };
            }
            let n = buf.len().min(1);
            self.bytes.extend_from_slice(&buf[..n]);
            Ok(n)
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    let framing = Framing::new(b"{}", [&b"abc"[..]].into_iter(), &RecoveryLimits::default()).unwrap();
    for offset in [0, 28, 30, 38, 40] {
        for zero in [false, true] {
            let mut sink = Sink { bytes: vec![], fail: Some(offset), zero, calls: 0 };
            assert!(
                matches!(framing.write_to(&mut sink),Err(RecoveryError::Io(kind)) if kind==if zero{io::ErrorKind::WriteZero}else{io::ErrorKind::BrokenPipe})
            );
            assert_eq!(sink.bytes.len(), offset);
        }
    }
    let mut sink = Sink { bytes: vec![], fail: None, zero: false, calls: 0 };
    framing.write_to(&mut sink).unwrap();
    assert_eq!(sink.bytes, toy());
    let mut sink = Sink { bytes: vec![], fail: None, zero: false, calls: 0 };
    let bad = Framing::new(b"{}", [&b"abc"[..]].into_iter(), &RecoveryLimits { part_bytes: 2, ..RecoveryLimits::default() })
        .and_then(|frame| frame.write_to(&mut sink));
    assert!(bad.is_err());
    assert_eq!(sink.calls, 0);
    let mut bytes = Vec::new();
    let mut sink = Remaining::new(&mut bytes, 1);
    assert!(sink.write_all(b"xx").is_err());
    assert!(sink.finish().is_err());
    assert!(bytes.is_empty());
    let mut sink = Remaining::new(&mut bytes, 2);
    sink.write_all(b"x").unwrap();
    assert!(sink.finish().is_err());
}

#[test]
fn exact_empty_frame_count_limits_and_header_disagreements_are_framing_failures() {
    let limits = RecoveryLimits::default();
    let empty = Framing::new(b"{}", std::iter::empty(), &limits).unwrap();
    assert_eq!(empty.len(), 30);
    let mut bytes = Vec::new();
    empty.write_to(&mut bytes).unwrap();
    let envelope = Envelope::parse(&bytes, &limits).unwrap();
    let plan = envelope.plan(std::iter::empty(), &limits).unwrap();
    assert_eq!((plan.payloads.len(), plan.requested, plan.capacity), (0, 0, 0));
    let one = [&b"abc"[..]];
    assert!(Framing::new(b"{}", one.into_iter(), &RecoveryLimits { envelope: 41, parts: 1, part_bytes: 3, ..RecoveryLimits::default() }).is_ok());
    for limits in [
        RecoveryLimits { envelope: 40, ..RecoveryLimits::default() },
        RecoveryLimits { manifest: 1, ..RecoveryLimits::default() },
        RecoveryLimits { parts: 0, ..RecoveryLimits::default() },
    ] {
        let mut sink = Vec::new();
        assert!(Framing::new(b"{}", one.into_iter(), &limits).and_then(|framing| framing.write_to(&mut sink)).is_err());
        assert!(sink.is_empty());
    }
    assert!(Framing::new(b"{}", std::iter::repeat_n(&b""[..], 4096), &limits).is_ok());
    assert!(matches!(
        Framing::new(b"{}", std::iter::repeat_n(&b""[..], 4097), &RecoveryLimits { parts: usize::MAX, ..RecoveryLimits::default() }),
        Err(RecoveryError::TooLarge("parts"))
    ));
    for total in [0, 40, 42] {
        let mut bytes = toy();
        bytes[12..20].copy_from_slice(&(total as u64).to_le_bytes());
        assert!(matches!(Envelope::parse(&bytes, &limits), Err(RecoveryError::Malformed("envelope_length"))));
    }
    let mut bytes = toy();
    bytes[20..28].copy_from_slice(&42u64.to_le_bytes());
    assert!(matches!(Envelope::parse(&bytes, &limits), Err(RecoveryError::Malformed("envelope_length"))));
}

#[test]
fn real_source_and_descriptor_storage_order_are_independent_of_relationships() {
    use super::super::*;
    let part = |value: u8| PreservedPart { name: format!("{value}"), content_type: "x".into(), data: Arc::from([value]), rels: vec![] };
    let mut workbook = Workbook::new();
    workbook.sheets.push(Arc::new(gridcraft_model::Sheet::new("two")));
    workbook.preserved = Some(Arc::new(Preserved { parts: vec![part(1)], vml: Some(part(2)), ..Default::default() }));
    Arc::make_mut(&mut workbook.sheets[0]).preserved = Some(Arc::new(Preserved { parts: vec![part(3)], vml: Some(part(4)), ..Default::default() }));
    Arc::make_mut(&mut workbook.sheets[1]).preserved = Some(Arc::new(Preserved { parts: vec![part(5)], ..Default::default() }));
    let limits = RecoveryLimits::default();
    assert_eq!(source_frames(&workbook, &limits).unwrap().map(|v| v[0]).collect::<Vec<_>>(), [1, 2, 3, 4, 5]);
    let preserved = |indices: &[u32], vml: Option<u32>| PreservedData {
        package_rels: vec![],
        rels: vec![],
        parts: indices
            .iter()
            .map(|i| Part { name: "x".into(), content_type: "x".into(), data: FrameRef { frame: *i, length: 1 }, rels: vec![] })
            .collect(),
        elements: vec![],
        vml: vml.map(|i| Part { name: "v".into(), content_type: "x".into(), data: FrameRef { frame: i, length: 1 }, rels: vec![] }),
        sheets: vec![],
    };
    let sidecars = Sidecars {
        workbook: WorkbookSidecar {
            source_dir: None,
            source_path: None,
            macros_unread: vec![],
            cell_picture_groups: vec![],
            preserved: Some(preserved(&[0], Some(1))),
        },
        sheets: vec![
            SheetSidecar {
                ordinal: 0,
                file_id: 0,
                spill: Default::default(),
                spill_ranges: Default::default(),
                preserved: Some(preserved(&[2], Some(3))),
            },
            SheetSidecar {
                ordinal: 1,
                file_id: 0,
                spill: Default::default(),
                spill_ranges: Default::default(),
                preserved: Some(preserved(&[4], None)),
            },
        ],
    };
    assert_eq!(descriptors(&sidecars, &limits).unwrap().map(|r| r.frame).collect::<Vec<_>>(), [0, 1, 2, 3, 4]);
    let framing = Framing::new(b"{}", source_frames(&workbook, &limits).unwrap(), &limits).unwrap();
    let mut bytes = Vec::new();
    framing.write_to(&mut bytes).unwrap();
    let envelope = Envelope::parse(&bytes, &limits).unwrap();
    let plan = envelope.plan(descriptors(&sidecars, &limits).unwrap(), &limits).unwrap();
    plan.compare_payloads(source_frames(&workbook, &limits).unwrap()).unwrap();
    let mut reversed: Vec<_> = descriptors(&sidecars, &limits).unwrap().collect();
    reversed.swap(3, 4);
    assert!(matches!(envelope.plan(reversed.into_iter(), &limits), Err(RecoveryError::Malformed("frame_reference"))));
    assert!(source_frames(&workbook, &RecoveryLimits { sheets: 1, ..RecoveryLimits::default() }).is_err());
    assert!(descriptors(&sidecars, &RecoveryLimits { sheets: 1, ..RecoveryLimits::default() }).is_err());
}
