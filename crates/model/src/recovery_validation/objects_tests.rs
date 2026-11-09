//! B3 contract fixtures. Dropping a family check, normalizing retained content, or inspecting
//! inert action JSON must fail these tests; model success never establishes projection safety.
use super::*;
use crate::{CellPicture, Color, EquationCache, LineInfo, NoteBox, NoteCorner, Sheet};
use gridcraft_core::{CellRef, MAX_COLS, MAX_ROWS};
use std::sync::Arc;

const CHARTS: [ChartKind; 26] = [
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
    ChartKind::Combo,
];
const SHAPES: [ShapeKind; 10] = [
    ShapeKind::Rectangle,
    ShapeKind::RoundedRectangle,
    ShapeKind::Ellipse,
    ShapeKind::Triangle,
    ShapeKind::Line,
    ShapeKind::Arrow,
    ShapeKind::TextBox,
    ShapeKind::Icon,
    ShapeKind::Ink,
    ShapeKind::Equation,
];
fn chart(id: u32) -> Chart {
    Chart {
        id,
        kind: ChartKind::Combo,
        anchor: Anchor::default(),
        title: Some("retained".into()),
        series: vec![Series {
            name: Some("[external]missing!broken(".into()),
            categories: Some("#REF!".into()),
            values: "not a formula".into(),
            bubble_sizes: Some("[outside]sizes".into()),
            color: Some(Color::Theme(11, -1000)),
            secondary: true,
            kind: Some(ChartKind::Bubble),
            smooth: true,
            quartiles: QuartileMethod::Inclusive,
            trendlines: vec![Trendline {
                kind: TrendlineKind::Polynomial(255),
                name: Some("raw".into()),
                forward: -f64::MAX,
                backward: f64::MAX,
                intercept: Some(-0.0),
                display_equation: true,
                display_r_squared: true,
                color: Some(Color::Auto),
                width: Some(-0.0),
                dash: Some(LineDash::SysDashDotDot),
            }],
        }],
        legend: LegendPos::None,
        data_labels: true,
        data_table: Some(ChartDataTable { show_keys: false, horizontal_border: true, vertical_border: false, outline: true }),
        gridlines: false,
        style: u32::MAX,
        hole_size: 255,
        first_slice_angle: u16::MAX,
        x_title: Some("x".into()),
        y_title: Some("y".into()),
        source: Some("#REF!".into()),
        by_rows: true,
        pivot: Some("[outside.xlsx]Missing!Pivot".into()),
    }
}
fn shape(id: u32) -> Shape {
    let mut math = EquationCache::default();
    math.refuse("unparseable(", "retained derived refusal");
    Shape {
        id,
        kind: ShapeKind::Equation,
        anchor: Anchor::default(),
        fill: Color::Rgb(0x123456),
        line: Color::Auto,
        text: "unparseable(".into(),
        math,
        format: ShapeFormat {
            no_line: true,
            font_size: Some(-f32::MAX),
            bold: true,
            italic: true,
            font: Some("unknown retained font".into()),
            text_color: Some(Color::Theme(0, 1000)),
            text_outline: Some((Color::Rgb(0), -0.0)),
            align: Some(ShapeTextAlign::Right),
            anchor: Some(ShapeTextAnchor::Bottom),
            rtl: true,
        },
    }
}
fn note() -> Comment {
    Comment {
        author: "author".into(),
        text: "text".into(),
        replies: vec![("reply".into(), "body".into())],
        threaded: true,
        resolved: true,
        visible: true,
        dates: vec!["not a date".into(), "".into(), "extra".into()],
        note_box: Some(NoteBox {
            from: NoteCorner { col: i64::MAX, col_px: u32::MAX, row: i64::MIN, row_px: 0 },
            to: NoteCorner { col: i64::MIN, col_px: 0, row: i64::MAX, row_px: u32::MAX },
            size_pt: Some((-0.0, f64::MAX)),
        }),
    }
}
fn floating(sheet: &mut Sheet, kind: usize, id: u32) {
    match kind {
        0 => sheet.charts.push(chart(id)),
        1 => sheet.images.push(Image {
            id,
            anchor: Anchor::default(),
            data: Arc::new(vec![0, 255, 1]),
            mime: "unknown/inert".into(),
            alt: "raw".into(),
        }),
        2 => sheet.shapes.push(shape(id)),
        3 => sheet.slicers.push(Slicer {
            id,
            table: "missing".into(),
            column: "absent".into(),
            caption: "raw".into(),
            anchor: Anchor::default(),
            selected: Some(vec![]),
            columns: 0,
        }),
        4 => sheet.controls.push(FormControl { id, ..FormControl::default() }),
        _ => panic!("fixture kind"),
    }
}
fn anchor_mut(sheet: &mut Sheet, kind: usize) -> &mut Anchor {
    match kind {
        0 => &mut sheet.charts[0].anchor,
        1 => &mut sheet.images[0].anchor,
        2 => &mut sheet.shapes[0].anchor,
        3 => &mut sheet.slicers[0].anchor,
        4 => &mut sheet.controls[0].anchor,
        _ => panic!("fixture kind"),
    }
}

#[test]
fn recovery_objects_all_chart_trend_and_drawing_variants_preserve_inert_content() {
    let mut wb = Workbook::new();
    let sheet = wb.sheet_mut(0).unwrap();
    sheet.charts.push(chart(0));
    sheet.shapes.push(shape(1));
    for kind in CHARTS {
        let sheet = wb.sheet_mut(0).unwrap();
        sheet.charts[0].kind = kind;
        sheet.charts[0].series[0].kind = Some(kind);
        let before = wb.clone();
        assert_eq!(validate_workbook_objects(&wb), Ok(()));
        assert_eq!(wb, before);
    }
    for kind in [
        TrendlineKind::Linear,
        TrendlineKind::Exponential,
        TrendlineKind::Logarithmic,
        TrendlineKind::Polynomial(0),
        TrendlineKind::Polynomial(255),
        TrendlineKind::Power,
        TrendlineKind::MovingAverage(0),
        TrendlineKind::MovingAverage(u32::MAX),
    ] {
        wb.sheet_mut(0).unwrap().charts[0].series[0].trendlines[0].kind = kind;
        assert_eq!(validate_workbook_objects(&wb), Ok(()));
    }
    for dash in [
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
        LineDash::SysDashDotDot,
    ] {
        wb.sheet_mut(0).unwrap().charts[0].series[0].trendlines[0].dash = Some(dash);
        assert_eq!(validate_workbook_objects(&wb), Ok(()));
    }
    for legend in [LegendPos::None, LegendPos::Bottom, LegendPos::Top, LegendPos::Left, LegendPos::Right] {
        wb.sheet_mut(0).unwrap().charts[0].legend = legend;
        assert_eq!(validate_workbook_objects(&wb), Ok(()));
    }
    for kind in SHAPES {
        wb.sheet_mut(0).unwrap().shapes[0].kind = kind;
        assert_eq!(validate_workbook_objects(&wb), Ok(()));
    }
    let sheet = wb.sheet_mut(0).unwrap();
    sheet.charts[0].series[0] = Series::default(); // all optional fields absent, exclusive quartiles
    sheet.charts[0].data_table = None;
    sheet.shapes[0].format = ShapeFormat::default();
    assert_eq!(validate_workbook_objects(&wb), Ok(()));
    for align in [ShapeTextAlign::Left, ShapeTextAlign::Center, ShapeTextAlign::Right] {
        for anchor in [ShapeTextAnchor::Top, ShapeTextAnchor::Middle, ShapeTextAnchor::Bottom] {
            let fmt = &mut wb.sheet_mut(0).unwrap().shapes[0].format;
            fmt.align = Some(align);
            fmt.anchor = Some(anchor);
            assert_eq!(validate_workbook_objects(&wb), Ok(()));
        }
    }
}

#[test]
fn recovery_objects_ids_share_one_workbook_namespace_and_zero_is_not_ambiguous_alone() {
    for first in 0..5 {
        for second in 0..5 {
            for cross_sheet in [false, true] {
                let mut wb = Workbook::new();
                floating(wb.sheet_mut(0).unwrap(), first, 0);
                if cross_sheet {
                    wb.sheets.push(Arc::new(Sheet::new("other")));
                }
                floating(wb.sheet_mut(usize::from(cross_sheet)).unwrap(), second, 1);
                assert_eq!(validate_workbook_objects(&wb), Ok(()));
                let sheet = wb.sheet_mut(usize::from(cross_sheet)).unwrap();
                match second {
                    0 => sheet.charts.last_mut().unwrap().id = 0,
                    1 => sheet.images.last_mut().unwrap().id = 0,
                    2 => sheet.shapes.last_mut().unwrap().id = 0,
                    3 => sheet.slicers.last_mut().unwrap().id = 0,
                    4 => sheet.controls.last_mut().unwrap().id = 0,
                    _ => panic!("unexpected fixture selector"),
                }
                assert_eq!(
                    validate_workbook_objects(&wb),
                    Err(RecoveryModelError::UnsupportedRepresentation(super::super::RecoveryRepresentation::AmbiguousIdentity))
                );
            }
        }
    }
}

#[test]
fn recovery_objects_table_pivot_and_floating_ids_remain_separate() {
    let mut wb = Workbook::new();
    let sheet = wb.sheet_mut(0).unwrap();
    floating(sheet, 0, 0);
    sheet.pivots.push(crate::PivotTable::default());
    sheet.tables.push(crate::Table {
        id: 0,
        name: "independent".into(),
        range: gridcraft_core::RangeRef::cell(CellRef::new(0, 0)),
        header_row: true,
        totals_row: false,
        columns: vec![crate::TableColumn {
            name: "field".into(),
            totals: crate::TotalsFn::None,
            totals_label: None,
            formula: None,
            unique_name: None,
            xml: None,
        }],
        style: "".into(),
        banded_rows: false,
        banded_cols: false,
        first_col: false,
        last_col: false,
        filter_button: false,
        filter: vec![],
        table_type: None,
        connection_id: None,
    });
    assert_eq!(super::super::features::validate_workbook_features(&wb), Ok(()));
    assert_eq!(validate_workbook_objects(&wb), Ok(()));
}

#[test]
fn recovery_objects_anchor_modes_extreme_geometry_and_zero_extents_keep_bits() {
    let mut wb = Workbook::new();
    let sheet = wb.sheet_mut(0).unwrap();
    sheet.default_row_height = f32::MAX;
    sheet.rows.insert(3, LineInfo { size: Some(f32::MAX / 2.0), ..LineInfo::default() });
    // Independent f64 arithmetic, without LineIndex/layout: all but one rows use default size.
    let extent = f64::from(MAX_ROWS - 1) * f64::from(f32::MAX) + f64::from(f32::MAX / 2.0);
    assert!(extent.is_finite() && extent > f64::from(f32::MAX));
    assert_eq!(super::super::structure::validate_workbook_structure(&wb), Ok(1));
    for mode in [AnchorMode::MoveAndSize, AnchorMode::MoveOnly, AnchorMode::Absolute] {
        for (dx, dy, width, height) in [(f32::MAX, -f32::MAX, f32::MAX, f32::MAX), (-0.0, 0.0, -0.0, 0.0)] {
            let sheet = wb.sheet_mut(0).unwrap();
            sheet.images = vec![Image {
                id: 0,
                anchor: Anchor { cell: CellRef::new(MAX_ROWS - 1, MAX_COLS - 1), dx, dy, width, height, mode },
                data: Arc::new(vec![]),
                mime: "".into(),
                alt: "".into(),
            }];
            assert_eq!(validate_workbook_objects(&wb), Ok(()));
            let after = wb.sheet(0).unwrap().images[0].anchor;
            assert_eq!(
                [after.dx.to_bits(), after.dy.to_bits(), after.width.to_bits(), after.height.to_bits()],
                [dx.to_bits(), dy.to_bits(), width.to_bits(), height.to_bits()]
            );
            assert_eq!(after.mode, mode);
        }
    }
}

#[test]
fn recovery_objects_every_anchor_family_rejects_bad_cell_and_numeric_geometry() {
    for kind in 0..5 {
        for field in 0..4 {
            for n in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, -1.0] {
                if field < 2 && n == -1.0 {
                    continue;
                }
                let mut wb = Workbook::new();
                floating(wb.sheet_mut(0).unwrap(), kind, 0);
                let anchor = anchor_mut(wb.sheet_mut(0).unwrap(), kind);
                match field {
                    0 => anchor.dx = n,
                    1 => anchor.dy = n,
                    2 => anchor.width = n,
                    3 => anchor.height = n,
                    _ => panic!("unexpected fixture selector"),
                }
                assert_eq!(validate_workbook_objects(&wb), Err(RecoveryModelError::Geometry));
                assert_eq!(
                    match field {
                        0 => anchor_mut(wb.sheet_mut(0).unwrap(), kind).dx,
                        1 => anchor_mut(wb.sheet_mut(0).unwrap(), kind).dy,
                        2 => anchor_mut(wb.sheet_mut(0).unwrap(), kind).width,
                        _ => anchor_mut(wb.sheet_mut(0).unwrap(), kind).height,
                    }
                    .to_bits(),
                    n.to_bits()
                );
            }
        }
        for at in [CellRef::new(MAX_ROWS, 0), CellRef::new(0, MAX_COLS)] {
            let mut wb = Workbook::new();
            floating(wb.sheet_mut(0).unwrap(), kind, 0);
            anchor_mut(wb.sheet_mut(0).unwrap(), kind).cell = at;
            assert_eq!(validate_workbook_objects(&wb), Err(RecoveryModelError::Coordinate));
        }
    }
}

#[test]
fn recovery_objects_trend_and_shape_scalar_faults_are_refused_without_editor_bounds() {
    for n in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        for field in 0..3 {
            let mut wb = Workbook::new();
            wb.sheet_mut(0).unwrap().charts.push(chart(0));
            let trend = &mut wb.sheet_mut(0).unwrap().charts[0].series[0].trendlines[0];
            match field {
                0 => trend.forward = n,
                1 => trend.backward = n,
                2 => trend.intercept = Some(n),
                _ => panic!("unexpected fixture selector"),
            }
            assert_eq!(validate_workbook_objects(&wb), Err(RecoveryModelError::Value));
        }
    }
    for n in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, -1.0] {
        let mut wb = Workbook::new();
        let sheet = wb.sheet_mut(0).unwrap();
        sheet.charts.push(chart(0));
        sheet.shapes.push(shape(1));
        wb.sheet_mut(0).unwrap().charts[0].series[0].trendlines[0].width = Some(n);
        assert_eq!(validate_workbook_objects(&wb), Err(RecoveryModelError::Geometry));
        wb.sheet_mut(0).unwrap().charts[0].series[0].trendlines[0].width = Some(f32::MAX);
        wb.sheet_mut(0).unwrap().shapes[0].format.text_outline = Some((Color::Auto, n));
        assert_eq!(validate_workbook_objects(&wb), Err(RecoveryModelError::Geometry));
        wb.sheet_mut(0).unwrap().shapes[0].format.text_outline = None;
        wb.sheet_mut(0).unwrap().shapes[0].format.font_size = Some(n);
        assert_eq!(validate_workbook_objects(&wb), if n.is_finite() { Ok(()) } else { Err(RecoveryModelError::Style) });
    }
    for n in [-f32::MAX, -0.0, 0.0, f32::MAX] {
        let mut wb = Workbook::new();
        wb.sheet_mut(0).unwrap().shapes.push(shape(0));
        wb.sheet_mut(0).unwrap().shapes[0].format.font_size = Some(n);
        assert_eq!(validate_workbook_objects(&wb), Ok(()));
        assert_eq!(wb.sheet(0).unwrap().shapes[0].format.font_size.unwrap().to_bits(), n.to_bits());
    }
}

#[test]
fn recovery_objects_every_object_color_and_sparkline_cell_is_checked() {
    for color in [Color::Rgb(0x1000000), Color::Theme(12, 0), Color::Theme(0, -1001)] {
        for field in 0..7 {
            let mut wb = Workbook::new();
            let sheet = wb.sheet_mut(0).unwrap();
            sheet.charts.push(chart(0));
            sheet.shapes.push(shape(1));
            sheet.sparklines.push(Sparkline {
                cell: CellRef::new(0, 0),
                source: "[outside]broken(".into(),
                kind: SparklineKind::Line,
                color: Color::Auto,
                markers: true,
            });
            match field {
                0 => sheet.charts[0].series[0].color = Some(color),
                1 => sheet.charts[0].series[0].trendlines[0].color = Some(color),
                2 => sheet.shapes[0].fill = color,
                3 => sheet.shapes[0].line = color,
                4 => sheet.shapes[0].format.text_color = Some(color),
                5 => sheet.shapes[0].format.text_outline = Some((color, 0.0)),
                6 => sheet.sparklines[0].color = color,
                _ => panic!("unexpected fixture selector"),
            }
            assert_eq!(validate_workbook_objects(&wb), Err(RecoveryModelError::Style));
        }
    }
    let mut wb = Workbook::new();
    for kind in [SparklineKind::Line, SparklineKind::Column, SparklineKind::WinLoss] {
        wb.sheet_mut(0).unwrap().sparklines.push(Sparkline {
            cell: CellRef::new(0, 0),
            source: "#REF!".into(),
            kind,
            color: Color::Auto,
            markers: false,
        });
    }
    let before = wb.clone();
    assert_eq!(validate_workbook_objects(&wb), Ok(()));
    assert_eq!(wb, before);
    wb.sheet_mut(0).unwrap().sparklines[2].cell.row = MAX_ROWS;
    assert_eq!(validate_workbook_objects(&wb), Err(RecoveryModelError::Coordinate));
}

#[test]
fn recovery_objects_controls_keep_dormant_fields_actions_and_unaligned_values() {
    let mut wb = Workbook::new();
    let button = FormControl {
        value: u32::MAX,
        min: u32::MAX,
        max: 0,
        step: 0,
        page: u32::MAX,
        link: Some("[external]missing!broken(".into()),
        name: "same".into(),
        ..FormControl::default()
    };
    wb.sheet_mut(0).unwrap().controls.push(button);
    let mut params = serde_json::Value::Null;
    for _ in 0..160 {
        params = serde_json::Value::Array(vec![params]);
    }
    wb.sheet_mut(0).unwrap().controls[0].action = Some(ControlAction::Command { id: "missing.command".into(), params });
    let action = wb.sheet(0).unwrap().controls[0].action.as_ref().unwrap() as *const _;
    assert_eq!(validate_workbook_objects(&wb), Ok(()));
    assert_eq!(wb.sheet(0).unwrap().controls[0].action.as_ref().unwrap() as *const _, action);
    wb.sheet_mut(0).unwrap().hyperlinks.insert(CellRef::new(MAX_ROWS, 0), Hyperlink { target: "".into(), tooltip: None, external: false });
    assert_eq!(validate_workbook_objects(&wb), Err(RecoveryModelError::Coordinate));
    let button = &wb.sheet(0).unwrap().controls[0];
    assert_eq!(button.action.as_ref().unwrap() as *const _, action);
    assert_eq!((button.value, button.min, button.max, button.step, button.page), (u32::MAX, u32::MAX, 0, 0, u32::MAX));
    wb.sheet_mut(0).unwrap().hyperlinks.clear();
    for input in [MacroInput::Selection, MacroInput::Fixed("[outside]bad(".into())] {
        for output in [None, Some("#REF!".into())] {
            wb.sheet_mut(0).unwrap().controls[0].action = Some(ControlAction::Macro { name: "missing macro".into(), input: input.clone(), output });
            assert_eq!(validate_workbook_objects(&wb), Ok(()));
        }
    }
    for kind in [ControlKind::Slider, ControlKind::Spin] {
        let control = FormControl {
            id: 1,
            kind,
            min: 2,
            max: 11,
            value: 3,
            step: 4,
            page: if kind == ControlKind::Slider { 4 } else { u32::MAX },
            name: "same".into(),
            ..FormControl::default()
        };
        assert_eq!(control.check_numbers(), Ok(()));
        wb.sheet_mut(0).unwrap().controls.push(control);
        assert_eq!(validate_workbook_objects(&wb), Ok(()));
        wb.sheet_mut(0).unwrap().controls.pop();
    }
}

#[test]
fn recovery_objects_controls_enforce_only_active_numeric_contract() {
    for kind in [ControlKind::Slider, ControlKind::Spin] {
        for (min, max, value, step, page) in [
            (0, 30001, 0, 1, 10),
            (30001, 30002, 30001, 1, 10),
            (5, 5, 5, 1, 10),
            (6, 5, 5, 1, 10),
            (0, 5, 0, 0, 10),
            (0, 5, 0, 6, 10),
            (2, 5, 1, 1, 10),
            (2, 5, 6, 1, 10),
            (0, 5, 0, 2, 1),
            (0, 5, 0, 1, 30001),
        ] {
            let mut wb = Workbook::new();
            wb.sheet_mut(0).unwrap().controls.push(FormControl { kind, min, max, value, step, page, ..FormControl::default() });
            let dormant_page = kind == ControlKind::Spin && (page == 1 || page == 30001);
            assert_eq!(validate_workbook_objects(&wb), if dormant_page { Ok(()) } else { Err(RecoveryModelError::Control) });
        }
    }
}

#[test]
fn recovery_objects_notes_pictures_slicers_and_hyperlinks_preserve_sharing_on_success_and_late_failure() {
    let mut wb = Workbook::new();
    let sheet = wb.sheet_mut(0).unwrap();
    floating(sheet, 1, 0);
    floating(sheet, 2, 1);
    floating(sheet, 3, 2);
    sheet.cell_pictures.insert(CellRef::new(1, 2), Arc::new(CellPicture { data: vec![0, 255, 3], mime: "unknown/inert".into(), alt: "raw".into() }));
    Arc::make_mut(&mut sheet.comments).insert(CellRef::new(5, 7), note());
    sheet
        .hyperlinks
        .insert(CellRef::new(9, 8), Hyperlink { target: "unparseable external path".into(), tooltip: Some("raw".into()), external: true });
    let comments = sheet.comments.clone();
    let image = sheet.images[0].data.clone();
    let picture = sheet.cell_pictures.values().next().unwrap().clone();
    let before = wb.clone();
    for fail in [false, true] {
        if fail {
            wb.sheet_mut(0).unwrap().hyperlinks.insert(CellRef::new(MAX_ROWS, 0), Hyperlink { target: "".into(), tooltip: None, external: false });
        }
        // Capture after fixture mutation: sheet_mut may perform copy-on-write beforehand.
        let math = wb.sheet(0).unwrap().shapes[0].math.get("unparseable(").unwrap().unwrap_err() as *const _;
        let sheet_ptr = Arc::as_ptr(&wb.sheets[0]);
        let counts = (Arc::strong_count(&comments), Arc::strong_count(&image), Arc::strong_count(&picture));
        assert_eq!(validate_workbook_objects(&wb), if fail { Err(RecoveryModelError::Coordinate) } else { Ok(()) });
        let sheet = wb.sheet(0).unwrap();
        assert_eq!(Arc::as_ptr(&wb.sheets[0]), sheet_ptr);
        assert_eq!((Arc::strong_count(&comments), Arc::strong_count(&image), Arc::strong_count(&picture)), counts);
        assert!(Arc::ptr_eq(&sheet.comments, &comments));
        assert!(Arc::ptr_eq(&sheet.images[0].data, &image));
        assert!(Arc::ptr_eq(sheet.cell_pictures.values().next().unwrap(), &picture));
        assert_eq!(sheet.shapes[0].math.get("unparseable(").unwrap().unwrap_err() as *const _, math);
        assert_eq!(sheet.comments.as_ref(), comments.as_ref());
        assert_eq!(sheet.comments[&CellRef::new(5, 7)].note_box.unwrap().size_pt.unwrap().0.to_bits(), (-0.0f64).to_bits());
        assert_eq!(sheet.images[0], before.sheet(0).unwrap().images[0]);
        assert_eq!(sheet.slicers[0].selected, Some(vec![]));
    }
    wb.sheet_mut(0).unwrap().hyperlinks.remove(&CellRef::new(MAX_ROWS, 0));
    wb.sheet_mut(0).unwrap().slicers[0].selected = None;
    assert_eq!(validate_workbook_objects(&wb), Ok(()));
}

#[test]
fn recovery_objects_every_optional_note_dimension_and_cell_map_key_is_checked() {
    for n in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, -1.0] {
        for dim in 0..2 {
            let mut wb = Workbook::new();
            let mut comment = note();
            comment.note_box.as_mut().unwrap().size_pt = Some(if dim == 0 { (n, 1.0) } else { (1.0, n) });
            Arc::make_mut(&mut wb.sheet_mut(0).unwrap().comments).insert(CellRef::new(5, 7), comment);
            assert_eq!(validate_workbook_objects(&wb), Err(RecoveryModelError::Geometry));
            let size = wb.sheet(0).unwrap().comments[&CellRef::new(5, 7)].note_box.unwrap().size_pt.unwrap();
            assert_eq!(if dim == 0 { size.0 } else { size.1 }.to_bits(), n.to_bits());
        }
    }
    for field in 0..3 {
        for at in [CellRef::new(MAX_ROWS, 0), CellRef::new(0, MAX_COLS)] {
            let mut wb = Workbook::new();
            let sheet = wb.sheet_mut(0).unwrap();
            match field {
                0 => {
                    Arc::make_mut(&mut sheet.comments).insert(at, note());
                }
                1 => {
                    sheet.hyperlinks.insert(at, Hyperlink { target: "".into(), tooltip: None, external: false });
                }
                2 => {
                    sheet.cell_pictures.insert(at, Arc::new(CellPicture { data: vec![], mime: "".into(), alt: "".into() }));
                }
                _ => panic!("unexpected fixture selector"),
            }
            assert_eq!(validate_workbook_objects(&wb), Err(RecoveryModelError::Coordinate));
        }
    }
    let mut wb = Workbook::new();
    let mut comment = note();
    comment.note_box.as_mut().unwrap().size_pt = None;
    Arc::make_mut(&mut wb.sheet_mut(0).unwrap().comments).insert(CellRef::new(5, 7), comment);
    assert_eq!(validate_workbook_objects(&wb), Ok(()));
    Arc::make_mut(&mut wb.sheet_mut(0).unwrap().comments).get_mut(&CellRef::new(5, 7)).unwrap().note_box = None;
    assert_eq!(validate_workbook_objects(&wb), Ok(()));
}

#[test]
fn chart_group_recovery_requires_distinct_local_members() {
    let mut wb = Workbook::new();
    let sheet = wb.sheet_mut(0).unwrap();
    sheet.charts = vec![chart(1), chart(2), chart(3)];
    sheet.chart_groups = vec![ChartGroup { name: "Revenue".into(), chart_ids: vec![2, 1] }];
    assert_eq!(validate_workbook_objects(&wb), Ok(()));
    for invalid in [vec![1, 1], vec![2, 99], vec![2], vec![1, 2, 3, 1]] {
        wb.sheet_mut(0).unwrap().chart_groups[0].chart_ids = invalid;
        assert_eq!(validate_workbook_objects(&wb), Err(RecoveryModelError::UnsupportedRepresentation(RecoveryRepresentation::AmbiguousIdentity)),);
    }
    let sheet = wb.sheet_mut(0).unwrap();
    sheet.chart_groups[0].chart_ids = vec![1, 2];
    sheet.chart_groups.push(ChartGroup { name: "Overlap".into(), chart_ids: vec![1, 3] });
    assert!(validate_workbook_objects(&wb).is_err());
}
