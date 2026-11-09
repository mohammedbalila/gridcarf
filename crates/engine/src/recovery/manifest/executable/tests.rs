use super::*;
use std::sync::Arc;
fn metadata() -> Metadata {
    Metadata {
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
    }
}
fn current(name: &str) -> Macro {
    Macro { name: name.into(), ..Default::default() }
}
fn run_book(wb: &Workbook, decode: bool) -> Result<Staged, RecoveryError> {
    super::super::check_executable(wb, &metadata(), &RecoveryLimits::default(), decode)
}

#[test]
fn supported_pending_btree_order_legacy_zero_and_typed_roundtrips_are_exact() {
    let mut wb = Workbook::new();
    let mut m = current("M");
    m.v = 0;
    m.steps = vec![Step::command("x", serde_json::json!({"n":-0.0}))];
    wb.macros = vec![m.clone()].into();
    assert!(run_book(&wb, false).unwrap().adopted.is_none());
    let raw = serde_json::to_value(&m).unwrap();
    wb.macros = Default::default();
    wb.macros_read = vec![raw.clone()];
    let staged = run_book(&wb, true).unwrap();
    assert_eq!(staged.adopted.as_ref().unwrap()[0], m);
    assert_eq!(wb.macros_read, [raw]);
    assert!(wb.macros.is_empty());
    assert_eq!(staged.stats.parse_entries, 1);
    assert!(staged.stats.count_passes > 0);
}
#[test]
fn strict_adoption_refuses_trim_shortform_defaults_unknown_and_wrong_future_channel() {
    let mut wb = Workbook::new();
    let good = serde_json::to_value(current("M")).unwrap();
    let mut trim = good.clone();
    trim["name"] = serde_json::json!(" M ");
    let mut missing = good.clone();
    missing.as_object_mut().unwrap().remove("v");
    let mut unknown = good.clone();
    unknown["future"] = serde_json::json!(1);
    let mut short = good.clone();
    short["steps"] = serde_json::json!([{"command":"x","params":{}}]);
    for raw in [trim, missing, unknown, short, serde_json::json!({"v":u64::MAX}), serde_json::json!({"v":"future"})] {
        wb.macros_read = vec![raw.clone()];
        assert!(run_book(&wb, true).is_err());
        assert_eq!(wb.macros_read, [raw]);
        assert!(wb.macros.is_empty());
    }
    wb.macros_read = vec![good.clone(), serde_json::json!(null)];
    assert!(run_book(&wb, true).is_err());
    assert!(wb.macros.is_empty());
    assert_eq!(wb.macros_read.len(), 2);
    wb.macros = vec![current("T")].into();
    assert!(run_book(&wb, true).is_err());
}
#[test]
fn namespace_matches_trim_ascii_case_reservations_without_touching_inert_futures() {
    let mut wb = Workbook::new();
    wb.macros = vec![current("Alpha"), current("aLPHA")].into();
    assert!(run_book(&wb, false).is_err());
    wb.macros = vec![current("Alpha")].into();
    wb.macros_unread = Arc::new(vec![serde_json::json!({"v":2,"name":" aLPHA "})]);
    assert!(run_book(&wb, false).is_err());
    wb.macros = vec![current("Ä")].into();
    wb.macros_unread = Arc::new(vec![
        serde_json::json!({"v":u64::MAX,"name":"ä","steps":[{"not":"current"}]}),
        serde_json::json!({"v":2,"name":"ä"}),
        serde_json::json!({"v":3}),
    ]);
    assert!(run_book(&wb, false).is_ok());
    assert_eq!(wb.macros_unread.len(), 3);
    wb.macros_unread = Arc::new(vec![serde_json::json!({"v":1})]);
    assert!(run_book(&wb, false).is_err());
}
#[test]
fn all_channels_are_bounded_before_any_real_helper_entry() {
    let mut wb = Workbook::new();
    wb.macros = vec![current("good"), current("bad")].into();
    wb.macros[1].steps = vec![Step::command("x", serde_json::json!({})); 3];
    let caps = Caps { steps: 2, ..Caps::default() };
    let mut stats = Stats::default();
    assert!(run(&wb, &metadata(), &RecoveryLimits::default(), false, &caps, &mut stats).is_err());
    assert_eq!((stats.parse_entries, stats.check_entries), (0, 0));
    assert_eq!(stats.observer_calls, 1);
    wb.macros = vec![current("good")].into();
    wb.macros_unread = Arc::new(vec![serde_json::json!({"v":2,"x":"wide"})]);
    let mut stats = Stats::default();
    assert!(run(&wb, &metadata(), &RecoveryLimits::default(), false, &Caps { item_chars: 4, ..Caps::default() }, &mut stats).is_err());
    assert_eq!(stats.parse_entries, 0);
}

#[test]
fn production_step_boundary_and_late_pending_failure_never_commit_partial_data() {
    let mut wb = Workbook::new();
    let mut macro_ = current("Wide");
    macro_.steps = vec![Step::new(StepKind::Stop { message: String::new(), rollback: false }); gridcraft_model::MAX_MACRO_STEPS];
    wb.macros_read = vec![serde_json::to_value(&macro_).unwrap()];
    let staged = run_book(&wb, true).unwrap();
    assert_eq!(staged.adopted.as_ref().unwrap()[0].steps.len(), 10000);
    macro_.steps.push(Step::command("x", serde_json::json!({})));
    wb.macros_read = vec![serde_json::to_value(&macro_).unwrap()];
    let mut stats = Stats::default();
    assert!(run(&wb, &metadata(), &RecoveryLimits::default(), true, &Caps::default(), &mut stats).is_err());
    assert_eq!((stats.parse_entries, stats.deserialize_entries, stats.check_entries), (0, 0, 0));
    assert!(wb.macros.is_empty());
    wb.macros_read = vec![serde_json::to_value(current("first")).unwrap(), serde_json::to_value(current("second")).unwrap()];
    let staged = run_book(&wb, true).unwrap();
    assert_eq!(staged.adopted.unwrap().iter().map(|m| m.name.as_str()).collect::<Vec<_>>(), ["first", "second"]);
    wb.macros_read.push(serde_json::json!({"v":1,"name":"invalid"}));
    assert!(run_book(&wb, true).is_err());
    assert!(wb.macros.is_empty());
}

#[test]
fn precise_scalar_forms_and_all_existing_character_limits_precede_helpers() {
    let mut macro_ = current("M");
    macro_.steps = vec![Step::command("x", serde_json::json!({"n":-0.0}))];
    let original = serde_json::to_value(&macro_).unwrap();
    macro_.steps[0] = Step::command("x", serde_json::json!({"n":0.0}));
    assert!(matches!(
        equal(&original, &macro_, &RecoveryLimits::default(), &mut Stats::default()),
        Err(RecoveryError::UnsupportedRepresentation("macro_adoption"))
    ));
    let mut wb = Workbook::new();
    let if_step = |value| Step::new(StepKind::If { cell: "A1".into(), op: gridcraft_model::CompareOp::Eq, value, then: vec![], otherwise: vec![] });
    macro_.steps = vec![if_step(serde_json::json!("é".repeat(4094)))];
    wb.macros = vec![macro_.clone()].into();
    run_book(&wb, false).unwrap();
    wb.macros[0].steps = vec![if_step(serde_json::json!("é".repeat(4095)))];
    let mut stats = Stats::default();
    assert!(run(&wb, &metadata(), &RecoveryLimits::default(), false, &Caps::default(), &mut stats).is_err());
    assert_eq!((stats.check_entries, stats.parse_entries), (0, 0));
    macro_.steps = vec![if_step(serde_json::json!("\n".repeat(2047)))];
    wb.macros = vec![macro_.clone()].into();
    run_book(&wb, false).unwrap();
    wb.macros[0].steps = vec![if_step(serde_json::json!("\n".repeat(2048)))];
    assert!(run_book(&wb, false).is_err());
    macro_ = current("M");
    macro_.description = "😀".repeat(4096);
    macro_.shortcut = Some("x".repeat(64));
    macro_.permissions = vec!["x".repeat(255); 256];
    macro_.inputs = vec![gridcraft_model::NamedInput { col_types: vec![gridcraft_model::ColType::Any; 16384], ..Default::default() }];
    wb.macros = vec![macro_.clone()].into();
    run_book(&wb, false).unwrap();
    wb.macros[0].inputs[0].col_types.push(gridcraft_model::ColType::Any);
    assert!(run_book(&wb, false).is_err());
    wb.macros = vec![macro_].into();
    wb.macros[0].description.push('x');
    let mut stats = Stats::default();
    assert!(run(&wb, &metadata(), &RecoveryLimits::default(), false, &Caps::default(), &mut stats).is_err());
    assert_eq!(stats.check_entries, 0);
}

#[test]
fn future_item_characters_channel_count_and_namespace_requests_are_real_bounds() {
    let mut wb = Workbook::new();
    let future = serde_json::json!({"v":u64::MAX,"data":"é","steps":vec![serde_json::json!({});10001]});
    wb.macros_unread = Arc::new(vec![future.clone()]);
    let measured = count::measure(&future, usize::MAX, usize::MAX).unwrap();
    let mut stats = Stats::default();
    run(&wb, &metadata(), &RecoveryLimits::default(), false, &Caps { item_chars: measured.chars, ..Caps::default() }, &mut stats).unwrap();
    let mut too = Stats::default();
    assert!(run(&wb, &metadata(), &RecoveryLimits::default(), false, &Caps { item_chars: measured.chars - 1, ..Caps::default() }, &mut too).is_err());
    assert_eq!(too.parse_entries, 0);
    assert!(measured.bytes > measured.chars);
    wb.macros_unread = Arc::new(vec![serde_json::json!({"v":2}); 1000]);
    run_book(&wb, false).unwrap();
    wb.macros_unread = Arc::new(vec![serde_json::json!({"v":2}); 1001]);
    let mut too = Stats::default();
    assert!(run(&wb, &metadata(), &RecoveryLimits::default(), false, &Caps::default(), &mut too).is_err());
    assert_eq!((too.parse_entries, too.audit_nodes), (0, 0));
    wb.macros_unread = Arc::new(vec![serde_json::json!({"v":2,"name":"future"})]);
    wb.macros_read = vec![serde_json::to_value(current("one")).unwrap(), serde_json::to_value(current("two")).unwrap()];
    let staged = run_book(&wb, true).unwrap();
    assert_eq!(staged.adopted.as_ref().unwrap().len(), 2);
    use std::io::Write;
    writeln!(
        std::io::stdout(),
        "D2_HELPER_REQUESTS {:?} macro_layout={} name_layout={}",
        staged.stats,
        std::mem::size_of::<Macro>(),
        std::mem::size_of::<Name<'_>>()
    )
    .unwrap();
}

#[test]
fn readonly_checks_preserve_all_names_python_crlf_formula_cache_and_spills() {
    use gridcraft_core::{CellRef, RangeRef, Value};
    use gridcraft_model::{Cell, DefinedName, Formula, FormulaEvalMode, PyCode, PyOutput};
    let mut wb = Workbook::new();
    wb.macros = vec![current("M")].into();
    wb.names = vec![
        DefinedName { name: "_gc_macro_0_0".into(), scope: None, formula: "kept".into(), comment: "hidden".into(), hidden: true },
        DefinedName { name: "_gc_macro_other".into(), scope: None, formula: "kept".into(), comment: String::new(), hidden: true },
        DefinedName { name: "visible".into(), scope: None, formula: "0".into(), comment: String::new(), hidden: false },
        DefinedName { name: "scoped".into(), scope: Some(0), formula: "A1".into(), comment: String::new(), hidden: true },
    ];
    let python = Arc::new(PyCode {
        code: "x\r\ny\rz".into(),
        object: true,
        stale: true,
        output: Some(PyOutput {
            value: Value::Text("result".into()),
            type_name: "object".into(),
            preview: "full preview".into(),
            convertible: false,
        }),
    });
    let formula = Arc::new(Formula {
        text: "PY(\"untouched\",1)".into(),
        expr: None,
        array: Some(RangeRef::new(CellRef::new(0, 0), CellRef::new(1, 0))),
        dynamic: true,
        eval_mode: FormulaEvalMode::Legacy,
    });
    let sheet = Arc::make_mut(&mut wb.sheets[0]);
    sheet.cells.set(CellRef::default(), Cell { value: Value::Number(99.0), py: Some(python.clone()), ..Default::default() });
    sheet.cells.set(CellRef::new(1, 0), Cell { value: Value::Number(-0.0), formula: Some(formula.clone()), ..Default::default() });
    sheet.spill.insert(CellRef::new(2, 0), Value::Text("spill".into()));
    sheet.spill_ranges.insert(CellRef::default(), RangeRef::new(CellRef::default(), CellRef::new(2, 0)));
    let before = write::manifest_bytes(&wb, &metadata(), &RecoveryLimits::default()).unwrap().bytes;
    run_book(&wb, false).unwrap();
    assert_eq!(write::manifest_bytes(&wb, &metadata(), &RecoveryLimits::default()).unwrap().bytes, before);
    assert!(Arc::ptr_eq(wb.sheets[0].cells.get(CellRef::default()).unwrap().py.as_ref().unwrap(), &python));
    assert!(Arc::ptr_eq(wb.sheets[0].cells.get(CellRef::new(1, 0)).unwrap().formula.as_ref().unwrap(), &formula));
    wb.macros[0].name = " M ".into();
    let bad_before = write::manifest_bytes(&wb, &metadata(), &RecoveryLimits::default()).unwrap().bytes;
    assert!(run_book(&wb, false).is_err());
    assert_eq!(write::manifest_bytes(&wb, &metadata(), &RecoveryLimits::default()).unwrap().bytes, bad_before);
}

#[test]
fn control_params_are_explicitly_audited_as_general_and_d1_refuses_first() {
    let mut wb = Workbook::new();
    wb.macros = vec![current("M")].into();
    Arc::make_mut(&mut wb.sheets[0]).controls.push(gridcraft_model::FormControl {
        action: Some(gridcraft_model::ControlAction::Command {
            id: "x".into(),
            params: serde_json::json!({"steps":[{},{},{}],"value":"x".repeat(5000)}),
        }),
        ..Default::default()
    });
    let mut stats = Stats::default();
    run(&wb, &metadata(), &RecoveryLimits::default(), false, &Caps { steps: 0, if_chars: 0, ..Caps::default() }, &mut stats).unwrap();
    assert!(stats.audit_nodes > 0 && stats.audit_keys > 0);
    assert_eq!(stats.step_peak, 0);
    let mut stats = Stats::default();
    assert!(run(&wb, &metadata(), &RecoveryLimits { nodes: 1, ..RecoveryLimits::default() }, false, &Caps::default(), &mut stats).is_err());
    assert_eq!((stats.observer_calls, stats.audit_nodes, stats.parse_entries), (1, 0, 0));
}

#[test]
fn remaining_source_field_limits_and_real_semantic_validator_are_reused() {
    let mut wb = Workbook::new();
    let mut good = current(&"😀".repeat(255));
    good.shortcut = Some("x".repeat(64));
    good.output.anchor = Some("r".repeat(1024));
    good.inputs = (0..64)
        .map(|i| gridcraft_model::NamedInput { name: format!("Input_{i}"), default_ref: Some("r".repeat(1024)), ..Default::default() })
        .collect();
    good.permissions = vec!["x".repeat(255); 256];
    good.steps = vec![Step {
        kind: StepKind::Repeat {
            over: "r".repeat(1024),
            of: Default::default(),
            max: 1,
            body: vec![Step {
                kind: StepKind::Stop { message: "é".repeat(4096), rollback: true },
                enabled: false,
                breakpoint: true,
                note: "n".repeat(4096),
            }],
        },
        enabled: false,
        breakpoint: true,
        note: String::new(),
    }];
    wb.macros = vec![good.clone()].into();
    run_book(&wb, false).unwrap();
    let mut cases = Vec::new();
    let mut value = good.clone();
    value.name.push('x');
    cases.push(value);
    let mut value = good.clone();
    value.shortcut.as_mut().unwrap().push('x');
    cases.push(value);
    let mut value = good.clone();
    value.output.anchor.as_mut().unwrap().push('x');
    cases.push(value);
    let mut value = good.clone();
    value.inputs.push(Default::default());
    cases.push(value);
    let mut value = good.clone();
    value.permissions.push("x".into());
    cases.push(value);
    let mut value = good.clone();
    value.permissions[0].push('x');
    cases.push(value);
    for value in cases {
        wb.macros = vec![value].into();
        let mut stats = Stats::default();
        assert!(run(&wb, &metadata(), &RecoveryLimits::default(), false, &Caps::default(), &mut stats).is_err());
        assert_eq!((stats.parse_entries, stats.check_entries), (0, 0));
    }
    for invalid in ["", "é", "bad-id"] {
        wb.macros = vec![Macro { name: "M".into(), steps: vec![Step::command(invalid, serde_json::json!({}))], ..Default::default() }].into();
        let mut stats = Stats::default();
        assert!(run(&wb, &metadata(), &RecoveryLimits::default(), false, &Caps::default(), &mut stats).is_err());
        assert_eq!(stats.check_entries, 1);
    }
    wb.macros = vec![Macro { v: 2, name: "M".into(), ..Default::default() }].into();
    assert!(run_book(&wb, false).is_err());
    wb.macros = vec![current("M")].into();
    wb.macros_unread = Arc::new(vec![serde_json::json!({"v":2,"name":123}), serde_json::json!({"v":2,"name":" \n "})]);
    run_book(&wb, false).unwrap();
}
