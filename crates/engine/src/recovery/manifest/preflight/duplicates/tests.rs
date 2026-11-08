use super::*;

fn check(raw: &str) -> Result<StructuralStats, RecoveryError> {
    audit(raw.as_bytes(), &RecoveryLimits::default())
}

#[test]
fn decoded_duplicates_are_refused_in_every_object_scope() {
    for raw in [
        r#"{"a":1,"\u0061":2}"#,
        r#"{"":null,"":false}"#,
        r#"{"é":[],"\u00e9":{}}"#,
        r#"{"a":{"x":1,"x":2}}"#,
        r#"{"workbook":{"macros":[{"steps":[{"params":{"x":0,"x":1}}]}]}}"#,
    ] {
        assert!(matches!(check(raw), Err(RecoveryError::Malformed("duplicate_key"))), "{raw}");
    }
    for raw in [r#"{"a":{"a":1},"b":{"a":2}}"#, r#"{"a":{"x":1},"b":{"x":2},"x":3}"#, r#"[{},[],null,true,1,-1,0.5,"x"]"#] {
        assert!(check(raw).is_ok(), "{raw}");
    }
}

#[test]
fn canonical_base64_checks_every_pad_bit_and_real_context_only() {
    for (text, length) in [("", 0), ("AQ==", 1), ("AQI=", 2), ("AQID", 3), ("////", 3)] {
        assert_eq!(base64(text).unwrap(), length);
        let raw = format!("{{\"workbook\":{{\"vba_project\":\"{text}\"}}}}");
        assert_eq!(check(&raw).unwrap().binary_decoded, length);
    }
    for text in ["A", "AQ", "AQ=", "AQ===", "A===", "=AAA", "AA=A", "AQ==AAAA", "AR==", "AQJ=", "AA-_", "éAAA", "AA A", "AA\nA"] {
        assert!(base64(text).is_err(), "{text:?}");
    }
    let alphabet = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    for (index, byte) in alphabet.iter().enumerate() {
        assert_eq!(base64(&format!("A{}==", *byte as char)).is_ok(), index % 16 == 0);
        assert_eq!(base64(&format!("AA{}=", *byte as char)).is_ok(), index % 4 == 0);
    }
    assert!(check(r#"{"sidecars":{"workbook":{"macros_unread":[{"data":"not base64","vba_project":"?"}]}}}"#).is_ok());
    assert!(check(r#"{"workbook":{"vba_project":null}}"#).is_ok());
    assert!(matches!(check(r#"{"workbook":{"sheets":[{"images":[{"data":"AR=="}]}]}}"#), Err(RecoveryError::UnsupportedRepresentation("base64"))));
}

#[test]
fn visitor_recounts_shared_budgets_and_drops_all_scratch() {
    let raw = r#"{"workbook":{"sheets":[{"cells":{"bands":{"0":{"rows":{"0":{"0":{"value":{"t":"Text","v":"é"}}}}}}}}]},"sidecars":{"sheets":[{"spill":{"B1":null}}],"workbook":{"preserved":{"parts":[{"rels":[{}]}],"vml":null},"source_path":{"windows":[55296]},"cell_picture_groups":[0]}}}"#;
    let limits = RecoveryLimits::default();
    let lexical = super::super::lexical_audit(raw.as_bytes(), &limits).unwrap();
    let result = check(raw).unwrap();
    let seen = result.observed;
    assert_eq!(
        (
            seen.nodes,
            seen.keys,
            seen.decoded_bytes,
            seen.max_depth,
            seen.sheets,
            seen.cells,
            seen.parts,
            seen.relationships,
            seen.native_bytes,
            seen.groups
        ),
        (
            lexical.nodes,
            lexical.keys,
            lexical.decoded_bytes,
            lexical.max_depth,
            lexical.sheets,
            lexical.cells,
            lexical.parts,
            lexical.relationships,
            lexical.native_bytes,
            lexical.groups
        )
    );
    assert!(result.requests.calls > 0 && result.requests.peak > 0 && result.key_peak > 0);
    assert_eq!((result.requests.current, result.requests.actual_current), (0, 0));
    let deep = format!("{}0{}", "[".repeat(65), "]".repeat(65));
    assert!(matches!(check(&deep), Err(RecoveryError::TooLarge("depth"))));
    assert!(check(&format!("{}0{}", "[".repeat(64), "]".repeat(64))).is_ok());
    assert!(check(&format!("{}0{}", "{\"a\":".repeat(64), "}".repeat(64))).is_ok());
    assert!(matches!(check(&format!("{}0{}", "{\"a\":".repeat(65), "}".repeat(65))), Err(RecoveryError::TooLarge("depth"))));
    assert!(matches!(check("{}{}"), Err(RecoveryError::Malformed("manifest_syntax"))));
}

#[test]
fn actual_visitor_boundaries_match_lexical_counters_without_sharing_quota_state() {
    let fixtures = [
        (r#"{"workbook":{"sheets":[{},{}]},"sidecars":{"workbook":{"preserved":{"sheets":[{}]}}}}"#, "sheets"),
        (r#"{"sidecars":{"sheets":[{"spill":{"A1":null,"B1":null}}]}}"#, "cells"),
        (r#"{"sidecars":{"workbook":{"preserved":{"parts":[{}],"vml":{}}}}}"#, "parts"),
        (r#"{"sidecars":{"workbook":{"preserved":{"rels":[{},{}],"parts":[{"rels":[{}]}]}}}}"#, "relationships"),
        (r#"{"metadata":{"original_path_hint":{"windows":[55296,0]}}}"#, "native_units"),
    ];
    for (raw, reason) in fixtures {
        let observed = super::super::lexical_audit(raw.as_bytes(), &RecoveryLimits::default()).unwrap();
        let mut limits = RecoveryLimits::default();
        let bound = match reason {
            "sheets" => &mut limits.sheets,
            "cells" => &mut limits.cells,
            "parts" => &mut limits.parts,
            "relationships" => &mut limits.relationships,
            _ => &mut limits.native_units,
        };
        *bound = match reason {
            "sheets" => observed.sheets,
            "cells" => observed.cells,
            "parts" => observed.parts,
            "relationships" => observed.relationships,
            _ => observed.native_bytes,
        };
        assert!(audit(raw.as_bytes(), &limits).is_ok(), "{reason}");
        match reason {
            "sheets" => limits.sheets -= 1,
            "cells" => limits.cells -= 1,
            "parts" => limits.parts -= 1,
            "relationships" => limits.relationships -= 1,
            _ => limits.native_units -= 1,
        }
        assert!(matches!(audit(raw.as_bytes(), &limits), Err(RecoveryError::TooLarge(found)) if found == reason), "{reason}");
    }
    let owner = br#"{"sidecars":{"workbook":{"preserved":{"rels":[{},{}]}}}}"#;
    assert!(audit(owner, &RecoveryLimits { owner_relationships: 2, ..RecoveryLimits::default() }).is_ok());
    assert!(matches!(
        audit(owner, &RecoveryLimits { owner_relationships: 1, ..RecoveryLimits::default() }),
        Err(RecoveryError::TooLarge("owner_relationships"))
    ));
    let native = br#"{"metadata":{"original_path_hint":{"windows":[55296]}}}"#;
    assert_eq!(check(std::str::from_utf8(native).unwrap()).unwrap().observed.native_bytes, 2);
    for path in [
        r#"{"unix":[256]}"#,
        r#"{"windows":[65536]}"#,
        r#"{"unix":[-0]}"#,
        r#"{"unix":[1.0]}"#,
        r#"{"unix":[true]}"#,
        r#"{"unix":null}"#,
        r#"{}"#,
        r#"{"unix":[],"windows":[]}"#,
        r#"{"unknown":[]}"#,
    ] {
        assert!(check(&format!("{{\"metadata\":{{\"original_path_hint\":{path}}}}}")).is_err(), "{path}");
    }
    for group in ["[-1]", "[4294967296]", "[1.0]", "[null]", "[{}]"] {
        assert!(check(&format!("{{\"sidecars\":{{\"workbook\":{{\"cell_picture_groups\":{group}}}}}}}")).is_err());
    }
}

#[test]
fn string_nodes_and_metadata_are_checked_before_arena_copy_at_real_callbacks() {
    for (raw, limits, reason) in [
        (r#"{"abc":null}"#, RecoveryLimits { nodes: 1, ..RecoveryLimits::default() }, "nodes"),
        (r#"{"abc":null}"#, RecoveryLimits { manifest: 2, ..RecoveryLimits::default() }, "strings"),
        (r#"{"metadata":{"title":"éxx"}}"#, RecoveryLimits { metadata: 3, ..RecoveryLimits::default() }, "metadata"),
        (r#"{"sidecars":{"workbook":{"preserved":{"elements":[{"xml":"éxx"}]}}}}"#, RecoveryLimits { xml: 3, ..RecoveryLimits::default() }, "xml"),
    ] {
        assert!(matches!(audit(raw.as_bytes(), &limits), Err(RecoveryError::TooLarge(found)) if found == reason));
    }
    let raw = r#"{"abc":null}"#;
    let limits = RecoveryLimits { nodes: 3, manifest: 3, ..RecoveryLimits::default() };
    let stats = audit(raw.as_bytes(), &limits).unwrap(); // Direct visitor seam, not the lexical manifest-byte gate.
    assert_eq!((stats.observed.nodes, stats.observed.keys, stats.observed.decoded_bytes), (3, 1, 3));
    let long = "workbook".repeat(5000);
    assert!(check(&format!("{{\"{long}\":{{\"data\":\"?\"}}}} ")).is_ok());
}

#[test]
fn base64_occurrences_use_aggregate_bounds_without_opaque_part_policy() {
    let raw = br#"{"workbook":{"vba_project":"AQ==","sheets":[{"images":[{"data":"Ag=="}]}]}}"#;
    let exact = RecoveryLimits { envelope: 2, part_bytes: 0, ..RecoveryLimits::default() };
    let stats = audit(raw, &exact).unwrap();
    assert_eq!((stats.binary_encoded, stats.binary_decoded), (8, 2));
    assert!(matches!(audit(raw, &RecoveryLimits { envelope: 1, ..exact }), Err(RecoveryError::TooLarge("base64"))));
    assert!(check(r#"{"workbook":{"vba_project":"\u002fw=="}}"#).is_ok());
    for value in ["0", "true", "[]", "{}"] {
        assert!(check(&format!("{{\"workbook\":{{\"vba_project\":{value}}}}}")).is_err());
    }
}

#[test]
fn failure_unwinds_live_scopes_and_preserves_first_static_refusal() {
    let limits = RecoveryLimits::default();
    let raw = br#"{"workbook":{"sheets":[{"cell_pictures":[[{"row":0,"col":0},{}],[{"row":0,"col":1},{"x":0,"x":1}]]}]}}"#;
    let mut state = Audit { limits: &limits, stats: StructuralStats::default(), keys: Keys::default(), refusal: None };
    let mut parser = serde_json::Deserializer::from_slice(raw);
    let parsed = Seed { audit: &mut state, context: Context::ROOT, depth: 0, role: Role::Plain, parent_item: None }.deserialize(&mut parser);
    assert!(parsed.is_err());
    assert!(matches!(state.refusal, Some(RecoveryError::Malformed("duplicate_key"))));
    assert_eq!(state.keys.live(), (0, 0));
    assert!(state.stats.requests.current > 0 && state.stats.pictures == 1);
    let _: Result<(), serde_json::Error> = state.checked(Err(RecoveryError::TooLarge("later")));
    assert!(matches!(state.refusal, Some(RecoveryError::Malformed("duplicate_key"))));
    state.keys.close(&mut state.stats.requests);
    assert_eq!((state.stats.requests.current, state.stats.requests.actual_current), (0, 0));
    use std::io::Write;
    writeln!(std::io::stdout(), "C2_ERROR_REQUESTS {:?}", state.stats).unwrap();
}

#[test]
fn many_tiny_decoded_keys_use_real_bounded_requests_and_late_duplicate_detection() {
    let members = (0..256).map(|index| format!("\"k{index:03}\":null")).collect::<Vec<_>>().join(",");
    let wire = format!("{{{members}}}");
    let limits = RecoveryLimits { manifest: wire.len(), nodes: 513, ..RecoveryLimits::default() };
    let stats = audit(wire.as_bytes(), &limits).unwrap();
    assert_eq!((stats.observed.nodes, stats.observed.keys, stats.key_peak), (513, 256, 1024));
    assert_eq!((stats.requests.current, stats.requests.actual_current), (0, 0));
    assert!(stats.key_comparisons <= 256 * 8);
    use std::io::Write;
    writeln!(std::io::stdout(), "C2_TINY_REQUESTS {:?}", stats).unwrap();
    assert!(matches!(check(&format!("{{{members},\"k000\":null}}")), Err(RecoveryError::Malformed("duplicate_key"))));
}
