use super::*;
use crate::recovery::{RecoveryError, RecoveryLimits};

fn audit(raw: &str) -> AuditStats {
    lexical_audit(raw.as_bytes(), &RecoveryLimits::default()).unwrap()
}
fn small(raw: &str, limits: RecoveryLimits, reason: &str) {
    assert!(matches!(lexical_audit(raw.as_bytes(), &limits), Err(RecoveryError::TooLarge(found)) if found == reason), "{raw}");
}

#[test]
fn literal_counts_every_value_key_and_real_container_depth() {
    let stats = audit(r#"{"a":[null,{},[]],"":1}"#);
    assert_eq!((stats.nodes, stats.keys, stats.max_depth, stats.decoded_bytes), (8, 2, 3, 1));
    assert_eq!((stats.number_bytes, stats.max_number), (1, 1));
    assert_eq!((audit("null").nodes, audit("null").max_depth), (1, 0));
    small(r#"{"a":null}"#, RecoveryLimits { nodes: 2, ..RecoveryLimits::default() }, "nodes");
    assert_eq!(lexical_audit(br#"{"a":null}"#, &RecoveryLimits { nodes: 3, ..RecoveryLimits::default() }).unwrap().nodes, 3);
    let wire = format!("{}0{}", "[".repeat(64), "]".repeat(64));
    assert_eq!((audit(&wire).nodes, audit(&wire).max_depth), (65, 64));
    small(&format!("[{wire}]"), RecoveryLimits::default(), "depth");
}

#[test]
fn decodes_escapes_utf8_and_surrogate_pairs_without_counting_string_brackets() {
    assert_eq!(audit(r#""\uD83D\uDE00""#).decoded_bytes, 4);
    assert_eq!(audit("\"é中\"").decoded_bytes, 5);
    let stats = audit(r#"{"\u0061":"\"[]{}\\\b\f\n\r\t\/"}"#);
    assert_eq!((stats.nodes, stats.keys, stats.max_depth), (3, 1, 1));
    assert_eq!(audit(r#"{"a":1,"\u0061":2}"#).keys, 2); // C2 must detect decoded duplicates.
    for raw in [
        r#""\uD800""#,
        r#""\uDC00""#,
        r#""\uD800\u0041""#,
        r#""\q""#,
        r#""\u000x""#,
        "\"\n\"",
        "[1,]",
        "{\"a\":1,}",
        "{}{}",
        "[",
        "",
        "{\"a\" 0}",
        "[01]",
    ] {
        assert!(matches!(lexical_audit(raw.as_bytes(), &RecoveryLimits::default()), Err(RecoveryError::Malformed(_))), "{raw}");
    }
    for bytes in [&b"\"\xff\""[..], &b"\"\xc0\x80\""[..], &b"\"\xed\xa0\x80\""[..]] {
        assert!(matches!(lexical_audit(bytes, &RecoveryLimits::default()), Err(RecoveryError::Malformed(_))));
    }
}

#[test]
fn long_number_tokens_have_only_the_actual_manifest_budget() {
    for number in [format!("0.{}", "1".repeat(300)), format!("1e+{}", "9".repeat(1000))] {
        let stats = audit(&number);
        assert_eq!((stats.nodes, stats.max_number, stats.number_bytes), (1, number.len(), number.len()));
        small(&number, RecoveryLimits { manifest: number.len() - 1, ..RecoveryLimits::default() }, "manifest");
    }
    for number in ["-", "1.", "1e", "1e+", "+1", "00", "-.1", "1true"] {
        assert!(lexical_audit(number.as_bytes(), &RecoveryLimits::default()).is_err(), "{number}");
    }
}

fn text_fixture(prefix: &str, suffix: &str, text: &str) -> String {
    format!("{prefix}\"{text}\"{suffix}")
}

#[test]
fn metadata_and_xml_count_decoded_bytes_at_their_actual_routes() {
    for (prefix, suffix) in [
        (r#"{"metadata":{"title":"#, "}}"),
        (r#"{"workbook":{"props":{"author":"#, "}}}"),
        (r#"{"workbook":{"sheets":[{"name":"#, "}]}}"),
        (r#"{"sidecars":{"workbook":{"preserved":{"parts":[{"content_type":"#, "}]}}}}"),
    ] {
        let limits = RecoveryLimits { metadata: 4, ..RecoveryLimits::default() };
        assert!(lexical_audit(text_fixture(prefix, suffix, "abcd").as_bytes(), &limits).is_ok());
        small(&text_fixture(prefix, suffix, "abcde"), limits, "metadata");
    }
    let raw = r#"{"metadata":{"t\u0069tle":"\u20acx"}}"#;
    assert!(lexical_audit(raw.as_bytes(), &RecoveryLimits { metadata: 4, ..RecoveryLimits::default() }).is_ok());
    small(raw, RecoveryLimits { metadata: 3, ..RecoveryLimits::default() }, "metadata");
    let raw = r#"{"sidecars":{"workbook":{"preserved":{"elements":[{"xml":"\u20acx"}]}}}}"#;
    assert!(lexical_audit(raw.as_bytes(), &RecoveryLimits { xml: 4, ..RecoveryLimits::default() }).is_ok());
    small(raw, RecoveryLimits { xml: 3, ..RecoveryLimits::default() }, "xml");
}

#[test]
fn general_is_absorbing_even_for_metadata_or_map_looking_keys() {
    let long = "a".repeat(32769);
    for (prefix, suffix) in [
        (r#"{"sidecars":{"workbook":{"macros_unread":[{"workbook":{"props":{"author":"#, "}}}]}}}"),
        (r#"{"workbook":{"macros":[{"steps":[{"params":{"rows":{"01":{"title":"#, "}}}}]}]}}"),
        (r#"{"workbook":{"sheets":[{"controls":[{"action":{"params":{"source_path":{"unix":"#, "}}}}]}]}}"),
        (r#"{"workbook":{"sheets":[{"cells":{"bands":{"0":{"rows":{"0":{"0":{"value":{"t":"Text","v":"#, "}}}}}}}}]}}"),
        (r#"{"workbook":{"sheets":[{"comments":{"A1":{"text":"#, "}}}]}}"),
    ] {
        assert!(lexical_audit(text_fixture(prefix, suffix, &long).as_bytes(), &RecoveryLimits::default()).is_ok());
    }
}

#[test]
fn native_units_and_picture_group_scalars_have_exact_shapes_and_ranges() {
    let unix = r#"{"sidecars":{"workbook":{"source_dir":{"unix":[255,0]}}}}"#;
    assert_eq!(lexical_audit(unix.as_bytes(), &RecoveryLimits { native_units: 2, ..RecoveryLimits::default() }).unwrap().native_bytes, 2);
    small(unix, RecoveryLimits { native_units: 1, ..RecoveryLimits::default() }, "native_units");
    let windows = r#"{"metadata":{"original_path_hint":{"windows":[55296]}}}"#;
    assert_eq!(lexical_audit(windows.as_bytes(), &RecoveryLimits { native_units: 2, ..RecoveryLimits::default() }).unwrap().native_bytes, 2);
    small(windows, RecoveryLimits { native_units: 1, ..RecoveryLimits::default() }, "native_units");
    for path in [
        r#"{"unix":[256]}"#,
        r#"{"windows":[65536]}"#,
        r#"{"unix":[-1]}"#,
        r#"{"unix":[1.0]}"#,
        r#"{"unix":["0"]}"#,
        r#"{"unix":null}"#,
        r#"{"unix":[[]]}"#,
        r#"{}"#,
        r#"{"unix":[],"windows":[]}"#,
        r#"{"other":[]}"#,
    ] {
        assert!(
            lexical_audit(format!("{{\"metadata\":{{\"original_path_hint\":{path}}}}}").as_bytes(), &RecoveryLimits::default()).is_err(),
            "{path}"
        );
    }
    assert_eq!(audit(r#"{"sidecars":{"workbook":{"cell_picture_groups":[0,1,0,4294967295]}}}"#).groups, 4);
    for groups in ["null", "[-1]", "[4294967296]", "[1.0]", "[true]", "[\"0\"]", "[{}]", "[[]]"] {
        assert!(
            lexical_audit(format!("{{\"sidecars\":{{\"workbook\":{{\"cell_picture_groups\":{groups}}}}}}}").as_bytes(), &RecoveryLimits::default())
                .is_err(),
            "{groups}"
        );
    }
}

#[test]
fn actual_collection_entries_charge_combined_budgets_and_owner_local_relationships() {
    let sheets = r#"{"workbook":{"sheets":[{},{}]},"sidecars":{"workbook":{"preserved":{"sheets":[{}]}}}}"#;
    assert_eq!(audit(sheets).sheets, 3);
    small(sheets, RecoveryLimits { sheets: 2, ..RecoveryLimits::default() }, "sheets");
    let cells =
        r#"{"workbook":{"sheets":[{"cells":{"len":0,"bands":{"0":{"rows":{"0":{"0":null}}}}}}]},"sidecars":{"sheets":[{"spill":{"B1":null}}]}}"#;
    assert_eq!(audit(cells).cells, 2);
    small(cells, RecoveryLimits { cells: 1, ..RecoveryLimits::default() }, "cells");
    let parts = r#"{"sidecars":{"workbook":{"preserved":{"parts":[{}],"vml":{}}},"sheets":[{"preserved":{"parts":[{}],"vml":null}}]}}"#;
    assert_eq!(audit(parts).parts, 3);
    small(parts, RecoveryLimits { parts: 2, ..RecoveryLimits::default() }, "parts");
    let rels = r#"{"sidecars":{"workbook":{"preserved":{"package_rels":[{},{}],"rels":[{},{}],"parts":[{"rels":[{},{}]}]}}}}"#;
    assert_eq!(audit(rels).relationships, 6);
    assert!(lexical_audit(rels.as_bytes(), &RecoveryLimits { owner_relationships: 2, relationships: 6, ..RecoveryLimits::default() }).is_ok());
    small(rels, RecoveryLimits { owner_relationships: 1, ..RecoveryLimits::default() }, "owner_relationships");
    small(rels, RecoveryLimits { relationships: 5, ..RecoveryLimits::default() }, "relationships");
}

fn at(path: &str, literal: &str) -> String {
    path.rsplit('.').fold(literal.to_owned(), |value, field| match field {
        "#0" => format!("[{value}]"),
        "#1" => format!("[null,{value}]"),
        _ => format!("{{\"{field}\":{value}}}"),
    })
}

#[test]
fn all_positive_metadata_families_use_the_same_closed_classification() {
    for path in [
        "workbook.names.#0.name",
        "workbook.props.title",
        "workbook.props.subject",
        "workbook.props.author",
        "workbook.props.company",
        "workbook.props.keywords",
        "workbook.props.description",
        "workbook.sheets.#0.name",
        "workbook.theme.name",
        "workbook.theme.major_font",
        "workbook.theme.minor_font",
        "workbook.theme.major_font_ea",
        "workbook.theme.minor_font_ea",
        "workbook.theme.major_font_cs",
        "workbook.theme.minor_font_cs",
        "workbook.theme.major_font_scripts.#0.#0",
        "workbook.theme.major_font_scripts.#0.#1",
        "workbook.theme.minor_font_scripts.#0.#0",
        "workbook.theme.minor_font_scripts.#0.#1",
        "workbook.styles.styles.#0.font.name",
        "workbook.styles.styles.#0.dxf_stated.scheme",
        "workbook.cell_styles.#0.#0",
        "workbook.cell_styles.#0.#1.font.name",
        "workbook.phonetics.#0.props.kind",
        "workbook.phonetics.#0.props.alignment",
        "workbook.sheets.#0.tables.#0.name",
        "workbook.sheets.#0.tables.#0.style",
        "workbook.sheets.#0.tables.#0.table_type",
        "workbook.sheets.#0.tables.#0.columns.#0.name",
        "workbook.sheets.#0.tables.#0.columns.#0.unique_name",
        "workbook.sheets.#0.tables.#0.columns.#0.xml.data_type",
        "workbook.sheets.#0.tables.#0.filter.#0.#1.Icon.set",
        "workbook.sheets.#0.autofilter.criteria.#0.#1.Icon.set",
        "workbook.sheets.#0.cond_formats.#0.rule.TimePeriod.period",
        "workbook.sheets.#0.cond_formats.#0.rule.TimePeriod.style.font.name",
        "workbook.sheets.#0.cond_formats.#0.rule.IconSet.set",
        "workbook.sheets.#0.cond_formats.#0.rule.CellIs.style.dxf_stated.scheme",
        "workbook.sheets.#0.comments.A1.author",
        "workbook.sheets.#0.comments.A1.replies.#0.#0",
        "workbook.sheets.#0.comments.A1.dates.#0",
        "workbook.sheets.#0.hyperlinks.A1.target",
        "workbook.sheets.#0.charts.#0.pivot",
        "workbook.sheets.#0.images.#0.mime",
        "workbook.sheets.#0.cell_pictures.#0.#1.mime",
        "workbook.sheets.#0.shapes.#0.format.font",
        "workbook.sheets.#0.controls.#0.name",
        "workbook.sheets.#0.controls.#0.action.name",
        "workbook.sheets.#0.controls.#0.action.id",
        "workbook.sheets.#0.slicers.#0.table",
        "workbook.sheets.#0.slicers.#0.column",
        "workbook.sheets.#0.pivots.#0.name",
        "workbook.sheets.#0.pivots.#0.style",
        "workbook.sheets.#0.pivots.#0.rows.#0.source_col",
        "workbook.sheets.#0.pivots.#0.columns.#0.source_col",
        "workbook.sheets.#0.pivots.#0.values.#0.source_col",
        "workbook.sheets.#0.pivots.#0.filters.#0.source_col",
        "workbook.sheets.#0.pivots.#0.calculated_fields.#0.name",
        "workbook.sheets.#0.print.paper",
        "workbook.sheets.#0.edit_ranges.#0.title",
        "workbook.structure_password.Iterated.algorithm",
        "workbook.sheets.#0.protection.password.Iterated.algorithm",
        "workbook.sheets.#0.edit_ranges.#0.password.Iterated.algorithm",
        "workbook.sheets.#0.cells.bands.0.rows.0.0.py.output.typeName",
        "workbook.macros.#0.name",
        "workbook.macros.#0.shortcut",
        "workbook.macros.#0.inputs.#0.name",
        "workbook.macros.#0.permissions.#0",
        "workbook.macros.#0.steps.#0.id",
        "workbook.macros.#0.steps.#0.body.#0.then.#0.else.#0.id",
        "metadata.title",
        "metadata.import_issues.#0.code",
        "metadata.import_issues.#0.feature",
        "metadata.import_issues.#0.location",
        "metadata.import_issues.#0.description",
        "sidecars.workbook.preserved.rels.#0.id",
        "sidecars.workbook.preserved.package_rels.#0.rel_type",
        "sidecars.workbook.preserved.parts.#0.rels.#0.target",
        "sidecars.workbook.preserved.parts.#0.name",
        "sidecars.workbook.preserved.vml.content_type",
        "sidecars.workbook.preserved.elements.#0.slot",
        "sidecars.workbook.preserved.sheets.#0.name",
        "sidecars.workbook.preserved.sheets.#0.rel_id",
    ] {
        let limits = RecoveryLimits { metadata: 4, ..RecoveryLimits::default() };
        assert!(lexical_audit(at(path, "\"four\"").as_bytes(), &limits).is_ok(), "{path}");
        assert!(matches!(lexical_audit(at(path, "\"fives\"").as_bytes(), &limits), Err(RecoveryError::TooLarge("metadata"))), "{path}");
    }
}

#[test]
fn content_strings_and_unknown_children_do_not_inherit_metadata_or_binary_rules() {
    let literal = format!("\"{}\"", "x".repeat(32769));
    for path in [
        "workbook.names.#0.formula",
        "workbook.names.#0.comment",
        "workbook.styles.styles.#0.num_fmt",
        "workbook.phonetics.#0.text",
        "workbook.phonetics.#0.runs.#0.text",
        "workbook.sheets.#0.tables.#0.columns.#0.totals_label",
        "workbook.sheets.#0.tables.#0.columns.#0.formula",
        "workbook.sheets.#0.tables.#0.columns.#0.xml.xpath",
        "workbook.sheets.#0.what_if_tables.#0.formula_text",
        "workbook.sheets.#0.validations.#0.input_title",
        "workbook.sheets.#0.comments.A1.text",
        "workbook.sheets.#0.comments.A1.replies.#0.#1",
        "workbook.sheets.#0.hyperlinks.A1.tooltip",
        "workbook.sheets.#0.charts.#0.title",
        "workbook.sheets.#0.charts.#0.series.#0.name",
        "workbook.sheets.#0.charts.#0.series.#0.trendlines.#0.name",
        "workbook.sheets.#0.images.#0.alt",
        "workbook.sheets.#0.shapes.#0.text",
        "workbook.sheets.#0.controls.#0.text",
        "workbook.sheets.#0.controls.#0.action.output",
        "workbook.sheets.#0.slicers.#0.caption",
        "workbook.sheets.#0.pivots.#0.values.#0.name",
        "workbook.sheets.#0.print.header",
        "workbook.sheets.#0.edit_ranges.#0.security_descriptor",
        "workbook.structure_password.Iterated.hash",
        "workbook.structure_password.Iterated.salt",
        "workbook.sheets.#0.cells.bands.0.rows.0.0.py.output.preview",
        "workbook.sheets.#0.cells.bands.0.rows.0.0.formula.text",
        "workbook.macros.#0.steps.#0.note",
        "workbook.macros.#0.steps.#0.value.title",
        "sidecars.workbook.macros_unread.#0.name",
        "workbook.props.unknown.author",
        "metadata.import_issues.#0.target",
        "sidecars.workbook.preserved.rels.#0.description",
    ] {
        assert!(lexical_audit(at(path, &literal).as_bytes(), &RecoveryLimits::default()).is_ok(), "{path}");
    }
    // Known b64 strings are classified but C2 owns canonical padding/alphabet/pad-bit checks.
    assert!(lexical_audit(at("workbook.vba_project", &literal).as_bytes(), &RecoveryLimits::default()).is_ok());
}

#[test]
fn rejection_counters_stop_before_entering_forbidden_children() {
    let raw = format!("{}0{}", "[".repeat(65), "]".repeat(65));
    let limits = RecoveryLimits::default();
    let mut scanner = Scanner::new(raw.as_bytes(), &limits);
    assert!(matches!(scanner.run(), Err(RecoveryError::TooLarge("depth"))));
    assert_eq!((scanner.depth, scanner.stats.max_depth, scanner.stats.nodes), (64, 64, 65));
    assert_eq!(scanner.pos, 64);
    let limits = RecoveryLimits { cells: 0, ..RecoveryLimits::default() };
    let raw = at("sidecars.sheets.#0.spill.A1", "[0,1,2]");
    let mut scanner = Scanner::new(raw.as_bytes(), &limits);
    assert!(matches!(scanner.run(), Err(RecoveryError::TooLarge("cells"))));
    assert_eq!(scanner.stats.cells, 0);
    assert_eq!(scanner.peek(), Some(b'['));
}

#[test]
fn actual_e1_borrowed_projection_has_matching_structural_counts() {
    use super::super::{ManifestRef, Metadata};
    use crate::{Selection, recovery::RecoveryProvenance};
    use std::sync::Arc;
    let mut workbook = gridcraft_model::Workbook::new();
    let python =
        gridcraft_model::PyCode { output: Some(gridcraft_model::PyOutput { type_name: "str".into(), ..Default::default() }), ..Default::default() };
    Arc::make_mut(&mut workbook.sheets[0])
        .cells
        .set(gridcraft_core::CellRef::default(), gridcraft_model::Cell { py: Some(Arc::new(python)), ..Default::default() });
    workbook.macros_unread = Arc::new(vec![serde_json::json!({"v":2,"data":"not base64","rows":{"01":"content"}})]);
    let metadata = Metadata {
        title: "Synthetic".into(),
        original_path_hint: None,
        file_kind: None,
        dirty_at_capture: true,
        import_issues: vec![],
        selection: Selection::default(),
        sheet_selections: vec![Selection::default(); 3],
        captured_revision: 0,
        id: "00000000000000000000000000000000".parse().unwrap(),
        sequence: 0,
        provenance: RecoveryProvenance::Plain,
    };
    let limits = RecoveryLimits::default();
    let wire = serde_json::to_string(&ManifestRef::new(&workbook, &metadata, &limits).unwrap()).unwrap();
    let observed = lexical_audit(wire.as_bytes(), &limits).unwrap();
    fn shape(value: &serde_json::Value, depth: usize) -> (usize, usize, usize, usize) {
        let mut result = (1, 0, depth, 0);
        match value {
            serde_json::Value::Object(map) => {
                result.0 += map.len();
                result.1 += map.len();
                result.2 += 1;
                for (key, value) in map {
                    let child = shape(value, depth + 1);
                    result.0 += child.0;
                    result.1 += child.1;
                    result.2 = result.2.max(child.2);
                    result.3 += key.len() + child.3;
                }
            }
            serde_json::Value::Array(values) => {
                result.2 += 1;
                for value in values {
                    let child = shape(value, depth + 1);
                    result.0 += child.0;
                    result.1 += child.1;
                    result.2 = result.2.max(child.2);
                    result.3 += child.3;
                }
            }
            serde_json::Value::String(text) => result.3 = text.len(),
            _ => {}
        }
        result
    }
    let counted = shape(&serde_json::from_str(&wire).unwrap(), 0);
    assert_eq!((observed.nodes, observed.keys, observed.max_depth, observed.decoded_bytes), counted);
    assert_eq!((observed.sheets, observed.cells, observed.groups), (1, 1, 0));
    let streamed = duplicates::audit(wire.as_bytes(), &limits).unwrap().observed;
    assert_eq!((streamed.nodes, streamed.keys, streamed.max_depth, streamed.decoded_bytes), counted);
    assert_eq!((streamed.sheets, streamed.cells, streamed.groups), (1, 1, 0));
    preflight_manifest(wire.as_bytes(), &limits).unwrap();
}

#[test]
fn known_model_map_keys_refuse_nonwriter_aliases_without_reclassifying_general_json() {
    for key in [r#""01""#, r#""+1""#, r#""-0""#, r#""\u0030""#] {
        let raw = at("workbook.sheets.#0.cells.bands", &format!("{{{key}:null}}"));
        assert!(
            matches!(lexical_audit(raw.as_bytes(), &RecoveryLimits::default()), Err(RecoveryError::UnsupportedRepresentation("numeric_key"))),
            "{key}"
        );
    }
    let raw = at("workbook.sheets.#0.cells.bands", r#"{"4294967296":null}"#);
    assert!(matches!(lexical_audit(raw.as_bytes(), &RecoveryLimits::default()), Err(RecoveryError::Malformed("numeric_key"))));
    for key in ["a1", "$A$1", "A01", "A0", "XFE1", "A1048577"] {
        let raw = at("workbook.sheets.#0.comments", &format!("{{\"{key}\":null}}"));
        assert!(lexical_audit(raw.as_bytes(), &RecoveryLimits::default()).is_err(), "{key}");
    }
    for (path, literal) in [
        ("workbook.sheets.#0.cells.bands", r#"{"0":null,"4294967295":null}"#),
        ("workbook.sheets.#0.comments", r#"{"A1":null,"XFD1048576":null}"#),
        ("workbook.sheets.#0.hyperlinks", r#"{"\u0041\u0031":null}"#),
        ("sidecars.workbook.macros_unread.#0.rows", r#"{"01":null,"$A$1":null}"#),
    ] {
        assert!(lexical_audit(at(path, literal).as_bytes(), &RecoveryLimits::default()).is_ok());
    }
}

#[test]
fn long_arbitrary_keys_have_no_fixed_label_or_metadata_cap() {
    let key = format!("workbook{}", "x".repeat(32769));
    let raw = format!("{{\"{key}\":{{\"props\":{{\"author\":\"{}\"}}}}}}", "y".repeat(32769));
    let stats = lexical_audit(raw.as_bytes(), &RecoveryLimits { metadata: 1, ..RecoveryLimits::default() }).unwrap();
    assert_eq!(stats.max_string, key.len());
    assert!(stats.decoded_bytes > 65536);
}

#[test]
fn composition_preserves_stage_order_independent_quotas_and_finite_parser_errors() {
    let limits = RecoveryLimits { nodes: 3, ..RecoveryLimits::default() };
    assert!(preflight_manifest(br#"{"x":0}"#, &limits).is_ok());
    assert!(matches!(preflight_manifest(br#"{"x":0}"#, &RecoveryLimits { nodes: 2, ..limits }), Err(RecoveryError::TooLarge("nodes"))));
    assert!(matches!(preflight_manifest(br#"{"x":0,"\u0078":1}"#, &RecoveryLimits::default()), Err(RecoveryError::Malformed("duplicate_key"))));
    let numeric_alias = at("workbook.sheets.#0.cells.bands", r#"{"01":null,"01":null}"#);
    assert!(matches!(preflight_manifest(numeric_alias.as_bytes(), &limits), Err(RecoveryError::TooLarge("nodes"))));
    assert!(matches!(
        preflight_manifest(numeric_alias.as_bytes(), &RecoveryLimits::default()),
        Err(RecoveryError::UnsupportedRepresentation("numeric_key"))
    ));
    let long = format!("1e+{}", "9".repeat(1000));
    assert_eq!(lexical_audit(long.as_bytes(), &RecoveryLimits::default()).unwrap().max_number, long.len());
    assert!(matches!(preflight_manifest(long.as_bytes(), &RecoveryLimits::default()), Err(RecoveryError::Malformed("manifest_syntax"))));
    assert!(preflight_manifest(format!("0.{}", "1".repeat(300)).as_bytes(), &RecoveryLimits::default()).is_ok());
    assert!(preflight_manifest(b"{}{}", &RecoveryLimits::default()).is_err());
    for (path, keys) in
        [("workbook.sheets.#0.cells.bands", r#"{"0":null,"0":null}"#), ("workbook.sheets.#0.comments", r#"{"A1":null,"\u0041\u0031":null}"#)]
    {
        assert!(matches!(preflight_manifest(at(path, keys).as_bytes(), &RecoveryLimits::default()), Err(RecoveryError::Malformed("duplicate_key"))));
    }
}
