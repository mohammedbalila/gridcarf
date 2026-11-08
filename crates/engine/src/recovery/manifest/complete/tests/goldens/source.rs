use super::*;
use gridcraft_core::{Array, CellError, DateSystem, MAX_COLS, MAX_ROWS};
use gridcraft_model::*;
fn at(row: u32, col: u32) -> CellRef {
    CellRef::new(row, col)
}
fn rect(r0: u32, c0: u32, r1: u32, c1: u32) -> RangeRef {
    RangeRef { start: at(r0, c0), end: at(r1, c1) }
}
pub(super) fn minimal() -> (Workbook, Metadata) {
    let mut metadata = super::super::metadata();
    metadata.title.clear();
    metadata.original_path_hint = None;
    metadata.file_kind = None;
    metadata.dirty_at_capture = false;
    metadata.sheet_selections.clear();
    metadata.id = "00000000000000000000000000000000".parse().unwrap();
    (Workbook::new(), metadata)
}

pub(super) fn ordinary() -> Workbook {
    let mut wb = Workbook::new();
    wb.date_system = DateSystem::D1904;
    wb.show_objects = ShowObjects::Placeholders;
    wb.remove_personal_info = true;
    wb.calc = CalcSettings {
        mode: CalcMode::Manual,
        iterative: true,
        max_iterations: u32::MAX,
        max_change: -0.0,
        precision_as_displayed: true,
        r1c1: true,
    };
    wb.view = WorkbookView { show_sheet_tabs: false, show_horizontal_scroll: false, show_vertical_scroll: false, tab_ratio: u32::MAX };
    wb.props =
        DocProps { title: "t".into(), subject: "s".into(), author: "a".into(), company: "c".into(), keywords: "k".into(), description: "d".into() };
    wb.protected_structure = true;
    wb.structure_password =
        Some(PasswordHash::Iterated { algorithm: "future".into(), hash: "raw".into(), salt: "opaque".into(), spin_count: u32::MAX });
    wb.custom_lists = vec![vec!["same".into(), "same".into()], vec![]];
    let border = BorderLine { style: BorderStyle::SlantDashDot, color: Color::Theme(1, 0) };
    let mut style = Style {
        font: Font {
            name: "synthetic".into(),
            size: -0.0,
            bold: true,
            italic: true,
            underline: Underline::DoubleAccounting,
            strike: true,
            color: Color::Theme(11, -1000),
            vert: VertAlign::Superscript,
            charset: Some(255),
            family: Some(255),
            minor: false,
        },
        fill: Fill { pattern: PatternType::LightGrid, fg: Color::Rgb(0x123456), bg: Color::Theme(0, 1000) },
        borders: Borders { left: border, right: border, top: border, bottom: border, diag_down: border, diag_up: border },
        align: Alignment { h: HAlign::Distributed, v: VAlign::Distributed, wrap: true, shrink: true, indent: 255, rotation: 255 },
        num_fmt: NumFmt(Arc::from("[unparseable format")),
        protection: Default::default(),
        dxf_stated: Default::default(),
    };
    style.protection.locked = false;
    style.protection.hidden = true;
    style.dxf_stated.bold = true;
    style.dxf_stated.italic = true;
    style.dxf_stated.strike = true;
    style.dxf_stated.underline = true;
    style.dxf_stated.size = true;
    style.dxf_stated.name = true;
    style.dxf_stated.scheme = Some("future".into());
    style.dxf_stated.protection = true;
    let id = StyleId(1);
    wb.styles = serde_json::from_value(serde_json::json!({"styles":[Style::default(),style.clone(),style.clone()]})).unwrap();
    wb.cell_styles = vec![("Normal".into(), style.clone())];
    wb.theme = Theme {
        name: "retained".into(),
        colors: [0xabcdef; 12],
        major_font: "major".into(),
        minor_font: "minor".into(),
        major_font_ea: "ea".into(),
        minor_font_ea: "ea2".into(),
        major_font_cs: "cs".into(),
        minor_font_cs: "cs2".into(),
        major_font_scripts: vec![("Arab".into(), "synthetic".into())],
        minor_font_scripts: vec![("".into(), "raw".into())],
    };
    wb.names = vec![DefinedName {
        name: "_gc_macro_0_0".into(),
        scope: Some(0),
        formula: "[external]broken(".into(),
        comment: "retained".into(),
        hidden: true,
    }];
    wb.phonetics = vec![PhoneticString {
        text: "A😀B".into(),
        runs: vec![PhoneticRun { start: 2, end: 3, text: String::new() }, PhoneticRun { start: 1, end: 4, text: "overlap".into() }],
        props: Some(PhoneticProps { font_id: u32::MAX, kind: Some("future".into()), alignment: Some("raw".into()) }),
    }];
    wb.vba_project = Some(Arc::new(vec![0, 255]));
    wb.csv_delimiter = Some(0);
    wb.macros = macros();
    wb.macros_unread = Arc::new(vec![
        serde_json::json!({"v":u64::MAX,"name":"Future","metadata":{"cells":null},"data":"not-base64"}),
        serde_json::json!({"v":2,"name":"Future","steps":[false]}),
    ]);
    let sheet = wb.sheet_mut(0).unwrap();
    sheet.file_id = u32::MAX;
    sheet.default_col_width = 12.5;
    sheet.default_row_height = 12.5;
    sheet.cols.insert(MAX_COLS - 1, LineInfo { size: Some(-0.0), hidden: true, outline: 255, collapsed: true, style: Some(id), custom: true });
    sheet.rows.insert(MAX_ROWS - 1, LineInfo { size: Some(12.5), hidden: true, outline: 255, collapsed: true, style: Some(id), custom: true });
    sheet.merges = vec![rect(8, 8, 9, 9)];
    sheet.freeze = Some((MAX_ROWS, MAX_COLS));
    sheet.tab_color = Some(Color::Theme(11, 1000));
    sheet.visibility = Visibility::VeryHidden;
    sheet.show_gridlines = false;
    sheet.show_headings = false;
    sheet.show_formulas = true;
    sheet.show_zeros = false;
    sheet.show_outline_symbols = false;
    sheet.summary_below = false;
    sheet.summary_right = false;
    sheet.right_to_left = true;
    sheet.zoom = u16::MAX;
    sheet.view = SheetViewMode::PageLayout;
    sheet.view_active = at(MAX_ROWS - 1, MAX_COLS - 1);
    sheet.view_top_left = at(1, 1);
    let formula =
        Formula { text: "=retained broken(".into(), expr: None, array: Some(rect(0, 0, 0, 1)), dynamic: true, eval_mode: FormulaEvalMode::Legacy };
    sheet.cells.set(
        at(0, 0),
        Cell { value: Value::Array(Arc::new(Array::scalar(Value::Number(-0.0)))), formula: Some(Arc::new(formula)), style: id, py: None },
    );
    sheet.spill_ranges.insert(at(0, 0), rect(0, 0, 0, 1));
    sheet.spill.insert(at(0, 1), Value::Empty);
    sheet.cells.set(at(0, 1), Cell::styled(Value::from("manual shadow"), id));
    sheet.cells.set(
        at(1, 0),
        Cell {
            value: Value::Bool(true),
            formula: None,
            style: id,
            py: Some(Arc::new(PyCode {
                code: "x\r\n\"😀\"".into(),
                object: true,
                output: Some(PyOutput { value: Value::Number(-0.0), type_name: "future type".into(), preview: "raw".into(), convertible: false }),
                stale: true,
            })),
        },
    );
    sheet.cells.set(at(1, 2), Cell { value: Value::from("manual"), py: Some(Arc::new(PyCode::default())), ..Default::default() });
    sheet.cells.set(
        at(1, 3),
        Cell {
            value: Value::Error(CellError::Blocked),
            py: Some(Arc::new(PyCode {
                code: "pass".into(),
                object: false,
                output: Some(PyOutput { value: Value::Bool(true), type_name: "bool".into(), preview: "true".into(), convertible: true }),
                stale: false,
            })),
            ..Default::default()
        },
    );
    sheet.cell_pictures.insert(at(1, 1), Arc::new(CellPicture { data: super::super::png(), mime: "image/png".into(), alt: "raw".into() }));
    sheet.tables = vec![Table {
        id: 0,
        name: " table ".into(),
        range: rect(2, 0, 4, 1),
        header_row: true,
        totals_row: true,
        columns: vec![TableColumn {
            name: "".into(),
            totals: TotalsFn::Custom,
            totals_label: Some("raw".into()),
            formula: Some("bad(".into()),
            unique_name: Some("u".into()),
            xml: Some(XmlColumnPr { map_id: u32::MAX, xpath: "raw".into(), data_type: "future".into(), denormalized: true }),
        }],
        style: "future".into(),
        banded_rows: false,
        banded_cols: true,
        first_col: true,
        last_col: true,
        filter_button: false,
        filter: vec![(0, FilterCriterion::Top10 { bottom: true, percent: true, count: u32::MAX })],
        table_type: Some("future".into()),
        connection_id: Some(u32::MAX),
    }];
    sheet.tables[0].columns.push(TableColumn {
        name: "Plain".into(),
        totals: TotalsFn::None,
        totals_label: None,
        formula: None,
        unique_name: None,
        xml: None,
    });
    sheet.what_if_tables = vec![WhatIfTable {
        anchor: at(3, 0),
        range: rect(3, 0, 3, 1),
        formula_text: "bad(".into(),
        input1: Some(at(9, 9)),
        input2: Some(at(8, 8)),
        two_variable: Some(true),
        row_oriented: Some(false),
        input1_deleted: Some(false),
        input2_deleted: Some(true),
        calculate: Some(false),
    }];
    sheet.what_if_tables.push(WhatIfTable {
        anchor: at(20, 0),
        range: rect(20, 0, 20, 1),
        formula_text: String::new(),
        input1: None,
        input2: None,
        two_variable: None,
        row_oriented: None,
        input1_deleted: None,
        input2_deleted: None,
        calculate: None,
    });
    sheet.autofilter = Some(AutoFilter {
        range: RangeRef::all(),
        criteria: vec![(MAX_COLS - 1, FilterCriterion::Icon { set: "future".into(), id: Some(u32::MAX) })],
    });
    sheet.cond_formats = vec![CondFormat {
        ranges: vec![RangeRef::all()],
        rule: CfRule::Expression { formula: "bad(".into(), style: Box::new(style.clone()) },
        priority: 0,
        stop_if_true: true,
    }];
    let mut rules = vec![
        CfRule::CellIs { op: CfOperator::Between, a: "1".into(), b: Some("2".into()), style: Box::new(style.clone()) },
        CfRule::ContainsText { text: "a".into(), style: Box::new(style.clone()) },
        CfRule::NotContainsText { text: "a".into(), style: Box::new(style.clone()) },
        CfRule::BeginsWith { text: "a".into(), style: Box::new(style.clone()) },
        CfRule::EndsWith { text: "a".into(), style: Box::new(style.clone()) },
        CfRule::Blanks { style: Box::new(style.clone()) },
        CfRule::NoBlanks { style: Box::new(style.clone()) },
        CfRule::Errors { style: Box::new(style.clone()) },
        CfRule::NoErrors { style: Box::new(style.clone()) },
        CfRule::Duplicate { style: Box::new(style.clone()) },
        CfRule::Unique { style: Box::new(style.clone()) },
        CfRule::Top10 { bottom: true, percent: true, rank: 3, style: Box::new(style.clone()) },
        CfRule::AboveAverage { below: true, equal: true, std_dev: 2, style: Box::new(style.clone()) },
        CfRule::TimePeriod { period: "today".into(), style: Box::new(style.clone()) },
        CfRule::DataBar { min: CfValueKind::Number(-0.0), max: CfValueKind::Max, color: Color::Rgb(3), gradient: false, show_value: false },
        CfRule::IconSet {
            set: "3Arrows".into(),
            thresholds: vec![CfValueKind::Percent(0.0), CfValueKind::Percent(50.0), CfValueKind::Percent(100.0)],
            reverse: true,
            show_value: false,
        },
    ];
    for stops in [
        vec![(CfValueKind::Min, Color::Auto), (CfValueKind::Max, Color::Rgb(1))],
        vec![(CfValueKind::Number(-0.0), Color::Auto), (CfValueKind::Percent(12.5), Color::Auto)],
        vec![(CfValueKind::Percentile(25.0), Color::Auto), (CfValueKind::Formula("bad(".into()), Color::Auto)],
    ] {
        rules.push(CfRule::ColorScale { stops });
    }
    sheet.cond_formats.extend(rules.into_iter().enumerate().map(|(i, rule)| CondFormat {
        ranges: vec![RangeRef::all()],
        rule,
        priority: i as u32 + 1,
        stop_if_true: false,
    }));
    sheet.autofilter.as_mut().unwrap().criteria.extend([
        (0, FilterCriterion::Values { values: vec!["a".into(), "a".into()], blanks: true }),
        (1, FilterCriterion::Custom { a: (">".into(), "1".into()), b: Some(("<=".into(), "2".into())), and: true }),
        (2, FilterCriterion::AboveAverage(false)),
        (3, FilterCriterion::FillColor(Color::Theme(2, 0))),
        (4, FilterCriterion::FontColor(Color::Auto)),
        (5, FilterCriterion::Icon { set: "3Arrows".into(), id: None }),
    ]);
    sheet.validations = vec![Validation {
        ranges: vec![RangeRef::all()],
        kind: ValidationKind::Custom,
        op: CfOperator::NotBetween,
        f1: "broken(".into(),
        f2: Some("external".into()),
        allow_blank: false,
        in_cell_dropdown: false,
        input_title: "i".into(),
        input_message: "m".into(),
        show_input: false,
        error_title: "e".into(),
        error_message: "m".into(),
        error_style: ErrorStyle::Information,
        show_error: false,
    }];
    sheet.pivots = vec![PivotTable {
        id: 0,
        name: "pivot".into(),
        source: "[external]broken(".into(),
        anchor: at(4, 0),
        rows: vec![PivotField {
            source_col: "missing".into(),
            sort: PivotSort::None,
            date_group: PivotDateGroup::Days,
            collapsed_items: vec!["raw".into()],
        }],
        columns: vec![PivotField {
            source_col: "missing column".into(),
            sort: PivotSort::Desc,
            date_group: PivotDateGroup::Months,
            collapsed_items: vec!["raw".into()],
        }],
        values: vec![PivotValue {
            source_col: "missing".into(),
            func: PivotFunc::VarP,
            name: "v".into(),
            show_as: PivotShowAs::Rank,
            number_format: Some("raw".into()),
        }],
        filters: vec![PivotFilter { source_col: "missing".into(), selected: Some(vec![]) }],
        layout: PivotLayout::Tabular,
        enable_show_details: false,
        grand_totals_rows: false,
        grand_totals_cols: false,
        subtotals: false,
        subtotals_top: false,
        style: "future".into(),
        show_headers: false,
        last_range: Some(rect(10, 10, 11, 11)),
        calculated_fields: vec![PivotCalcField { name: " raw ".into(), formula: "broken(".into() }],
        values_on_rows: true,
        values_position: Some(1),
    }];
    sheet.pivots.push(PivotTable { id: 1, name: "DefaultPivot".into(), source: "Sheet1!A1:B2".into(), anchor: at(20, 20), ..Default::default() });
    let anchor = Anchor { cell: at(MAX_ROWS - 1, MAX_COLS - 1), dx: -1.25, dy: 2.5, width: 12.5, height: -0.0, mode: AnchorMode::Absolute };
    sheet.charts = vec![Chart {
        id: 0,
        kind: ChartKind::Combo,
        anchor,
        title: Some("raw".into()),
        series: vec![Series {
            name: Some("bad(".into()),
            categories: Some("external".into()),
            values: "broken(".into(),
            bubble_sizes: Some("raw".into()),
            color: Some(Color::Auto),
            secondary: true,
            kind: Some(ChartKind::Bubble),
            smooth: true,
            quartiles: QuartileMethod::Inclusive,
            trendlines: vec![Trendline {
                kind: TrendlineKind::Polynomial(255),
                name: Some("raw".into()),
                forward: 1.25,
                backward: 25.5,
                intercept: Some(-0.0),
                display_equation: true,
                display_r_squared: true,
                color: Some(Color::Rgb(0)),
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
        source: Some("raw".into()),
        by_rows: true,
        pivot: Some("missing".into()),
    }];
    sheet.charts[0].series[0].trendlines.push(Trendline::default());
    sheet.charts[0].series.push(Series::default());
    let mut chart = sheet.charts[0].clone();
    chart.id = 8;
    chart.kind = ChartKind::Line;
    chart.anchor = Anchor::default();
    chart.title = None;
    chart.series = vec![Series::default()];
    chart.legend = LegendPos::Right;
    chart.data_labels = false;
    chart.data_table = None;
    chart.gridlines = true;
    chart.style = 1;
    chart.hole_size = 50;
    chart.first_slice_angle = 0;
    chart.x_title = None;
    chart.y_title = None;
    chart.source = None;
    chart.by_rows = false;
    chart.pivot = None;
    sheet.charts.push(chart);
    sheet.images = vec![Image { id: 1, anchor, data: Arc::new(super::super::png()), mime: "image/png".into(), alt: "raw".into() }];
    sheet.shapes = vec![Shape {
        id: 2,
        kind: ShapeKind::Equation,
        anchor,
        fill: Color::Auto,
        line: Color::Rgb(0),
        text: "bad(".into(),
        math: Default::default(),
        format: ShapeFormat {
            no_line: true,
            font_size: Some(-0.0),
            bold: true,
            italic: true,
            font: Some("raw".into()),
            text_color: Some(Color::Auto),
            text_outline: Some((Color::Auto, -0.0)),
            align: Some(ShapeTextAlign::Right),
            anchor: Some(ShapeTextAnchor::Bottom),
            rtl: true,
        },
    }];
    sheet.shapes.push(Shape {
        id: 9,
        kind: ShapeKind::Rectangle,
        anchor: Anchor::default(),
        fill: Color::Auto,
        line: Color::Auto,
        text: String::new(),
        format: ShapeFormat::default(),
        math: Default::default(),
    });
    sheet.images.push(Image { id: 10, anchor: Anchor::default(), data: Arc::new(super::super::png()), mime: "image/png".into(), alt: String::new() });
    sheet.sparklines = vec![Sparkline { cell: at(5, 0), source: "external".into(), kind: SparklineKind::WinLoss, color: Color::Auto, markers: true }];
    sheet.slicers =
        vec![Slicer { id: 3, table: "missing".into(), column: "missing".into(), caption: "raw".into(), anchor, selected: Some(vec![]), columns: 0 }];
    sheet.controls = vec![FormControl {
        id: 4,
        kind: ControlKind::Slider,
        anchor,
        name: "raw".into(),
        text: "raw".into(),
        link: Some("bad(".into()),
        value: 2,
        min: 1,
        max: 9,
        step: 3,
        page: 3,
        horizontal: false,
        action: Some(ControlAction::Macro { name: "missing".into(), input: MacroInput::Fixed("raw".into()), output: Some("bad(".into()) }),
        print: false,
        locked: false,
    }];
    sheet.comments_mut().insert(
        at(1, 1),
        Comment {
            author: "a".into(),
            text: "t".into(),
            replies: vec![("a".into(), "b".into())],
            threaded: true,
            resolved: true,
            visible: true,
            dates: vec!["not a date".into()],
            note_box: Some(NoteBox {
                from: NoteCorner { row: -1, col: 2, row_px: u32::MAX, col_px: 0 },
                to: NoteCorner { row: 3, col: -2, row_px: 0, col_px: u32::MAX },
                size_pt: Some((-0.0, 25.5)),
            }),
        },
    );
    sheet.comments_mut().insert(
        at(0, 2),
        Comment {
            author: String::new(),
            text: String::new(),
            replies: vec![],
            threaded: false,
            resolved: false,
            visible: false,
            dates: vec![],
            note_box: None,
        },
    );
    sheet.hyperlinks.insert(at(0, 2), Hyperlink { target: "Sheet1!A1".into(), tooltip: None, external: false });
    sheet.hyperlinks.insert(at(1, 1), Hyperlink { target: "unresolved".into(), tooltip: Some("raw".into()), external: true });
    sheet.protection = Some(SheetProtection {
        password: Some(PasswordHash::Legacy(u16::MAX)),
        legacy_password_hash: Some(u64::MAX),
        select_locked: true,
        select_unlocked: true,
        format_cells: true,
        format_columns: true,
        format_rows: true,
        insert_columns: true,
        insert_rows: true,
        delete_columns: true,
        delete_rows: true,
        sort: true,
        autofilter: true,
    });
    sheet.edit_ranges = vec![EditRange {
        title: "".into(),
        ranges: vec![RangeRef::all(), RangeRef::all()],
        password: Some(PasswordHash::Legacy(0)),
        security_descriptor: Some("opaque ACL".into()),
    }];
    sheet.print = PrintSettings {
        orientation: Orientation::Landscape,
        paper: "unknown".into(),
        margins: [-0.0, 12.5, 0.0, 1.0, 2.0, 3.0],
        print_area: Some(RangeRef::all()),
        title_rows: Some((0, MAX_ROWS - 1)),
        title_cols: Some((0, MAX_COLS - 1)),
        scale: 0,
        fit_to: Some((0, u16::MAX)),
        gridlines: true,
        headings: true,
        center_h: true,
        center_v: true,
        header: "odd".into(),
        footer: "odd f".into(),
        row_breaks: vec![MAX_ROWS - 1, 1, 1],
        col_breaks: vec![MAX_COLS - 1, 1, 1],
        different_first: true,
        different_odd_even: true,
        first_header: "first".into(),
        first_footer: "first f".into(),
        even_header: "even".into(),
        even_footer: "even f".into(),
        hf_scale_with_doc: false,
        hf_align_with_margins: false,
        page_order: PageOrder::OverThenDown,
    };
    sheet.cells.set(at(64, 0), Cell { style: StyleId(2), ..Default::default() });
    sheet.cells.set(at(MAX_ROWS - 1, MAX_COLS - 1), Cell::value(Value::Number(-0.0)));
    for (col, error) in CellError::ALL.into_iter().chain([CellError::Circ]).enumerate() {
        sheet.cells.set(at(2, col as u32), Cell::value(Value::Error(error)));
    }
    sheet.cells.set(
        at(6, 0),
        Cell {
            value: Value::Number(42.0),
            formula: Some(Arc::new(Formula {
                text: "RAND()".into(),
                expr: gridcraft_formula::parse("RAND()").ok().map(Arc::new),
                array: None,
                dynamic: false,
                eval_mode: FormulaEvalMode::Array,
            })),
            ..Default::default()
        },
    );
    sheet.controls.push(FormControl {
        id: 5,
        action: Some(ControlAction::Command { id: "x".into(), params: serde_json::json!({"data":"***","metadata":{"cells":null}}) }),
        ..Default::default()
    });
    sheet.controls.push(FormControl {
        id: 6,
        kind: ControlKind::Spin,
        action: Some(ControlAction::Macro { name: "M".into(), input: MacroInput::Selection, output: None }),
        ..Default::default()
    });
    sheet.controls.push(FormControl { id: 7, ..Default::default() });
    wb.phonetics.push(PhoneticString { text: "plain".into(), runs: vec![], props: None });
    wb.phonetics.push(PhoneticString { text: "props".into(), runs: vec![], props: Some(PhoneticProps::default()) });
    wb.names.push(DefinedName { name: "Global".into(), scope: None, formula: "1".into(), comment: String::new(), hidden: false });
    wb.sheets.push(Arc::new(Sheet::new("Hidden")));
    wb.sheet_mut(1).unwrap().visibility = Visibility::Hidden;
    wb.sheets.push(Arc::new(Sheet::new("Active")));
    wb.set_active_sheet(2);
    wb
}

fn macros() -> Macros {
    let command = Step {
        kind: StepKind::Command { id: "x".into(), params: serde_json::json!({"data":"***","metadata":{"cells":null},"steps":[false]}) },
        enabled: false,
        breakpoint: true,
        note: "note".into(),
    };
    let stop = Step::new(StepKind::Stop { message: "kept".into(), rollback: true });
    vec![
        Macro {
            name: "M".into(),
            description: "desc".into(),
            shortcut: Some("Ctrl+M".into()),
            inputs: vec![
                NamedInput {
                    name: "Rates".into(),
                    kind: InputKind::Range,
                    rows: [1, 9],
                    cols: [1, 5],
                    header: true,
                    col_types: vec![ColType::Any, ColType::Number, ColType::Text, ColType::Date, ColType::Bool],
                    non_empty: true,
                    default_ref: Some("Sheet1!A1:E9".into()),
                },
                NamedInput { name: "Single".into(), kind: InputKind::Cell, ..Default::default() },
            ],
            steps: vec![
                command,
                Step::new(StepKind::Repeat {
                    over: "Rates".into(),
                    of: RepeatOf::Columns,
                    max: 3,
                    body: vec![Step::command("x", serde_json::json!(null))],
                }),
                Step::new(StepKind::If {
                    cell: "A1".into(),
                    op: CompareOp::Eq,
                    value: serde_json::json!(-0.0),
                    then: vec![stop.clone()],
                    otherwise: vec![Step::command("x", serde_json::json!({}))],
                }),
                stop,
                Step::new(StepKind::Stop { message: String::new(), rollback: false }),
                Step::new(StepKind::Repeat { over: "A1:B1".into(), of: RepeatOf::Rows, max: 10000, body: vec![] }),
                Step::new(StepKind::If { cell: "A1".into(), op: CompareOp::Empty, value: serde_json::Value::Null, then: vec![], otherwise: vec![] }),
            ],
            output: OutputRule { anchor: Some("Z1".into()), on_conflict: OnConflict::ShiftRight },
            permissions: vec!["x".into()],
            ..Default::default()
        },
        Macro { name: "Default".into(), ..Default::default() },
    ]
    .into()
}
pub(super) fn full_metadata(memory: usize) -> Metadata {
    let mut m = super::super::metadata();
    m.title = "golden 😀\n\"\\".into();
    m.id = "0123456789abcdef0123456789abcdef".parse().unwrap();
    m.captured_revision = u64::MAX;
    m.sequence = 7;
    m.import_issues = vec![CompatibilityIssue {
        code: "raw.code".into(),
        feature: "Feature".into(),
        location: Some("where".into()),
        effect: CompatibilityEffect::Static,
        editability: Editability::PreservedOnly,
        description: "kept".into(),
    }];
    m.import_issues.push(CompatibilityIssue {
        code: "empty".into(),
        feature: "empty".into(),
        location: None,
        effect: CompatibilityEffect::Unknown,
        editability: Editability::Editable,
        description: String::new(),
    });
    m.selection = Selection { active: at(7, 3), anchor: at(9, 9), ranges: vec![rect(1, 1, 2, 2), rect(7, 3, 7, 4)] };
    m.sheet_selections =
        vec![m.selection.clone(), Selection::default(), Selection::at(at(100, 100)), Selection::at(at(200, 200)), Selection::range(RangeRef::all())];
    m.sheet_selections.truncate(memory);
    m
}

const REL: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
pub(super) const CHART: &str = r#"<chartsheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheetViews><sheetView workbookViewId="0"/></sheetViews><pageSetup r:id="rId2"/><drawing r:id="rId1"/></chartsheet>"#;
pub(super) const DRAWING: &str = r#"<xdr:wsDr xmlns:xdr="http://schemas.openxmlformats.org/drawingml/2006/spreadsheetDrawing"/>"#;
fn rel(id: &str, kind: &str, target: &str, external: bool) -> PreservedRel {
    PreservedRel { id: id.into(), rel_type: format!("{REL}/{kind}"), target: target.into(), external }
}
fn part(name: &str, kind: &str, data: &[u8], rels: Vec<PreservedRel>) -> PreservedPart {
    PreservedPart { name: name.into(), content_type: kind.into(), data: Arc::from(data), rels }
}
pub(super) fn owners() -> Workbook {
    let mut w = Workbook::new();
    let mut parts = vec![
        part("docProps/custom.xml", "application/vnd.openxmlformats-officedocument.custom-properties+xml", b"<root/>", vec![]),
        part("customXml/book.xml", "application/xml", b"<root/>", vec![rel("ext", "hyperlink", "book.xlsx#A1", true)]),
        part(
            "xl/chartsheets/keep.xml",
            "application/vnd.openxmlformats-officedocument.spreadsheetml.chartsheet+xml",
            CHART.as_bytes(),
            vec![rel("rId2", "printerSettings", "xl/printerSettings/keep.bin", false), rel("rId1", "drawing", "xl/drawings/keep.xml", false)],
        ),
        part("xl/drawings/keep.xml", "application/vnd.openxmlformats-officedocument.drawing+xml", DRAWING.as_bytes(), vec![]),
        part("customXml/zero.bin", "application/octet-stream", b"", vec![]),
        part("xl/printerSettings/keep.bin", "application/vnd.openxmlformats-officedocument.spreadsheetml.printerSettings", &[7, 8], vec![]),
    ];
    parts[1].data = parts[0].data.clone();
    w.preserved = Some(Arc::new(Preserved {
        package_rels: vec![rel("pkg", "custom-properties", "docProps/custom.xml", false)],
        rels: vec![
            rel("chart", "chartsheet", "xl/chartsheets/keep.xml", false),
            rel("book", "customXml", "customXml/book.xml", false),
            rel("zero", "customXml", "customXml/zero.bin", false),
        ],
        parts,
        elements: vec![PreservedXml {
            slot: "fileVersion".into(),
            xml: r#"<fileVersion xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" appName="synthetic" />"#.into(),
        }],
        vml: None,
        sheets: vec![PreservedSheet { name: "Chart".into(), visibility: Visibility::Hidden, rel_id: "chart".into(), before: 1, sheet_id: 2 }],
    }));
    let s = w.sheet_mut(0).unwrap();
    s.file_id = 1;
    s.preserved = Some(Arc::new(Preserved {
        rels: vec![rel("q", "queryTable", "xl/queryTables/keep.xml", false)],
        parts: vec![
            part("xl/queryTables/keep.xml", "application/xml", b"<queryTable/>", vec![]),
            part("xl/media/kept.png", "image/png", &super::super::png(), vec![]),
        ],
        vml: Some(part(
            "xl/drawings/keep.vml",
            "application/vnd.openxmlformats-officedocument.vmlDrawing",
            b"<xml/>",
            vec![rel("image", "image", "xl/media/kept.png", false)],
        )),
        ..Default::default()
    }));
    w
}
pub(super) fn pictures(witness: bool) -> Workbook {
    let mut w = Workbook::new();
    let a = super::super::picture();
    let b = super::super::picture();
    let s = w.sheet_mut(0).unwrap();
    for (i, p) in if witness { vec![a.clone(), a.clone()] } else { vec![a.clone(), b, a.clone()] }.into_iter().enumerate() {
        s.cell_pictures.insert(at(0, i as u32), p);
    }
    if witness {
        let name = "xl/media/cellImage2.png/child.bin";
        w.preserved = Some(Arc::new(Preserved {
            rels: vec![rel("kept", "customXml", name, false)],
            parts: vec![part(name, "application/octet-stream", b"retained-witness", vec![])],
            ..Default::default()
        }));
    } else {
        let mut second = Sheet::new("Other");
        second.cell_pictures.insert(at(0, 0), a);
        w.sheets.push(Arc::new(second));
    }
    w
}
pub(super) fn empty_owners() -> Workbook {
    let mut w = Workbook::new();
    w.preserved = Some(Arc::new(Preserved::default()));
    w.sheet_mut(0).unwrap().preserved = Some(Arc::new(Preserved::default()));
    w
}
#[cfg(any(unix, windows))]
pub(super) fn native() -> (Workbook, Metadata) {
    let (mut w, mut m) = minimal();
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStringExt;
        w.source_dir = Some(std::ffi::OsString::from_vec(vec![47, 255]).into());
        w.source_path = Some(std::ffi::OsString::from_vec(vec![47, 254]).into());
        m.original_path_hint = Some(NativePath::Windows(vec![55296, 0]));
    }
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStringExt;
        w.source_dir = Some(std::ffi::OsString::from_wide(&[67, 58, 92]).into());
        w.source_path = Some(std::ffi::OsString::from_wide(&[67, 58, 92, 55296]).into());
        m.original_path_hint = Some(NativePath::Unix(vec![255, 0]));
    }
    (w, m)
}
