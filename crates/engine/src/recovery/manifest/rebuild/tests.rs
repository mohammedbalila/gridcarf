use super::super::{Manifest, observe_manifest, write};
use super::*;
use gridcraft_core::{RangeRef, Value};
use gridcraft_model::{Anchor, Cell, Color, Shape, ShapeFormat, ShapeKind, Style, StyleId, StyleTable};

fn metadata() -> Metadata {
    Metadata {
        title: "kept".into(),
        original_path_hint: Some(crate::recovery::NativePath::Unix(vec![0xff, b'x'])),
        file_kind: Some(crate::recovery::RecoveryFileKind::Xlsx),
        dirty_at_capture: true,
        import_issues: vec![],
        selection: crate::Selection::default(),
        sheet_selections: vec![crate::Selection::default(); 4],
        captured_revision: 17,
        id: "00000000000000000000000000000000".parse().unwrap(),
        sequence: 2,
        provenance: crate::recovery::RecoveryProvenance::Plain,
    }
}
fn formula(text: &str) -> Arc<Formula> {
    Arc::new(Formula { text: text.into(), expr: None, array: None, dynamic: false, eval_mode: gridcraft_model::FormulaEvalMode::Legacy })
}
fn typed_fixture() -> Workbook {
    let mut wb = Workbook::new();
    let mut style = Style::default();
    style.font.size = -0.0;
    wb.styles = serde_json::from_value(serde_json::json!({"styles":[style,style]})).unwrap();
    let sheet = Arc::make_mut(&mut wb.sheets[0]);
    sheet.file_id = 77;
    sheet.cells.set(CellRef::default(), Cell { value: Value::Number(42.0), formula: Some(formula("1+2")), ..Default::default() });
    sheet.cells.set(
        CellRef::new(64, 2),
        Cell {
            value: Value::Number(-0.0),
            formula: Some(Arc::new(Formula {
                text: "SUM(".into(),
                expr: None,
                array: Some(RangeRef::new(CellRef::new(64, 2), CellRef::new(65, 2))),
                dynamic: true,
                eval_mode: gridcraft_model::FormulaEvalMode::Legacy,
            })),
            ..Default::default()
        },
    );
    sheet.cells.set(CellRef::new(1024, 1), Cell { style: StyleId(1), ..Default::default() });
    sheet.cells.set(CellRef::new(3, 4), Cell { value: Value::Text("cache".into()), formula: Some(formula(&"x".repeat(8193))), ..Default::default() });
    sheet.shapes.push(Shape {
        id: 7,
        kind: ShapeKind::Rectangle,
        anchor: Anchor::default(),
        fill: Color::Auto,
        line: Color::Auto,
        text: "kept".into(),
        format: ShapeFormat { font_size: Some(-0.0), ..Default::default() },
        math: Default::default(),
    });
    wb
}

#[test]
fn wire_decoded_candidate_rebuilds_derived_only_and_keeps_fresh_uid_stable() {
    let limits = RecoveryLimits::default();
    let metadata = metadata();
    let source = typed_fixture();
    let encoded = write::manifest_bytes(&source, &metadata, &limits).unwrap();
    let mut decoded: Manifest = serde_json::from_slice(&encoded.bytes).unwrap();
    assert_ne!(source.sheets[0].uid.get(), decoded.workbook.sheets[0].uid.get());
    let fresh = decoded.workbook.sheets[0].uid.get();
    assert!(decoded.workbook.sheets[0].cells.get(CellRef::default()).unwrap().formula.as_ref().unwrap().expr.is_none());
    Arc::make_mut(&mut decoded.workbook.sheets[0]).file_id = 77; // Synthetic E3 sidecar attachment, not D3 reconstruction.
    let before = write::manifest_bytes(&decoded.workbook, &decoded.metadata, &limits).unwrap().bytes;
    let meta_before = serde_json::to_vec(&decoded.metadata).unwrap();
    let mut stats = Stats::default();
    let rebuilt = run(decoded.workbook, &decoded.metadata, &limits, &mut stats).unwrap();
    assert_eq!(write::manifest_bytes(&rebuilt, &decoded.metadata, &limits).unwrap().bytes, before);
    assert_eq!(serde_json::to_vec(&decoded.metadata).unwrap(), meta_before);
    assert_eq!(rebuilt.sheets[0].uid.get(), fresh);
    assert_eq!(rebuilt.sheets[0].file_id, 77);
    assert_eq!(decoded.metadata.sheet_selections.len(), 4);
    let cells = &rebuilt.sheets[0].cells;
    assert!(cells.get(CellRef::default()).unwrap().formula.as_ref().unwrap().expr.is_some());
    assert!(cells.get(CellRef::new(64, 2)).unwrap().formula.as_ref().unwrap().expr.is_none());
    assert!(cells.get(CellRef::new(3, 4)).unwrap().formula.as_ref().unwrap().expr.is_none());
    let Value::Number(value) = cells.get(CellRef::new(64, 2)).unwrap().value else { panic!("number") };
    assert_eq!(value.to_bits(), (-0.0f64).to_bits());
    assert_eq!(rebuilt.styles.get(StyleId(1)).font.size.to_bits(), (-0.0f32).to_bits());
    assert_eq!(rebuilt.sheets[0].shapes[0].format.font_size.unwrap().to_bits(), (-0.0f32).to_bits());
    let mut probe = rebuilt.styles.clone();
    let style = probe.get(StyleId(0)).clone();
    assert_eq!(probe.intern(style), StyleId(1));
    assert_eq!(probe.len(), 2);
    assert_eq!(
        (
            stats.model_before,
            stats.model_after,
            stats.store_calls,
            stats.style_calls,
            stats.formulas,
            stats.drops,
            stats.actual_before,
            stats.actual_after
        ),
        (1, 1, 1, 1, 3, 0, 4, 4)
    );
    use std::io::Write;
    writeln!(std::io::stdout(), "D3_REBUILD_COUNTS {:?}", stats).unwrap();
}

#[test]
fn malformed_private_stores_refuse_before_lossy_rebuild_or_detachment() {
    let value = serde_json::to_value(Cell { value: Value::Number(1.0), ..Default::default() }).unwrap();
    for store in [
        serde_json::json!({"len":0,"bands":{"0":{"rows":{}}}}),
        serde_json::json!({"len":0,"bands":{"0":{"rows":{"0":{}}}}}),
        serde_json::json!({"len":0,"bands":{"0":{"rows":{"0":{"0":value}}}}}),
        serde_json::json!({"len":1,"bands":{"1":{"rows":{"0":{"0":value}}}}}),
        serde_json::json!({"len":1,"bands":{"0":{"rows":{"0":{"0":Cell::default()}}}}}),
    ] {
        let mut source = Workbook::new();
        Arc::make_mut(&mut source.sheets[0]).cells = serde_json::from_value(store).unwrap();
        let candidate = source.clone();
        let before = serde_json::to_vec(&source).unwrap();
        let mut stats = Stats::default();
        assert!(matches!(run(candidate, &metadata(), &RecoveryLimits::default(), &mut stats), Err(RecoveryError::Malformed("model_storage"))));
        assert_eq!((stats.model_before, stats.sheet_detach, stats.store_calls, stats.style_calls), (1, 0, 0, 0));
        assert_eq!(serde_json::to_vec(&source).unwrap(), before);
    }
}

#[test]
fn real_d2_budget_and_pending_refusal_precede_model_and_clone() {
    let source = typed_fixture();
    let metadata = metadata();
    let count = observe_manifest(&source, &metadata, &RecoveryLimits::default(), false).unwrap().nodes;
    let mut stats = Stats::default();
    assert!(run(source.clone(), &metadata, &RecoveryLimits { nodes: count - 1, ..RecoveryLimits::default() }, &mut stats).is_err());
    assert_eq!((stats.d2_calls, stats.model_before, stats.sheet_detach, stats.store_calls), (1, 0, 0, 0));
    let candidate = super::super::rebuild_owned(source.clone(), &metadata, &RecoveryLimits { nodes: count, ..RecoveryLimits::default() }).unwrap();
    assert_eq!(candidate.sheets[0].cells.len(), 4);
    let mut pending = Workbook::new();
    pending.macros_read = vec![serde_json::json!({})];
    let mut stats = Stats::default();
    assert!(run(pending, &metadata, &RecoveryLimits::default(), &mut stats).is_err());
    assert_eq!(stats.model_before, 0);
}

#[test]
fn owned_isolation_uses_text_authority_without_walking_or_cloning_old_expr() {
    let mut source = typed_fixture();
    let old = Arc::new(gridcraft_formula::parse("7+8").unwrap());
    Arc::make_mut(&mut source.sheets[0]).cells.get_mut(CellRef::default()).unwrap().formula = Some(Arc::new(Formula {
        text: "1+2".into(),
        expr: Some(old.clone()),
        array: None,
        dynamic: false,
        eval_mode: gridcraft_model::FormulaEvalMode::Legacy,
    }));
    let before = write::manifest_bytes(&source, &metadata(), &RecoveryLimits::default()).unwrap().bytes;
    let source_sheet = source.sheets[0].clone();
    let out = super::super::rebuild_owned(source.clone(), &metadata(), &RecoveryLimits::default()).unwrap();
    assert!(Arc::ptr_eq(&source.sheets[0], &source_sheet));
    assert!(!Arc::ptr_eq(&out.sheets[0], &source_sheet));
    assert!(Arc::ptr_eq(source.sheets[0].cells.get(CellRef::default()).unwrap().formula.as_ref().unwrap().expr.as_ref().unwrap(), &old));
    assert_eq!(write::manifest_bytes(&source, &metadata(), &RecoveryLimits::default()).unwrap().bytes, before);
    assert_eq!(
        out.sheets[0].cells.get(CellRef::default()).unwrap().formula.as_ref().unwrap().expr.as_deref(),
        Some(&gridcraft_formula::parse("1+2").unwrap())
    );
}

#[test]
fn finite_model_errors_map_exhaustively_to_static_families() {
    use RecoveryModelError::*;
    for (error, reason) in [
        (Storage, "model_storage"),
        (Coordinate, "model_coordinate"),
        (Range, "model_range"),
        (Style, "model_style"),
        (Value, "model_value"),
        (Array, "model_array"),
        (CellOwnership, "model_cell_ownership"),
        (Spill, "model_spill"),
        (Workbook, "model_workbook"),
        (Geometry, "model_geometry"),
        (Table, "model_table"),
        (Filter, "model_filter"),
        (WhatIf, "model_what_if"),
        (Pivot, "model_pivot"),
        (Control, "model_control"),
        (Print, "model_print"),
        (Phonetic, "model_phonetic"),
        (Python, "model_python"),
    ] {
        assert!(matches!(model_error(error),RecoveryError::Malformed(actual)if actual==reason));
    }
    assert!(matches!(model_error(Capacity), RecoveryError::TooLarge("model_allocation")));
    assert!(matches!(
        model_error(UnsupportedRepresentation(RecoveryRepresentation::AmbiguousIdentity)),
        RecoveryError::UnsupportedRepresentation("model_identity")
    ));
    assert!(matches!(
        model_error(UnsupportedRepresentation(RecoveryRepresentation::OverlappingRectangles)),
        RecoveryError::UnsupportedRepresentation("model_rectangles")
    ));
}

#[test]
fn duplicate_placement_grid_and_bad_style_refuse_before_derived_helpers() {
    let value = serde_json::to_value(Cell { value: Value::Number(1.0), ..Default::default() }).unwrap();
    for store in [
        serde_json::json!({"len":2,"bands":{"0":{"rows":{"0":{"0":value}}},"1":{"rows":{"0":{"0":value}}}}}),
        serde_json::json!({"len":1,"bands":{"16384":{"rows":{"1048576":{"0":value}}}}}),
        serde_json::json!({"len":usize::MAX,"bands":{}}),
    ] {
        let mut source = Workbook::new();
        Arc::make_mut(&mut source.sheets[0]).cells = serde_json::from_value(store).unwrap();
        let mut stats = Stats::default();
        assert!(matches!(run(source.clone(), &metadata(), &RecoveryLimits::default(), &mut stats), Err(RecoveryError::Malformed(_))));
        assert_eq!((stats.model_before, stats.store_calls, stats.style_calls, stats.sheet_detach), (1, 0, 0, 0));
    }
    let mut source = typed_fixture();
    Arc::make_mut(&mut source.sheets[0]).cells.get_mut(CellRef::default()).unwrap().style = StyleId(999);
    let mut stats = Stats::default();
    assert!(matches!(run(source, &metadata(), &RecoveryLimits::default(), &mut stats), Err(RecoveryError::Malformed("model_style"))));
    assert_eq!((stats.store_calls, stats.style_calls), (0, 0));
}

#[test]
fn ordered_style_duplicates_and_finite_font_bits_survive_without_repair() {
    for size in [-2.5f32, -0.0, 0.0] {
        let mut candidate = typed_fixture();
        let mut style = Style::default();
        style.font.size = size;
        let table: StyleTable = serde_json::from_value(serde_json::json!({"styles":[style,style]})).unwrap();
        candidate.styles = table;
        Arc::make_mut(&mut candidate.sheets[0]).shapes[0].format.font_size = Some(size);
        let out = super::super::rebuild_owned(candidate, &metadata(), &RecoveryLimits::default()).unwrap();
        assert_eq!(out.styles.len(), 2);
        for id in [StyleId(0), StyleId(1)] {
            assert_eq!(out.styles.get(id).font.size.to_bits(), size.to_bits());
        }
        assert_eq!(out.sheets[0].shapes[0].format.font_size.unwrap().to_bits(), size.to_bits());
        assert_eq!(out.sheets[0].cells.get(CellRef::new(1024, 1)).unwrap().style, StyleId(1));
        let mut probe = out.styles.clone();
        assert_eq!(probe.intern(probe.get(StyleId(0)).clone()), StyleId(1));
        assert_eq!(probe.len(), 2);
    }
}

#[test]
fn python_spills_names_preserved_arcs_and_native_paths_are_unchanged() {
    use gridcraft_model::{DefinedName, Preserved, PreservedPart, PyCode, PyOutput};
    let mut source = typed_fixture();
    let python = Arc::new(PyCode {
        code: "x\r\ny\rz".into(),
        object: true,
        stale: true,
        output: Some(PyOutput {
            value: Value::Text("output".into()),
            type_name: "object".into(),
            preview: "untrimmed preview".into(),
            convertible: false,
        }),
    });
    let bytes: Arc<[u8]> = Arc::from([1, 2, 3]);
    let preserved = Arc::new(Preserved {
        parts: vec![PreservedPart { name: "retained.bin".into(), content_type: "application/x-inert".into(), data: bytes.clone(), rels: vec![] }],
        ..Default::default()
    });
    source.preserved = Some(preserved.clone());
    source.source_dir = Some("/kept-dir".into());
    source.source_path = Some("/kept-file.xlsx".into());
    source.names.push(DefinedName {
        name: "_gc_macro_0_0".into(),
        scope: None,
        formula: "kept".into(),
        comment: "not imported".into(),
        hidden: true,
    });
    let sheet = Arc::make_mut(&mut source.sheets[0]);
    sheet.preserved = Some(preserved.clone());
    sheet.cells.set(CellRef::new(2, 2), Cell { value: Value::Number(99.0), py: Some(python.clone()), ..Default::default() });
    sheet.spill.insert(CellRef::new(65, 2), Value::Text("spill cache".into()));
    sheet.spill_ranges.insert(CellRef::new(64, 2), RangeRef::new(CellRef::new(64, 2), CellRef::new(65, 2)));
    let metadata = metadata();
    let before = write::manifest_bytes(&source, &metadata, &RecoveryLimits::default()).unwrap().bytes;
    let out = super::super::rebuild_owned(source.clone(), &metadata, &RecoveryLimits::default()).unwrap();
    assert_eq!(write::manifest_bytes(&out, &metadata, &RecoveryLimits::default()).unwrap().bytes, before);
    assert_eq!(write::manifest_bytes(&source, &metadata, &RecoveryLimits::default()).unwrap().bytes, before);
    assert!(Arc::ptr_eq(out.preserved.as_ref().unwrap(), &preserved));
    assert!(Arc::ptr_eq(out.sheets[0].preserved.as_ref().unwrap(), &preserved));
    assert!(Arc::ptr_eq(&out.preserved.as_ref().unwrap().parts[0].data, &bytes));
    assert!(Arc::ptr_eq(out.sheets[0].cells.get(CellRef::new(2, 2)).unwrap().py.as_ref().unwrap(), &python));
    assert_eq!(out.source_path, source.source_path);
    assert_eq!(out.source_dir, source.source_dir);
    assert_eq!(out.names, source.names);
}
