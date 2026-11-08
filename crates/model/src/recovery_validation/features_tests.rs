use super::*;
use crate::*;
use gridcraft_core::{CellRef, MAX_COLS, MAX_ROWS, RangeRef};
use std::sync::Arc;

fn range(r0: u32, c0: u32, r1: u32, c1: u32) -> RangeRef {
    RangeRef { start: CellRef::new(r0, c0), end: CellRef::new(r1, c1) }
}
fn table() -> Table {
    Table {
        id: 0,
        name: " retained table ".into(),
        range: range(0, 0, 0, 0),
        header_row: true,
        totals_row: false,
        columns: vec![TableColumn {
            name: "".into(),
            totals: TotalsFn::Custom,
            totals_label: Some("=broken(".into()),
            formula: Some("=[external]bad".into()),
            unique_name: Some("original".into()),
            xml: Some(XmlColumnPr { map_id: u32::MAX, xpath: "/unresolved/node".into(), data_type: "unknown".into(), denormalized: true }),
        }],
        style: "unknown".into(),
        banded_rows: false,
        banded_cols: true,
        first_col: true,
        last_col: true,
        filter_button: true,
        filter: vec![],
        table_type: Some("xml".into()),
        connection_id: Some(u32::MAX),
    }
}
fn workbook() -> Workbook {
    Workbook::default()
}
fn check(wb: &Workbook) -> Result<(), RecoveryModelError> {
    validate_workbook_features(wb)
}
fn unsupported(reason: RecoveryRepresentation) -> Result<(), RecoveryModelError> {
    Err(RecoveryModelError::UnsupportedRepresentation(reason))
}
fn filters() -> Vec<FilterCriterion> {
    vec![
        FilterCriterion::Values { values: vec!["a".into(), "a".into()], blanks: true },
        FilterCriterion::Custom { a: ("unknown".into(), "[external]".into()), b: Some(("?".into(), "bad(".into())), and: false },
        FilterCriterion::Custom { a: ("".into(), "".into()), b: None, and: true },
        FilterCriterion::Top10 { bottom: true, percent: true, count: u32::MAX },
        FilterCriterion::AboveAverage(false),
        FilterCriterion::AboveAverage(true),
        FilterCriterion::FillColor(Color::Auto),
        FilterCriterion::FontColor(Color::Theme(11, -1000)),
        FilterCriterion::Icon { set: "unknown".into(), id: None },
        FilterCriterion::Icon { set: "3Arrows".into(), id: Some(u32::MAX) },
    ]
}
fn values() -> Vec<CfValueKind> {
    vec![
        CfValueKind::Min,
        CfValueKind::Max,
        CfValueKind::Number(-0.0),
        CfValueKind::Percent(-999.0),
        CfValueKind::Percentile(f64::MAX),
        CfValueKind::Formula("=[external]bad(".into()),
    ]
}
fn styled_rules(style: Box<Style>) -> Vec<CfRule> {
    vec![
        CfRule::CellIs { op: CfOperator::Between, a: "bad(".into(), b: None, style: style.clone() },
        CfRule::Expression { formula: "[external]".into(), style: style.clone() },
        CfRule::ContainsText { text: "a".into(), style: style.clone() },
        CfRule::NotContainsText { text: "a".into(), style: style.clone() },
        CfRule::BeginsWith { text: "a".into(), style: style.clone() },
        CfRule::EndsWith { text: "a".into(), style: style.clone() },
        CfRule::Blanks { style: style.clone() },
        CfRule::NoBlanks { style: style.clone() },
        CfRule::Errors { style: style.clone() },
        CfRule::NoErrors { style: style.clone() },
        CfRule::Duplicate { style: style.clone() },
        CfRule::Unique { style: style.clone() },
        CfRule::Top10 { bottom: true, percent: true, rank: u32::MAX, style: style.clone() },
        CfRule::AboveAverage { below: true, equal: true, std_dev: u8::MAX, style: style.clone() },
        CfRule::TimePeriod { period: "unfamiliar".into(), style },
    ]
}
fn cf(rule: CfRule) -> CondFormat {
    CondFormat { ranges: vec![range(0, 0, MAX_ROWS - 1, MAX_COLS - 1)], rule, priority: 0, stop_if_true: true }
}

#[test]
fn recovery_features_table_static_header_xml_and_separate_regions_are_retained() {
    let mut wb = workbook();
    let sh = wb.sheet_mut(0).unwrap();
    sh.tables.push(table());
    sh.merges.push(range(0, 0, 0, 0)); // Separate families may overlap.
    sh.autofilter = Some(AutoFilter { range: range(9, 3, 10, 3), criteria: vec![] });
    let before = wb.clone();
    assert_eq!(check(&wb), Ok(()));
    assert_eq!(wb, before);
    assert_eq!(wb.table(" RETAINED TABLE "), Some((0, 0)));
    assert_eq!(wb.sheet(0).unwrap().tables[0].data_range(), None);
    for (header, totals) in [(false, false), (false, true), (true, false), (true, true)] {
        let t = &mut wb.sheet_mut(0).unwrap().tables[0];
        t.header_row = header;
        t.totals_row = totals;
        let before = wb.clone();
        assert_eq!(check(&wb), Ok(()));
        assert_eq!(wb, before);
    }
    for totals in [
        TotalsFn::None,
        TotalsFn::Average,
        TotalsFn::Count,
        TotalsFn::CountNums,
        TotalsFn::Max,
        TotalsFn::Min,
        TotalsFn::Sum,
        TotalsFn::StdDev,
        TotalsFn::Var,
        TotalsFn::Custom,
    ] {
        wb.sheet_mut(0).unwrap().tables[0].columns[0].totals = totals;
        let before = wb.clone();
        assert_eq!(check(&wb), Ok(()));
        assert_eq!(wb, before);
    }
}

#[test]
fn recovery_features_table_coordinates_width_offsets_and_names_are_checked() {
    let mut wb = workbook();
    wb.sheet_mut(0).unwrap().tables.push(table());
    for bad in [range(0, 0, 0, MAX_COLS), range(1, 0, 0, 0)] {
        wb.sheet_mut(0).unwrap().tables[0].range = bad;
        assert!(matches!(check(&wb), Err(RecoveryModelError::Coordinate | RecoveryModelError::Range)));
    }
    wb.sheet_mut(0).unwrap().tables[0] = table();
    wb.sheet_mut(0).unwrap().tables[0].columns.clear();
    assert_eq!(check(&wb), Err(RecoveryModelError::Table));
    wb.sheet_mut(0).unwrap().tables[0] = table();
    wb.sheet_mut(0).unwrap().tables[0].filter = vec![(1, FilterCriterion::AboveAverage(true))];
    assert_eq!(check(&wb), Err(RecoveryModelError::Filter));
    wb.sheet_mut(0).unwrap().tables[0].filter = vec![(0, FilterCriterion::AboveAverage(true)), (0, FilterCriterion::AboveAverage(false))];
    assert_eq!(check(&wb), unsupported(RecoveryRepresentation::AmbiguousIdentity));
    let t = &mut wb.sheet_mut(0).unwrap().tables[0];
    t.filter.clear();
    t.range.end.col = 1;
    t.columns.push(t.columns[0].clone());
    t.columns[0].name = "Name".into();
    t.columns[1].name = "nAME".into();
    assert_eq!(check(&wb), unsupported(RecoveryRepresentation::AmbiguousIdentity));
    wb.sheet_mut(0).unwrap().tables[0].columns[1].name = " Name".into();
    assert_eq!(check(&wb), Ok(()));
}

#[test]
fn recovery_features_table_names_and_sweep_refuse_ambiguity_while_inert_ids_remain() {
    let mut wb = workbook();
    wb.sheet_mut(0).unwrap().tables.push(table());
    let mut second = table();
    second.id = 1;
    second.name = "other".into();
    second.range = range(0, 1, 0, 1);
    wb.sheet_mut(0).unwrap().tables.push(second.clone());
    assert_eq!(check(&wb), Ok(()));
    wb.sheet_mut(0).unwrap().tables[1].range = range(0, 0, 0, 0);
    assert_eq!(check(&wb), unsupported(RecoveryRepresentation::OverlappingRectangles));
    wb.sheet_mut(0).unwrap().tables.pop();
    wb.sheets.push(Arc::new(Sheet::new("Second")));
    second.id = 0;
    wb.sheet_mut(1).unwrap().tables.push(second);
    let before = wb.clone();
    assert_eq!(check(&wb), Ok(())); // Duplicate inert table ids, including zero, are retained.
    assert_eq!(wb, before);
    wb.sheet_mut(1).unwrap().tables[0].id = u32::MAX;
    wb.sheet_mut(1).unwrap().tables[0].name = " RETAINED TABLE ".into();
    assert_eq!(check(&wb), unsupported(RecoveryRepresentation::AmbiguousIdentity));
    wb.sheet_mut(1).unwrap().tables[0].name = "Ä".into();
    wb.sheet_mut(0).unwrap().tables[0].name = "ä".into();
    assert_eq!(check(&wb), Ok(()));
}

#[test]
fn recovery_features_filter_payloads_are_preserved_and_colors_checked() {
    let mut wb = workbook();
    for criterion in filters() {
        wb.sheet_mut(0).unwrap().autofilter = Some(AutoFilter { range: range(0, 0, 0, 0), criteria: vec![(0, criterion)] });
        let before = wb.clone();
        assert_eq!(check(&wb), Ok(()));
        assert_eq!(wb, before);
    }
    for criterion in [FilterCriterion::FillColor(Color::Rgb(0x1000000)), FilterCriterion::FontColor(Color::Theme(12, 0))] {
        wb.sheet_mut(0).unwrap().autofilter.as_mut().unwrap().criteria = vec![(0, criterion)];
        assert_eq!(check(&wb), Err(RecoveryModelError::Style));
    }
    let f = wb.sheet_mut(0).unwrap().autofilter.as_mut().unwrap();
    f.criteria.clear();
    f.range = range(0, 0, MAX_ROWS, 0);
    assert_eq!(check(&wb), Err(RecoveryModelError::Coordinate));
}

#[test]
fn recovery_features_cf_every_style_variant_checks_embedded_payload_without_editor_limits() {
    let mut wb = workbook();
    let mut style = Style::default();
    style.font.size = -0.0;
    for rule in styled_rules(Box::new(style.clone())) {
        wb.sheet_mut(0).unwrap().cond_formats = vec![cf(rule)];
        let before = wb.clone();
        assert_eq!(check(&wb), Ok(()));
        assert_eq!(wb, before);
    }
    for mutate in [0, 1, 2] {
        let mut bad = style.clone();
        match mutate {
            0 => bad.font.size = f32::NAN,
            1 => bad.fill.bg = Color::Rgb(0x1000000),
            _ => bad.align.rotation = 91,
        }
        for rule in styled_rules(Box::new(bad)) {
            wb.sheet_mut(0).unwrap().cond_formats = vec![cf(rule)];
            assert_eq!(check(&wb), Err(RecoveryModelError::Style));
        }
    }
}

#[test]
fn recovery_features_cf_visual_payloads_empty_thresholds_and_huge_sparse_ranges_are_content() {
    let mut wb = workbook();
    let mut rules = vec![
        CfRule::ColorScale { stops: vec![] },
        CfRule::ColorScale { stops: values().into_iter().map(|v| (v, Color::Auto)).collect() },
        CfRule::DataBar {
            min: CfValueKind::Percentile(-100.0),
            max: CfValueKind::Number(f64::MAX),
            color: Color::Theme(0, 1000),
            gradient: false,
            show_value: false,
        },
    ];
    for set in ["3Arrows", "4Rating", "5Quarters", "unknown"] {
        for len in [0, 2, 3, 4, 5] {
            rules.push(CfRule::IconSet { set: set.into(), thresholds: values().into_iter().take(len).collect(), reverse: true, show_value: false });
        }
    }
    wb.sheet_mut(0).unwrap().cond_formats = rules.into_iter().map(cf).collect();
    wb.sheet_mut(0).unwrap().cond_formats.push(CondFormat {
        ranges: vec![],
        rule: CfRule::IconSet { set: "".into(), thresholds: vec![], reverse: false, show_value: true },
        priority: 0,
        stop_if_true: false,
    });
    let before = wb.clone();
    assert_eq!(check(&wb), Ok(()));
    assert_eq!(wb, before);
    assert!(wb.sheet(0).unwrap().cells.is_empty());
}

#[test]
fn recovery_features_cf_nonfinite_every_numeric_kind_and_nested_color_are_refused() {
    let mut wb = workbook();
    for n in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        for value in [CfValueKind::Number(n), CfValueKind::Percent(n), CfValueKind::Percentile(n)] {
            for rule in [
                CfRule::ColorScale { stops: vec![(value.clone(), Color::Auto)] },
                CfRule::DataBar { min: value.clone(), max: CfValueKind::Max, color: Color::Auto, gradient: true, show_value: true },
                CfRule::DataBar { min: CfValueKind::Min, max: value.clone(), color: Color::Auto, gradient: false, show_value: false },
                CfRule::IconSet { set: "unknown".into(), thresholds: vec![value], reverse: false, show_value: true },
            ] {
                wb.sheet_mut(0).unwrap().cond_formats = vec![cf(rule)];
                assert_eq!(check(&wb), Err(RecoveryModelError::Value));
            }
        }
    }
    for rule in [
        CfRule::ColorScale { stops: vec![(CfValueKind::Min, Color::Theme(0, 1001))] },
        CfRule::DataBar { min: CfValueKind::Min, max: CfValueKind::Max, color: Color::Rgb(u32::MAX), gradient: false, show_value: true },
    ] {
        wb.sheet_mut(0).unwrap().cond_formats = vec![cf(rule)];
        assert_eq!(check(&wb), Err(RecoveryModelError::Style));
    }
    wb.sheet_mut(0).unwrap().cond_formats = vec![cf(CfRule::ColorScale { stops: vec![] })];
    wb.sheet_mut(0).unwrap().cond_formats[0].ranges.push(range(2, 0, 1, 0));
    assert_eq!(check(&wb), Err(RecoveryModelError::Range));
}

#[test]
fn recovery_features_validation_all_kinds_operators_flags_and_text_are_retained() {
    let mut wb = workbook();
    for kind in [
        ValidationKind::Any,
        ValidationKind::Whole,
        ValidationKind::Decimal,
        ValidationKind::List,
        ValidationKind::Date,
        ValidationKind::Time,
        ValidationKind::TextLength,
        ValidationKind::Custom,
    ] {
        for op in [
            CfOperator::Between,
            CfOperator::NotBetween,
            CfOperator::Equal,
            CfOperator::NotEqual,
            CfOperator::Greater,
            CfOperator::Less,
            CfOperator::GreaterOrEqual,
            CfOperator::LessOrEqual,
        ] {
            for error_style in [ErrorStyle::Stop, ErrorStyle::Warning, ErrorStyle::Information] {
                wb.sheet_mut(0).unwrap().validations = vec![Validation {
                    ranges: vec![],
                    kind,
                    op,
                    f1: "=[external]bad(".into(),
                    f2: Some("".into()),
                    allow_blank: false,
                    in_cell_dropdown: false,
                    input_title: "title".into(),
                    input_message: "input".into(),
                    show_input: false,
                    error_title: "error".into(),
                    error_message: "message".into(),
                    error_style,
                    show_error: false,
                }];
                let before = wb.clone();
                assert_eq!(check(&wb), Ok(()));
                assert_eq!(wb, before);
            }
        }
    }
    wb.sheet_mut(0).unwrap().validations[0].ranges = vec![range(0, 0, MAX_ROWS - 1, MAX_COLS - 1)];
    assert_eq!(check(&wb), Ok(()));
    wb.sheet_mut(0).unwrap().validations[0].ranges.push(range(0, MAX_COLS, 0, MAX_COLS));
    assert_eq!(check(&wb), Err(RecoveryModelError::Coordinate));
}

fn what_if() -> WhatIfTable {
    WhatIfTable {
        anchor: CellRef::new(0, 0),
        range: range(0, 0, 0, 0),
        formula_text: "=bad external(".into(),
        input1: None,
        input2: None,
        two_variable: None,
        row_oriented: None,
        input1_deleted: None,
        input2_deleted: None,
        calculate: None,
    }
}
#[test]
fn recovery_features_what_if_optional_flags_inputs_and_overlaps_are_preserved() {
    let mut wb = workbook();
    for a in [None, Some(false), Some(true)] {
        for b in [None, Some(false), Some(true)] {
            let mut t = what_if();
            t.two_variable = a;
            t.row_oriented = b;
            t.input1_deleted = a;
            t.input2_deleted = b;
            t.calculate = a;
            wb.sheet_mut(0).unwrap().what_if_tables = vec![t];
            let before = wb.clone();
            assert_eq!(check(&wb), Ok(()));
            assert_eq!(wb, before);
        }
    }
    let mut t = what_if();
    t.range = range(0, 0, 3, 3);
    t.input1 = Some(CellRef::new(MAX_ROWS - 1, MAX_COLS - 1));
    t.input2 = Some(CellRef::new(0, 0));
    let mut other = what_if();
    other.anchor = CellRef::new(1, 1);
    other.range = range(1, 1, 2, 2);
    wb.sheet_mut(0).unwrap().what_if_tables = vec![t, other];
    assert_eq!(check(&wb), Ok(()));
}
#[test]
fn recovery_features_what_if_anchor_and_optional_input_coordinates_are_checked() {
    let mut wb = workbook();
    for n in 0..5 {
        let mut t = what_if();
        match n {
            0 => t.anchor.row = MAX_ROWS,
            1 => t.range.end.col = MAX_COLS,
            2 => t.input1 = Some(CellRef::new(MAX_ROWS, 0)),
            3 => t.input2 = Some(CellRef::new(0, MAX_COLS)),
            _ => t.anchor.col = 1,
        }
        wb.sheet_mut(0).unwrap().what_if_tables = vec![t];
        assert_eq!(check(&wb), Err(if n == 4 { RecoveryModelError::WhatIf } else { RecoveryModelError::Coordinate }));
    }
    wb.sheet_mut(0).unwrap().what_if_tables = vec![what_if(), what_if()];
    assert_eq!(check(&wb), unsupported(RecoveryRepresentation::AmbiguousIdentity));
}

#[test]
fn recovery_features_pivot_external_sources_stale_output_duplicate_aggregations_and_enums() {
    let mut wb = workbook();
    for source in ["[missing]Sheet!A1", "unparseable((", "missing name", ""] {
        let mut p = PivotTable {
            source: source.into(),
            anchor: CellRef::new(8, 8),
            last_range: Some(range(0, 0, 0, 0)),
            values_position: Some(usize::MAX),
            ..PivotTable::default()
        };
        for sort in [PivotSort::Asc, PivotSort::Desc, PivotSort::None] {
            for date_group in [PivotDateGroup::None, PivotDateGroup::Years, PivotDateGroup::Quarters, PivotDateGroup::Months, PivotDateGroup::Days] {
                p.rows.push(PivotField { source_col: "missing".into(), sort, date_group, collapsed_items: vec!["x".into(), "x".into()] });
            }
        }
        p.columns = p.rows.clone();
        p.filters =
            vec![PivotFilter { source_col: "missing".into(), selected: None }, PivotFilter { source_col: "missing".into(), selected: Some(vec![]) }];
        for func in [
            PivotFunc::Sum,
            PivotFunc::Count,
            PivotFunc::Average,
            PivotFunc::Max,
            PivotFunc::Min,
            PivotFunc::Product,
            PivotFunc::CountNumbers,
            PivotFunc::StdDev,
            PivotFunc::StdDevP,
            PivotFunc::Var,
            PivotFunc::VarP,
        ] {
            for show_as in [
                PivotShowAs::Normal,
                PivotShowAs::PercentOfGrandTotal,
                PivotShowAs::PercentOfColumnTotal,
                PivotShowAs::PercentOfRowTotal,
                PivotShowAs::RunningTotal,
                PivotShowAs::Rank,
            ] {
                p.values.push(PivotValue {
                    source_col: "missing".into(),
                    func,
                    name: "same".into(),
                    show_as,
                    number_format: Some("unfamiliar".into()),
                });
            }
        }
        p.calculated_fields = vec![PivotCalcField { name: " retained ".into(), formula: "=broken(".into() }];
        for layout in [PivotLayout::Compact, PivotLayout::Outline, PivotLayout::Tabular] {
            p.layout = layout;
            wb.sheet_mut(0).unwrap().pivots = vec![p.clone()];
            let before = wb.clone();
            assert_eq!(check(&wb), Ok(()));
            assert_eq!(wb, before);
        }
    }
}

#[test]
fn recovery_features_pivot_active_position_coordinates_and_trim_ascii_identity() {
    let mut wb = workbook();
    let mut p = PivotTable { values_on_rows: true, values_position: Some(0), ..PivotTable::default() };
    wb.sheet_mut(0).unwrap().pivots.push(p.clone());
    assert_eq!(check(&wb), Ok(()));
    wb.sheet_mut(0).unwrap().pivots[0].values_position = Some(1);
    assert_eq!(check(&wb), Err(RecoveryModelError::Pivot));
    wb.sheet_mut(0).unwrap().pivots[0].rows.push(PivotField::default());
    assert_eq!(check(&wb), Ok(())); // Append position equals the actual nonempty Rows length.
    for n in 0..2 {
        wb.sheet_mut(0).unwrap().pivots[0] = p.clone();
        if n == 0 {
            wb.sheet_mut(0).unwrap().pivots[0].anchor.col = MAX_COLS;
        } else {
            wb.sheet_mut(0).unwrap().pivots[0].last_range = Some(range(1, 0, 0, 0));
        }
        assert!(matches!(check(&wb), Err(RecoveryModelError::Coordinate | RecoveryModelError::Range)));
    }
    p.calculated_fields =
        vec![PivotCalcField { name: " X\t".into(), formula: "".into() }, PivotCalcField { name: "x".into(), formula: "bad(".into() }];
    wb.sheet_mut(0).unwrap().pivots[0] = p.clone();
    assert_eq!(check(&wb), unsupported(RecoveryRepresentation::AmbiguousIdentity));
    p.calculated_fields[1].name = "Ä".into();
    wb.sheet_mut(0).unwrap().pivots[0] = p;
    assert_eq!(check(&wb), Ok(()));
}

#[test]
fn recovery_features_pivot_global_identity_and_cross_family_ids_are_separate() {
    let mut wb = workbook();
    wb.sheet_mut(0).unwrap().pivots.push(PivotTable::default());
    wb.sheet_mut(0).unwrap().tables.push(table());
    assert_eq!(check(&wb), Ok(()));
    wb.sheets.push(Arc::new(Sheet::new("Second")));
    let p = PivotTable { name: "other".into(), ..PivotTable::default() };
    wb.sheet_mut(1).unwrap().pivots.push(p);
    assert_eq!(check(&wb), unsupported(RecoveryRepresentation::AmbiguousIdentity));
    wb.sheet_mut(1).unwrap().pivots[0].id = u32::MAX;
    wb.sheet_mut(1).unwrap().pivots[0].name = "pIVOTtABLE1".into();
    assert_eq!(check(&wb), unsupported(RecoveryRepresentation::AmbiguousIdentity));
    wb.sheet_mut(1).unwrap().pivots[0].name = " PivotTable1".into();
    assert_eq!(check(&wb), Ok(()));
}

#[test]
fn recovery_features_success_and_late_failure_preserve_sharing_order_and_float_bits() {
    let mut wb = workbook();
    wb.sheet_mut(0).unwrap().tables.push(table());
    wb.sheet_mut(0).unwrap().cond_formats =
        vec![cf(CfRule::DataBar { min: CfValueKind::Number(-0.0), max: CfValueKind::Max, color: Color::Auto, gradient: true, show_value: false })];
    let before = wb.clone();
    let ptr = Arc::as_ptr(&wb.sheets[0]);
    let count = Arc::strong_count(&wb.sheets[0]);
    assert_eq!(check(&wb), Ok(()));
    assert_eq!(wb, before);
    assert_eq!(Arc::as_ptr(&wb.sheets[0]), ptr);
    assert_eq!(Arc::strong_count(&wb.sheets[0]), count);
    let sh = wb.sheet_mut(0).unwrap();
    sh.pivots.push(PivotTable { values_on_rows: true, values_position: Some(usize::MAX), ..PivotTable::default() });
    let before = wb.clone();
    let ptr = Arc::as_ptr(&wb.sheets[0]);
    let count = Arc::strong_count(&wb.sheets[0]);
    assert_eq!(check(&wb), Err(RecoveryModelError::Pivot));
    assert_eq!(wb, before);
    assert_eq!(Arc::as_ptr(&wb.sheets[0]), ptr);
    assert_eq!(Arc::strong_count(&wb.sheets[0]), count);
    if let CfRule::DataBar { min: CfValueKind::Number(n), .. } = &wb.sheets[0].cond_formats[0].rule {
        assert_eq!(n.to_bits(), (-0.0f64).to_bits());
    } else {
        panic!("fixture changed");
    }
}

#[test]
fn recovery_features_nonfinite_refusal_preserves_nan_payload_and_arc_identity() {
    let mut wb = workbook();
    let bits = 0x7ff8_0000_0000_4321;
    wb.sheet_mut(0).unwrap().cond_formats = vec![cf(CfRule::IconSet {
        set: "unfamiliar".into(),
        thresholds: vec![CfValueKind::Number(f64::from_bits(bits))],
        reverse: true,
        show_value: false,
    })];
    let ptr = Arc::as_ptr(&wb.sheets[0]);
    let count = Arc::strong_count(&wb.sheets[0]);
    assert_eq!(check(&wb), Err(RecoveryModelError::Value));
    assert_eq!(Arc::as_ptr(&wb.sheets[0]), ptr);
    assert_eq!(Arc::strong_count(&wb.sheets[0]), count);
    if let CfRule::IconSet { thresholds, .. } = &wb.sheets[0].cond_formats[0].rule {
        if let CfValueKind::Number(n) = thresholds[0] {
            assert_eq!(n.to_bits(), bits);
        } else {
            panic!("fixture changed");
        }
    } else {
        panic!("fixture changed");
    }
}
