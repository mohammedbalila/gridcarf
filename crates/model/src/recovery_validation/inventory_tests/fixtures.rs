use super::*;

pub(super) fn at(row: u32, col: u32) -> CellRef {
    CellRef::new(row, col)
}
pub(super) fn rect(r0: u32, c0: u32, r1: u32, c1: u32) -> RangeRef {
    RangeRef { start: at(r0, c0), end: at(r1, c1) }
}

/// Every stored family is populated. External/stale/inert data deliberately disagrees with
/// ordinary editor defaults. Variants that cannot coexist are exercised separately below.
pub(super) fn inventory_workbook() -> Workbook {
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
    let style = Style {
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
        protection: Protection { locked: false, hidden: true },
        dxf_stated: DxfStated {
            bold: true,
            italic: true,
            strike: true,
            underline: true,
            size: true,
            name: true,
            scheme: Some("future".into()),
            protection: true,
        },
    };
    let id = wb.styles.intern(style.clone());
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
        name: " raw name ".into(),
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
    wb.macros = vec![Macro {
        v: u32::MAX,
        name: "  ".into(),
        description: "desc".into(),
        shortcut: Some("raw".into()),
        inputs: vec![NamedInput {
            name: "duplicate".into(),
            kind: InputKind::Cell,
            rows: [9, 1],
            cols: [u32::MAX, 0],
            header: true,
            col_types: vec![ColType::Any, ColType::Number, ColType::Text, ColType::Date, ColType::Bool],
            non_empty: true,
            default_ref: Some("broken(".into()),
        }],
        steps: vec![Step {
            kind: StepKind::Command { id: "".into(), params: serde_json::json!({"opaque": [1,2,3]}) },
            enabled: false,
            breakpoint: true,
            note: "note".into(),
        }],
        output: OutputRule { anchor: Some("bad(".into()), on_conflict: OnConflict::ShiftRight },
        permissions: vec!["untrusted".into()],
    }]
    .into();
    wb.macros_read = vec![serde_json::json!({"pending": true})];
    wb.macros_unread = Arc::new(vec![serde_json::json!({"future": 99})]);
    let rel = PreservedRel { id: "raw".into(), rel_type: "unknown".into(), target: "../opaque".into(), external: true };
    let part = PreservedPart { name: "opaque".into(), content_type: "unknown".into(), data: Arc::from([1u8, 2, 255]), rels: vec![rel.clone()] };
    wb.preserved = Some(Arc::new(Preserved {
        package_rels: vec![rel.clone()],
        rels: vec![rel],
        parts: vec![part.clone()],
        elements: vec![PreservedXml { slot: "future".into(), xml: "unparsed XML".into() }],
        vml: Some(part),
        sheets: vec![PreservedSheet {
            name: "chart sheet".into(),
            visibility: Visibility::VeryHidden,
            rel_id: "opaque".into(),
            before: 99,
            sheet_id: u32::MAX,
        }],
        native_analysis: None,
    }));
    wb.source_dir = Some("synthetic/context".into());
    wb.source_path = Some("synthetic/book".into());
    let preserved = wb.preserved.clone();
    let sheet = wb.sheet_mut(0).unwrap();
    sheet.file_id = u32::MAX;
    sheet.preserved = preserved;
    sheet.default_col_width = f32::MAX;
    sheet.default_row_height = f32::MAX;
    sheet.cols.insert(MAX_COLS - 1, LineInfo { size: Some(-0.0), hidden: true, outline: 255, collapsed: true, style: Some(id), custom: true });
    sheet.rows.insert(MAX_ROWS - 1, LineInfo { size: Some(f32::MAX), hidden: true, outline: 255, collapsed: true, style: Some(id), custom: true });
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
    let formula = Formula {
        text: "=retained broken(".into(),
        expr: Formula::new("=1+2").expr,
        array: Some(rect(0, 0, 0, 1)),
        dynamic: true,
        eval_mode: FormulaEvalMode::Legacy,
    };
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
    sheet.cell_pictures.insert(at(1, 1), Arc::new(CellPicture { data: vec![255], mime: "unknown".into(), alt: "raw".into() }));
    sheet.tables = vec![Table {
        id: 0,
        name: " table ".into(),
        range: rect(2, 0, 2, 0),
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
    sheet.what_if_tables = vec![WhatIfTable {
        anchor: at(3, 0),
        range: rect(3, 0, 3, 1),
        formula_text: "bad(".into(),
        input1: Some(at(9, 9)),
        input2: None,
        two_variable: Some(true),
        row_oriented: Some(false),
        input1_deleted: Some(false),
        input2_deleted: Some(true),
        calculate: Some(false),
    }];
    sheet.autofilter = Some(AutoFilter {
        range: RangeRef::all(),
        criteria: vec![(MAX_COLS - 1, FilterCriterion::Icon { set: "future".into(), id: Some(u32::MAX) })],
    });
    sheet.cond_formats = vec![CondFormat {
        ranges: vec![RangeRef::all()],
        rule: CfRule::Expression { formula: "bad(".into(), style: Box::new(style) },
        priority: 0,
        stop_if_true: true,
    }];
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
    let anchor =
        Anchor { cell: at(MAX_ROWS - 1, MAX_COLS - 1), dx: -f32::MAX, dy: f32::MAX, width: f32::MAX, height: -0.0, mode: AnchorMode::Absolute };
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
                forward: -f64::MAX,
                backward: f64::MAX,
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
    sheet.images = vec![Image { id: 1, anchor, data: Arc::new(vec![255]), mime: "unknown".into(), alt: "raw".into() }];
    let mut math = EquationCache::default();
    math.refuse("bad(", "retained cache");
    sheet.shapes = vec![Shape {
        id: 2,
        kind: ShapeKind::Equation,
        anchor,
        fill: Color::Auto,
        line: Color::Rgb(0),
        text: "bad(".into(),
        math,
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
                from: NoteCorner { row: i64::MIN, col: i64::MAX, row_px: u32::MAX, col_px: 0 },
                to: NoteCorner { row: i64::MAX, col: i64::MIN, row_px: 0, col_px: u32::MAX },
                size_pt: Some((-0.0, f64::MAX)),
            }),
        },
    );
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
        margins: [-0.0, f32::MAX, 0.0, 1.0, 2.0, 3.0],
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
    wb
}
