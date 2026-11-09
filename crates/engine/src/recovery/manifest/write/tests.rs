use super::*;
use serde::Serialize;

#[test]
fn bounded_real_writer_exact_plus_one_and_old_new_requests_use_length() {
    let value = serde_json::json!({"é":[1,-0.0,null]});
    let expected = serde_json::to_vec(&value).unwrap();
    let result = encode(&value, expected.len()).unwrap();
    assert_eq!(result.bytes, expected);
    assert!(result.growth.calls > 0 && result.growth.peak <= expected.len() * 2);
    use std::io::Write;
    writeln!(std::io::stdout(), "E2_SERDE_BUFFER_REQUESTS {:?} bytes={} capacity={}", result.growth, result.bytes.len(), result.bytes.capacity())
        .unwrap();
    assert!(matches!(encode(&value, expected.len() - 1), Err(RecoveryError::TooLarge("manifest"))));
    let mut writer = Bounded::new(16);
    writer.write_all(b"abcd").unwrap();
    writer.bytes.truncate(1);
    writer.write_all(b"efghijk").unwrap();
    assert_eq!((writer.bytes.len(), writer.requested, writer.growth.calls), (8, 8, 2));
    assert_eq!(writer.growth.peak, 12);
    assert!(writer.bytes.capacity() >= 8);
    assert!(writer.write_all(&[0; 9]).is_err());
    assert!(matches!(writer.refusal, Some(RecoveryError::TooLarge("manifest"))));
    writeln!(std::io::stdout(), "E2_BUFFER_REQUESTS {:?} requested={} observed={}", writer.growth, writer.requested, writer.bytes.capacity())
        .unwrap();
}
#[test]
fn comparison_streams_every_byte_and_preserves_primary_mismatch() {
    let value = serde_json::json!({"a":"é","z":-0.0});
    let expected = serde_json::to_vec(&value).unwrap();
    compare(&value, &expected).unwrap();
    for index in [0, expected.len() / 2, expected.len() - 1] {
        let mut bad = expected.clone();
        bad[index] ^= 1;
        assert!(matches!(compare(&value, &bad), Err(RecoveryError::UnsupportedRepresentation("manifest_writer_form"))));
    }
    for bad in [
        &expected[..expected.len() - 1],
        &[&expected[..], b"x"].concat()[..],
        br#"{"a":"\u00e9","z":-0.0}"#,
        r#"{"z":-0.0,"a":"é"}"#.as_bytes(),
        r#"{"a":"é","z":0.0}"#.as_bytes(),
    ] {
        assert!(matches!(compare(&value, bad), Err(RecoveryError::UnsupportedRepresentation("manifest_writer_form"))));
    }
    struct Bad;
    impl Serialize for Bad {
        fn serialize<S: serde::Serializer>(&self, _: S) -> Result<S::Ok, S::Error> {
            Err(serde::ser::Error::custom("hostile dynamic message"))
        }
    }
    assert!(matches!(encode(&Bad, 32), Err(RecoveryError::Malformed("manifest_writer"))));
}
#[test]
fn actual_workbook_writer_requires_real_upstream_observation() {
    use super::super::*;
    use std::sync::Arc;
    let mut workbook = Workbook::new();
    let metadata = Metadata {
        frozen_active_sheet: None,
        title: "x".into(),
        original_path_hint: None,
        file_kind: None,
        dirty_at_capture: true,
        import_issues: vec![],
        selection: crate::Selection::default(),
        sheet_selections: vec![],
        captured_revision: 0,
        id: "00000000000000000000000000000000".parse().unwrap(),
        sequence: 0,
        provenance: crate::recovery::RecoveryProvenance::Plain,
    };
    let limits = RecoveryLimits::default();
    let encoded = manifest_bytes(&workbook, &metadata, &limits).unwrap();
    compare_manifest(&workbook, &metadata, &encoded.bytes, &limits).unwrap();
    Arc::make_mut(&mut workbook.sheets[0])
        .cells
        .set(CellRef::default(), gridcraft_model::Cell { value: Value::Number(f64::NAN), ..Default::default() });
    assert!(matches!(manifest_bytes(&workbook, &metadata, &limits), Err(RecoveryError::UnsupportedRepresentation("nonfinite"))));
    workbook.macros_read = vec![serde_json::json!({})];
    assert!(matches!(manifest_bytes(&workbook, &metadata, &limits), Err(RecoveryError::UnsupportedRepresentation("pending_macros"))));
}

#[test]
fn first_writer_and_comparison_refusals_survive_a_later_serde_custom_error() {
    struct Replace;
    impl Serialize for Replace {
        fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
            use serde::ser::SerializeSeq;
            if let Ok(mut sequence) = s.serialize_seq(Some(1)) {
                let _ = sequence.serialize_element("long");
            }
            Err(serde::ser::Error::custom("later dynamic failure"))
        }
    }
    assert!(matches!(encode(&Replace, 1), Err(RecoveryError::TooLarge("manifest"))));
    assert!(matches!(compare(&Replace, b"[null]"), Err(RecoveryError::UnsupportedRepresentation("manifest_writer_form"))));
    let mut bounded = Bounded::new(0);
    assert!(bounded.write(b"x").is_err());
    let calls = bounded.growth.calls;
    assert!(bounded.write(b"y").is_err());
    assert_eq!(bounded.growth.calls, calls);
}
