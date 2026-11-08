use super::fixtures::*;
use super::*;

#[test]
fn recovery_public_api_all_families_preserve_fields_sharing_caches_and_bits() {
    let mut wb = inventory_workbook();
    // Prepare before pointer capture: COW from the finite expected snapshot must not be
    // mistaken for validator mutation. No hostile JSON cloning is used in this fixture.
    let expected = wb.clone();
    let sheet = &wb.sheets[0];
    let pointers = (
        Arc::as_ptr(sheet),
        Arc::as_ptr(&sheet.comments),
        Arc::as_ptr(&sheet.images[0].data),
        Arc::as_ptr(sheet.cells.get(at(0, 0)).unwrap().formula.as_ref().unwrap()),
        Arc::as_ptr(sheet.cells.get(at(1, 0)).unwrap().py.as_ref().unwrap()),
    );
    let counts = (Arc::strong_count(sheet), Arc::strong_count(&sheet.comments), Arc::strong_count(&sheet.images[0].data));
    let uid = sheet.uid.get();
    let math = sheet.shapes[0].math.get("bad(").unwrap().unwrap_err() as *const _;
    let active = wb.active_sheet.get();
    assert_eq!(crate::validate_recovery_model(&wb), Ok(()));
    assert_eq!(wb, expected);
    assert!(wb.macros.shares(&expected.macros));
    assert_eq!(
        pointers,
        (
            Arc::as_ptr(&wb.sheets[0]),
            Arc::as_ptr(&wb.sheets[0].comments),
            Arc::as_ptr(&wb.sheets[0].images[0].data),
            Arc::as_ptr(wb.sheets[0].cells.get(at(0, 0)).unwrap().formula.as_ref().unwrap()),
            Arc::as_ptr(wb.sheets[0].cells.get(at(1, 0)).unwrap().py.as_ref().unwrap())
        )
    );
    assert_eq!(
        counts,
        (Arc::strong_count(&wb.sheets[0]), Arc::strong_count(&wb.sheets[0].comments), Arc::strong_count(&wb.sheets[0].images[0].data))
    );
    assert_eq!((wb.sheets[0].uid.get(), wb.active_sheet.get()), (uid, active));
    assert_eq!(wb.sheets[0].shapes[0].math.get("bad(").unwrap().unwrap_err() as *const _, math);
    assert_eq!(wb.sheets[0].print.margins[0].to_bits(), (-0.0f32).to_bits());
    let Value::Array(array) = &wb.sheets[0].cells.get(at(0, 0)).unwrap().value else { panic!("array") };
    let Value::Number(n) = array.data[0] else { panic!("number") };
    assert_eq!(n.to_bits(), (-0.0f64).to_bits());
    // The last family checked is phonetics. A late offset error must leave every earlier
    // family (including the derived AST/math/accounting and opaque fields) unchanged.
    wb.phonetics[0].runs[1].end = 5;
    let failed = wb.clone();
    let sheet_ptr = Arc::as_ptr(&wb.sheets[0]);
    assert_eq!(crate::validate_recovery_model(&wb), Err(RecoveryModelError::Phonetic));
    assert_eq!(wb, failed);
    assert_eq!(wb.active_sheet.get(), active);
    assert_eq!(wb.sheets[0].shapes[0].math.get("bad(").unwrap().unwrap_err() as *const _, math);
    assert_eq!(Arc::as_ptr(&wb.sheets[0]), sheet_ptr);
    assert!(wb.macros.shares(&failed.macros));
    let nan = f32::from_bits(0x7fc0_0042);
    wb.sheet_mut(0).unwrap().print.margins[5] = nan;
    let sheet_ptr = Arc::as_ptr(&wb.sheets[0]);
    assert_eq!(crate::validate_recovery_model(&wb), Err(RecoveryModelError::Print));
    assert_eq!(wb.sheets[0].print.margins[5].to_bits(), nan.to_bits());
    assert_eq!(Arc::as_ptr(&wb.sheets[0]), sheet_ptr);
}

#[test]
fn recovery_public_api_every_checked_family_has_an_accepted_neighbor_and_hostile_field() {
    type Mutation = (&'static str, fn(&mut Workbook), RecoveryModelError);
    let cases: &[Mutation] = &[
        ("raw count", |w| w.sheet_mut(0).unwrap().cells = serde_json::from_str("{\"bands\":{},\"len\":1}").unwrap(), RecoveryModelError::Storage),
        ("active sheet", |w| w.active_sheet.set(99), RecoveryModelError::Workbook),
        ("name scope", |w| w.names[0].scope = Some(99), RecoveryModelError::Workbook),
        ("calc finite", |w| w.calc.max_change = f64::INFINITY, RecoveryModelError::Value),
        ("theme color", |w| w.theme.colors[11] = 0x1000000, RecoveryModelError::Style),
        ("named style", |w| w.cell_styles[0].1.font.size = f32::INFINITY, RecoveryModelError::Style),
        ("cell value", |w| w.sheet_mut(0).unwrap().cells.get_mut(at(0, 1)).unwrap().value = Value::Number(f64::INFINITY), RecoveryModelError::Value),
        (
            "cell array",
            |w| w.sheet_mut(0).unwrap().cells.get_mut(at(0, 1)).unwrap().value = Value::Array(Arc::new(Array { rows: 0, cols: 1, data: vec![] })),
            RecoveryModelError::Array,
        ),
        ("cell style", |w| w.sheet_mut(0).unwrap().cells.get_mut(at(0, 1)).unwrap().style = StyleId(u32::MAX), RecoveryModelError::Style),
        (
            "cell ownership",
            |w| w.sheet_mut(0).unwrap().cells.get_mut(at(0, 0)).unwrap().py = Some(Arc::new(PyCode::default())),
            RecoveryModelError::CellOwnership,
        ),
        (
            "formula array",
            |w| Arc::make_mut(w.sheet_mut(0).unwrap().cells.get_mut(at(0, 0)).unwrap().formula.as_mut().unwrap()).array = Some(rect(1, 0, 1, 1)),
            RecoveryModelError::CellOwnership,
        ),
        (
            "python code",
            |w| Arc::make_mut(w.sheet_mut(0).unwrap().cells.get_mut(at(1, 0)).unwrap().py.as_mut().unwrap()).code = "x".repeat(8080),
            RecoveryModelError::Python,
        ),
        (
            "python output",
            |w| {
                Arc::make_mut(w.sheet_mut(0).unwrap().cells.get_mut(at(1, 0)).unwrap().py.as_mut().unwrap()).output.as_mut().unwrap().value =
                    Value::Number(f64::INFINITY)
            },
            RecoveryModelError::Value,
        ),
        (
            "spill count",
            |w| {
                w.sheet_mut(0).unwrap().spill.clear();
            },
            RecoveryModelError::Spill,
        ),
        (
            "spill scalar",
            |w| {
                w.sheet_mut(0).unwrap().spill.insert(at(0, 1), Value::Array(Arc::new(Array::scalar(Value::Empty))));
            },
            RecoveryModelError::Array,
        ),
        (
            "row key",
            |w| {
                w.sheet_mut(0).unwrap().rows.insert(MAX_ROWS, LineInfo::default());
            },
            RecoveryModelError::Coordinate,
        ),
        (
            "column style",
            |w| w.sheet_mut(0).unwrap().cols.get_mut(&(MAX_COLS - 1)).unwrap().style = Some(StyleId(u32::MAX)),
            RecoveryModelError::Style,
        ),
        ("line size", |w| w.sheet_mut(0).unwrap().default_row_height = -1.0, RecoveryModelError::Geometry),
        ("merge range", |w| w.sheet_mut(0).unwrap().merges[0].end.row = MAX_ROWS, RecoveryModelError::Coordinate),
        ("freeze count", |w| w.sheet_mut(0).unwrap().freeze = Some((MAX_ROWS + 1, 0)), RecoveryModelError::Coordinate),
        ("view coordinate", |w| w.sheet_mut(0).unwrap().view_active.row = MAX_ROWS, RecoveryModelError::Coordinate),
        ("tab color", |w| w.sheet_mut(0).unwrap().tab_color = Some(Color::Theme(12, 0)), RecoveryModelError::Style),
        ("table width", |w| w.sheet_mut(0).unwrap().tables[0].range.end.col = 1, RecoveryModelError::Table),
        ("table filter", |w| w.sheet_mut(0).unwrap().tables[0].filter[0].0 = 1, RecoveryModelError::Filter),
        ("what if", |w| w.sheet_mut(0).unwrap().what_if_tables[0].anchor.row = 2, RecoveryModelError::WhatIf),
        ("auto filter", |w| w.sheet_mut(0).unwrap().autofilter.as_mut().unwrap().criteria[0].0 = MAX_COLS, RecoveryModelError::Filter),
        ("conditional range", |w| w.sheet_mut(0).unwrap().cond_formats[0].ranges[0].end.col = MAX_COLS, RecoveryModelError::Coordinate),
        (
            "conditional dxf",
            |w| {
                w.sheet_mut(0).unwrap().cond_formats[0].rule =
                    CfRule::Blanks { style: Box::new(Style { font: Font { size: f32::NAN, ..Font::default() }, ..Style::default() }) }
            },
            RecoveryModelError::Style,
        ),
        ("validation", |w| w.sheet_mut(0).unwrap().validations[0].ranges[0].start.row = MAX_ROWS, RecoveryModelError::Coordinate),
        ("pivot position", |w| w.sheet_mut(0).unwrap().pivots[0].values_position = Some(2), RecoveryModelError::Pivot),
        ("chart anchor", |w| w.sheet_mut(0).unwrap().charts[0].anchor.width = -1.0, RecoveryModelError::Geometry),
        ("series color", |w| w.sheet_mut(0).unwrap().charts[0].series[0].color = Some(Color::Rgb(0x1000000)), RecoveryModelError::Style),
        ("trend finite", |w| w.sheet_mut(0).unwrap().charts[0].series[0].trendlines[0].intercept = Some(f64::NAN), RecoveryModelError::Value),
        ("image anchor", |w| w.sheet_mut(0).unwrap().images[0].anchor.cell.row = MAX_ROWS, RecoveryModelError::Coordinate),
        ("shape font", |w| w.sheet_mut(0).unwrap().shapes[0].format.font_size = Some(f32::INFINITY), RecoveryModelError::Style),
        ("sparkline color", |w| w.sheet_mut(0).unwrap().sparklines[0].color = Color::Theme(0, 1001), RecoveryModelError::Style),
        ("slicer anchor", |w| w.sheet_mut(0).unwrap().slicers[0].anchor.height = -1.0, RecoveryModelError::Geometry),
        ("control numbers", |w| w.sheet_mut(0).unwrap().controls[0].step = 0, RecoveryModelError::Control),
        (
            "note size",
            |w| w.sheet_mut(0).unwrap().comments_mut().get_mut(&at(1, 1)).unwrap().note_box.as_mut().unwrap().size_pt = Some((f64::NAN, 1.0)),
            RecoveryModelError::Geometry,
        ),
        (
            "hyperlink key",
            |w| {
                let h = w.sheet_mut(0).unwrap().hyperlinks.remove(&at(1, 1)).unwrap();
                w.sheet_mut(0).unwrap().hyperlinks.insert(at(MAX_ROWS, 0), h);
            },
            RecoveryModelError::Coordinate,
        ),
        (
            "picture key",
            |w| {
                let p = w.sheet_mut(0).unwrap().cell_pictures.remove(&at(1, 1)).unwrap();
                w.sheet_mut(0).unwrap().cell_pictures.insert(at(0, MAX_COLS), p);
            },
            RecoveryModelError::Coordinate,
        ),
        ("print margin", |w| w.sheet_mut(0).unwrap().print.margins[5] = -1.0, RecoveryModelError::Print),
        ("print break", |w| w.sheet_mut(0).unwrap().print.col_breaks[0] = MAX_COLS, RecoveryModelError::Print),
        ("edit range", |w| w.sheet_mut(0).unwrap().edit_ranges[0].ranges[0].end.row = MAX_ROWS, RecoveryModelError::Coordinate),
        ("phonetic offset", |w| w.phonetics[0].runs[0].end = 5, RecoveryModelError::Phonetic),
    ];
    for (label, mutate, error) in cases {
        let mut wb = inventory_workbook();
        assert_eq!(crate::validate_recovery_model(&wb), Ok(()), "neighbor {label}");
        mutate(&mut wb);
        assert_eq!(crate::validate_recovery_model(&wb), Err(*error), "{label}");
    }
    for family in 0..8 {
        let mut wb = inventory_workbook();
        assert_eq!(crate::validate_recovery_model(&wb), Ok(()));
        let sheet = wb.sheet_mut(0).unwrap();
        match family {
            0 => {
                let v = sheet.tables[0].clone();
                sheet.tables.push(v);
            }
            1 => {
                let v = sheet.pivots[0].clone();
                sheet.pivots.push(v);
            }
            2 => sheet.shapes[0].id = sheet.charts[0].id,
            3 => {
                let v = sheet.merges[0];
                sheet.merges.push(v);
            }
            4 => {
                let v = sheet.what_if_tables[0].clone();
                sheet.what_if_tables.push(v);
            }
            5 => {
                let v = wb.names[0].clone();
                wb.names.push(v);
            }
            6 => {
                let v = wb.phonetics[0].clone();
                wb.phonetics.push(v);
            }
            7 => {
                let v = wb.cell_styles[0].clone();
                wb.cell_styles.push(v);
            }
            _ => panic!("fixture family"),
        }
        assert!(matches!(crate::validate_recovery_model(&wb), Err(RecoveryModelError::UnsupportedRepresentation(_))));
    }
}

#[test]
fn recovery_public_api_raw_store_precedes_every_semantic_or_fallback_check() {
    let mut wb = inventory_workbook();
    wb.calc.max_change = f64::INFINITY;
    wb.sheet_mut(0).unwrap().cells = serde_json::from_str("{\"bands\":{},\"len\":1}").unwrap();
    assert_eq!(crate::validate_recovery_model(&wb), Err(RecoveryModelError::Storage));
    assert_eq!(wb.sheets[0].cells.len(), 1); // No rebuild repairs the false cached count.
}

#[test]
fn recovery_public_api_never_walks_or_serializes_unbudgeted_executable_json() {
    let mut wb = Workbook::new();
    let mut nested = serde_json::Value::Null;
    for _ in 0..2048 {
        nested = serde_json::Value::Array(vec![nested]);
    }
    wb.macros = vec![Macro { v: u32::MAX, name: String::new(), steps: vec![Step::command("", nested)], ..Macro::default() }].into();
    wb.macros_read = vec![serde_json::json!({"pending": true})];
    wb.macros_unread = Arc::new(vec![serde_json::json!({"future": 99})]);
    let StepKind::Command { params, id: _ } = &wb.macros[0].steps[0].kind else { panic!("command") };
    let pointer = params as *const _;
    let pending = wb.macros_read.as_ptr();
    let future = Arc::as_ptr(&wb.macros_unread);
    assert_eq!(crate::validate_recovery_model(&wb), Ok(()));
    let StepKind::Command { params, id: _ } = &wb.macros[0].steps[0].kind else { panic!("command") };
    assert_eq!(params as *const _, pointer);
    assert_eq!((wb.macros_read.as_ptr(), Arc::as_ptr(&wb.macros_unread)), (pending, future));
    // Dismantle our own synthetic tree iteratively; do not exercise recursive JSON Drop.
    let StepKind::Command { params, id: _ } = &mut wb.macros[0].steps[0].kind else { panic!("command") };
    let mut owned = std::mem::take(params);
    while let serde_json::Value::Array(mut child) = owned {
        owned = child.pop().unwrap();
    }
}

macro_rules! accept {
    ($values:expr, |$w:ident, $value:ident| $body:expr) => {
        for $value in $values {
            let mut $w = inventory_workbook();
            $body;
            assert_eq!(crate::validate_recovery_model(&$w), Ok(()));
        }
    };
}

#[test]
fn recovery_public_api_accepts_every_workbook_cell_scalar_enum_alternative() {
    accept!([CalcMode::Automatic, CalcMode::AutomaticExceptTables, CalcMode::Manual], |w, v| w.calc.mode = v);
    accept!([ShowObjects::All, ShowObjects::Placeholders, ShowObjects::None], |w, v| w.show_objects = v);
    accept!([DateSystem::D1900, DateSystem::D1904], |w, v| w.date_system = v);
    accept!([Visibility::Visible, Visibility::Hidden, Visibility::VeryHidden], |w, v| w.sheet_mut(0).unwrap().visibility = v);
    accept!([SheetViewMode::Normal, SheetViewMode::PageLayout, SheetViewMode::PageBreakPreview], |w, v| w.sheet_mut(0).unwrap().view = v);
    accept!([FormulaEvalMode::Array, FormulaEvalMode::Legacy], |w, v| Arc::make_mut(
        w.sheet_mut(0).unwrap().cells.get_mut(at(0, 0)).unwrap().formula.as_mut().unwrap()
    )
    .eval_mode = v);
    for error in CellError::ALL.into_iter().chain([CellError::Circ]) {
        let mut wb = inventory_workbook();
        wb.sheet_mut(0).unwrap().cells.get_mut(at(0, 1)).unwrap().value = Value::Error(error);
        assert_eq!(crate::validate_recovery_model(&wb), Ok(()));
    }
}

#[test]
fn recovery_public_api_accepts_every_style_scalar_enum_alternative() {
    accept!([Color::Auto, Color::Rgb(0xffffff), Color::Theme(11, 1000)], |w, v| w.cell_styles[0].1.font.color = v);
    accept!([Underline::None, Underline::Single, Underline::Double, Underline::SingleAccounting, Underline::DoubleAccounting], |w, v| w
        .cell_styles[0]
        .1
        .font
        .underline =
        v);
    accept!([VertAlign::Baseline, VertAlign::Superscript, VertAlign::Subscript], |w, v| w.cell_styles[0].1.font.vert = v);
    accept!(
        [
            PatternType::None,
            PatternType::Solid,
            PatternType::Gray50,
            PatternType::Gray75,
            PatternType::Gray25,
            PatternType::Gray125,
            PatternType::Gray0625,
            PatternType::DarkHorizontal,
            PatternType::DarkVertical,
            PatternType::DarkDown,
            PatternType::DarkUp,
            PatternType::DarkGrid,
            PatternType::DarkTrellis,
            PatternType::LightHorizontal,
            PatternType::LightVertical,
            PatternType::LightDown,
            PatternType::LightUp,
            PatternType::LightGrid,
            PatternType::LightTrellis
        ],
        |w, v| w.cell_styles[0].1.fill.pattern = v
    );
    accept!(
        [
            BorderStyle::None,
            BorderStyle::Thin,
            BorderStyle::Medium,
            BorderStyle::Thick,
            BorderStyle::Dashed,
            BorderStyle::Dotted,
            BorderStyle::Double,
            BorderStyle::Hair,
            BorderStyle::MediumDashed,
            BorderStyle::DashDot,
            BorderStyle::MediumDashDot,
            BorderStyle::DashDotDot,
            BorderStyle::MediumDashDotDot,
            BorderStyle::SlantDashDot
        ],
        |w, v| w.cell_styles[0].1.borders.left.style = v
    );
    accept!(
        [HAlign::General, HAlign::Left, HAlign::Center, HAlign::Right, HAlign::Fill, HAlign::Justify, HAlign::CenterAcross, HAlign::Distributed],
        |w, v| w.cell_styles[0].1.align.h = v
    );
    accept!([VAlign::Top, VAlign::Center, VAlign::Bottom, VAlign::Justify, VAlign::Distributed], |w, v| w.cell_styles[0].1.align.v = v);
}

#[test]
fn recovery_public_api_accepts_every_feature_scalar_enum_alternative() {
    accept!(
        [
            TotalsFn::None,
            TotalsFn::Average,
            TotalsFn::Count,
            TotalsFn::CountNums,
            TotalsFn::Max,
            TotalsFn::Min,
            TotalsFn::Sum,
            TotalsFn::StdDev,
            TotalsFn::Var,
            TotalsFn::Custom
        ],
        |w, v| w.sheet_mut(0).unwrap().tables[0].columns[0].totals = v
    );
    accept!(
        [
            CfOperator::Between,
            CfOperator::NotBetween,
            CfOperator::Equal,
            CfOperator::NotEqual,
            CfOperator::Greater,
            CfOperator::Less,
            CfOperator::GreaterOrEqual,
            CfOperator::LessOrEqual
        ],
        |w, v| w.sheet_mut(0).unwrap().validations[0].op = v
    );
    accept!(
        [
            ValidationKind::Any,
            ValidationKind::Whole,
            ValidationKind::Decimal,
            ValidationKind::List,
            ValidationKind::Date,
            ValidationKind::Time,
            ValidationKind::TextLength,
            ValidationKind::Custom
        ],
        |w, v| w.sheet_mut(0).unwrap().validations[0].kind = v
    );
    accept!([ErrorStyle::Stop, ErrorStyle::Warning, ErrorStyle::Information], |w, v| w.sheet_mut(0).unwrap().validations[0].error_style = v);
    accept!([PivotLayout::Compact, PivotLayout::Outline, PivotLayout::Tabular], |w, v| w.sheet_mut(0).unwrap().pivots[0].layout = v);
    accept!([PivotSort::Asc, PivotSort::Desc, PivotSort::None], |w, v| w.sheet_mut(0).unwrap().pivots[0].rows[0].sort = v);
    accept!([PivotDateGroup::None, PivotDateGroup::Years, PivotDateGroup::Quarters, PivotDateGroup::Months, PivotDateGroup::Days], |w, v| w
        .sheet_mut(0)
        .unwrap()
        .pivots[0]
        .rows[0]
        .date_group =
        v);
    accept!(
        [
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
            PivotFunc::VarP
        ],
        |w, v| w.sheet_mut(0).unwrap().pivots[0].values[0].func = v
    );
    accept!(
        [
            PivotShowAs::Normal,
            PivotShowAs::PercentOfGrandTotal,
            PivotShowAs::PercentOfColumnTotal,
            PivotShowAs::PercentOfRowTotal,
            PivotShowAs::RunningTotal,
            PivotShowAs::Rank
        ],
        |w, v| w.sheet_mut(0).unwrap().pivots[0].values[0].show_as = v
    );
}

#[test]
fn recovery_public_api_accepts_every_drawing_scalar_enum_alternative() {
    accept!([AnchorMode::MoveAndSize, AnchorMode::MoveOnly, AnchorMode::Absolute], |w, v| w.sheet_mut(0).unwrap().images[0].anchor.mode = v);
    accept!(
        [
            ChartKind::ColumnClustered,
            ChartKind::ColumnStacked,
            ChartKind::ColumnStacked100,
            ChartKind::BarClustered,
            ChartKind::BarStacked,
            ChartKind::BarStacked100,
            ChartKind::Line,
            ChartKind::LineMarkers,
            ChartKind::LineStacked,
            ChartKind::Pie,
            ChartKind::Doughnut,
            ChartKind::Area,
            ChartKind::AreaStacked,
            ChartKind::AreaStacked100,
            ChartKind::Scatter,
            ChartKind::ScatterLines,
            ChartKind::Bubble,
            ChartKind::Radar,
            ChartKind::Histogram,
            ChartKind::Waterfall,
            ChartKind::Funnel,
            ChartKind::Treemap,
            ChartKind::Sunburst,
            ChartKind::BoxWhisker,
            ChartKind::Stock,
            ChartKind::Combo
        ],
        |w, v| w.sheet_mut(0).unwrap().charts[0].kind = v
    );
    accept!([LegendPos::None, LegendPos::Bottom, LegendPos::Top, LegendPos::Left, LegendPos::Right], |w, v| w.sheet_mut(0).unwrap().charts[0]
        .legend = v);
    accept!([QuartileMethod::Exclusive, QuartileMethod::Inclusive], |w, v| w.sheet_mut(0).unwrap().charts[0].series[0].quartiles = v);
    accept!(
        [
            TrendlineKind::Linear,
            TrendlineKind::Exponential,
            TrendlineKind::Logarithmic,
            TrendlineKind::Polynomial(0),
            TrendlineKind::Power,
            TrendlineKind::MovingAverage(u32::MAX)
        ],
        |w, v| w.sheet_mut(0).unwrap().charts[0].series[0].trendlines[0].kind = v
    );
    accept!(
        [
            LineDash::Solid,
            LineDash::Dot,
            LineDash::Dash,
            LineDash::LgDash,
            LineDash::DashDot,
            LineDash::LgDashDot,
            LineDash::LgDashDotDot,
            LineDash::SysDash,
            LineDash::SysDot,
            LineDash::SysDashDot,
            LineDash::SysDashDotDot
        ],
        |w, v| w.sheet_mut(0).unwrap().charts[0].series[0].trendlines[0].dash = Some(v)
    );
    accept!(
        [
            ShapeKind::Rectangle,
            ShapeKind::RoundedRectangle,
            ShapeKind::Ellipse,
            ShapeKind::Triangle,
            ShapeKind::Line,
            ShapeKind::Arrow,
            ShapeKind::TextBox,
            ShapeKind::Icon,
            ShapeKind::Ink,
            ShapeKind::Equation
        ],
        |w, v| w.sheet_mut(0).unwrap().shapes[0].kind = v
    );
    accept!([ControlKind::Button, ControlKind::Slider, ControlKind::Spin], |w, v| w.sheet_mut(0).unwrap().controls[0].kind = v);
    accept!([ShapeTextAlign::Left, ShapeTextAlign::Center, ShapeTextAlign::Right], |w, v| w.sheet_mut(0).unwrap().shapes[0].format.align = Some(v));
    accept!([ShapeTextAnchor::Top, ShapeTextAnchor::Middle, ShapeTextAnchor::Bottom], |w, v| w.sheet_mut(0).unwrap().shapes[0].format.anchor =
        Some(v));
    accept!([SparklineKind::Line, SparklineKind::Column, SparklineKind::WinLoss], |w, v| w.sheet_mut(0).unwrap().sparklines[0].kind = v);
}

#[test]
fn recovery_public_api_accepts_every_metadata_macro_scalar_enum_alternative() {
    accept!([Orientation::Portrait, Orientation::Landscape], |w, v| w.sheet_mut(0).unwrap().print.orientation = v);
    accept!([PageOrder::DownThenOver, PageOrder::OverThenDown], |w, v| w.sheet_mut(0).unwrap().print.page_order = v);
    accept!([InputKind::Cell, InputKind::Range], |w, v| w.macros[0].inputs[0].kind = v);
    accept!([OnConflict::Ask, OnConflict::Fail, OnConflict::Overwrite, OnConflict::ShiftDown, OnConflict::ShiftRight], |w, v| w.macros[0]
        .output
        .on_conflict = v);
}

#[test]
fn recovery_public_api_payload_variants_and_metadata_modes_are_retained() {
    let style = Box::new(Style::default());
    let rules = vec![
        CfRule::CellIs { op: CfOperator::Between, a: "bad(".into(), b: None, style: style.clone() },
        CfRule::Expression { formula: "bad(".into(), style: style.clone() },
        CfRule::ContainsText { text: "raw".into(), style: style.clone() },
        CfRule::NotContainsText { text: "raw".into(), style: style.clone() },
        CfRule::BeginsWith { text: "raw".into(), style: style.clone() },
        CfRule::EndsWith { text: "raw".into(), style: style.clone() },
        CfRule::Blanks { style: style.clone() },
        CfRule::NoBlanks { style: style.clone() },
        CfRule::Errors { style: style.clone() },
        CfRule::NoErrors { style: style.clone() },
        CfRule::Duplicate { style: style.clone() },
        CfRule::Unique { style: style.clone() },
        CfRule::Top10 { bottom: true, percent: true, rank: u32::MAX, style: style.clone() },
        CfRule::AboveAverage { below: true, equal: true, std_dev: u8::MAX, style: style.clone() },
        CfRule::TimePeriod { period: "future".into(), style },
        CfRule::ColorScale {
            stops: vec![
                (CfValueKind::Min, Color::Auto),
                (CfValueKind::Max, Color::Auto),
                (CfValueKind::Number(-0.0), Color::Auto),
                (CfValueKind::Percent(-999.0), Color::Auto),
                (CfValueKind::Percentile(f64::MAX), Color::Auto),
                (CfValueKind::Formula("bad(".into()), Color::Auto),
            ],
        },
        CfRule::DataBar { min: CfValueKind::Min, max: CfValueKind::Max, color: Color::Auto, gradient: false, show_value: false },
        CfRule::IconSet { set: "future".into(), thresholds: vec![], reverse: true, show_value: false },
    ];
    for rule in rules {
        let mut wb = inventory_workbook();
        wb.sheet_mut(0).unwrap().cond_formats[0].rule = rule;
        assert_eq!(crate::validate_recovery_model(&wb), Ok(()));
    }
    for criterion in [
        FilterCriterion::Values { values: vec!["raw".into()], blanks: true },
        FilterCriterion::Custom { a: ("?".into(), "bad(".into()), b: Some(("?".into(), "raw".into())), and: false },
        FilterCriterion::Top10 { bottom: true, percent: true, count: u32::MAX },
        FilterCriterion::AboveAverage(true),
        FilterCriterion::FillColor(Color::Auto),
        FilterCriterion::FontColor(Color::Auto),
        FilterCriterion::Icon { set: "future".into(), id: None },
    ] {
        let mut wb = inventory_workbook();
        wb.sheet_mut(0).unwrap().autofilter.as_mut().unwrap().criteria[0].1 = criterion;
        assert_eq!(crate::validate_recovery_model(&wb), Ok(()));
    }
    for object in [false, true] {
        for stale in [false, true] {
            for convertible in [false, true] {
                let mut wb = inventory_workbook();
                let py = Arc::make_mut(wb.sheet_mut(0).unwrap().cells.get_mut(at(1, 0)).unwrap().py.as_mut().unwrap());
                py.object = object;
                py.stale = stale;
                py.output.as_mut().unwrap().convertible = convertible;
                assert_eq!(crate::validate_recovery_model(&wb), Ok(()));
            }
        }
    }
    for action in [
        ControlAction::Macro { name: "missing".into(), input: MacroInput::Selection, output: None },
        ControlAction::Macro { name: "missing".into(), input: MacroInput::Fixed("bad(".into()), output: Some("raw".into()) },
        ControlAction::Command { id: "missing".into(), params: serde_json::json!({"opaque": []}) },
    ] {
        let mut wb = inventory_workbook();
        wb.sheet_mut(0).unwrap().controls[0].action = Some(action);
        assert_eq!(crate::validate_recovery_model(&wb), Ok(()));
    }
    for of in [RepeatOf::Rows, RepeatOf::Columns] {
        let mut wb = inventory_workbook();
        wb.macros[0].steps[0].kind = StepKind::Repeat { over: "bad(".into(), of, max: u32::MAX, body: vec![] };
        assert_eq!(crate::validate_recovery_model(&wb), Ok(()));
    }
    for op in [
        CompareOp::Eq,
        CompareOp::Ne,
        CompareOp::Gt,
        CompareOp::Ge,
        CompareOp::Lt,
        CompareOp::Le,
        CompareOp::Empty,
        CompareOp::NotEmpty,
        CompareOp::Contains,
    ] {
        let mut wb = inventory_workbook();
        wb.macros[0].steps[0].kind =
            StepKind::If { cell: "bad(".into(), op, value: serde_json::json!({"opaque": true}), then: vec![], otherwise: vec![] };
        assert_eq!(crate::validate_recovery_model(&wb), Ok(()));
    }
    let mut wb = inventory_workbook();
    wb.macros[0].steps[0].kind = StepKind::Stop { message: "raw".into(), rollback: true };
    assert_eq!(crate::validate_recovery_model(&wb), Ok(()));
    for value in [
        Value::Empty,
        Value::Number(-0.0),
        Value::from("raw"),
        Value::Bool(false),
        Value::Error(CellError::Circ),
        Value::Array(Arc::new(Array::scalar(Value::Empty))),
    ] {
        wb.sheet_mut(0).unwrap().cells.get_mut(at(0, 1)).unwrap().value = value;
        assert_eq!(crate::validate_recovery_model(&wb), Ok(()));
    }
    wb.sheet_mut(0).unwrap().edit_ranges.push(EditRange { title: "".into(), ranges: vec![], password: None, security_descriptor: None });
    wb.phonetics.push(PhoneticString { text: "".into(), runs: vec![], props: Some(PhoneticProps::default()) });
    wb.sheet_mut(0).unwrap().print.fit_to = Some((0, 0));
    for page in [HfPage::Odd, HfPage::Even, HfPage::First] {
        *wb.sheet_mut(0).unwrap().print.hf_mut(page, false) = String::new();
        assert_eq!(crate::validate_recovery_model(&wb), Ok(()));
    }
}

#[test]
fn recovery_public_api_python_output_limit_and_nan_payload_are_preserved_on_refusal() {
    let mut wb = inventory_workbook();
    let py = Arc::make_mut(wb.sheet_mut(0).unwrap().cells.get_mut(at(1, 0)).unwrap().py.as_mut().unwrap());
    let limit = crate::python::MAX_RESULT_CELLS;
    py.output.as_mut().unwrap().value = Value::Array(Arc::new(Array { rows: 1, cols: limit, data: vec![Value::Empty; limit] }));
    assert_eq!(crate::validate_recovery_model(&wb), Ok(()));
    let py = Arc::make_mut(wb.sheet_mut(0).unwrap().cells.get_mut(at(1, 0)).unwrap().py.as_mut().unwrap());
    py.output.as_mut().unwrap().value = Value::Array(Arc::new(Array { rows: 1, cols: limit + 1, data: vec![Value::Empty; limit + 1] }));
    assert_eq!(crate::validate_recovery_model(&wb), Err(RecoveryModelError::Python));
    let nan = f64::from_bits(0x7ff8_0000_0000_0042);
    let py = Arc::make_mut(wb.sheet_mut(0).unwrap().cells.get_mut(at(1, 0)).unwrap().py.as_mut().unwrap());
    py.output.as_mut().unwrap().value = Value::Number(nan);
    let pointer = Arc::as_ptr(wb.sheets[0].cells.get(at(1, 0)).unwrap().py.as_ref().unwrap());
    assert_eq!(crate::validate_recovery_model(&wb), Err(RecoveryModelError::Value));
    let after = wb.sheets[0].cells.get(at(1, 0)).unwrap().py.as_ref().unwrap();
    assert_eq!(Arc::as_ptr(after), pointer);
    let Value::Number(n) = after.output.as_ref().unwrap().value else { panic!("number") };
    assert_eq!(n.to_bits(), nan.to_bits());
}
