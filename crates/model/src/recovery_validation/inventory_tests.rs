//! Literal retained-model field inventory: C = checked typed invariant (including child checks),
//! I = inert retained bytes/flags/enum values, D = deferred owner, retained without walking.
//! All C/I/D data also needs engine raw predecode/node/string/byte budgets. A C field's inert
//! nested fields are separately classified below. Each field pattern is exhaustive, no `..`.
//! Struct inventories are compile-drift guards, not recursive production traversal.
use super::*;
use crate::style::{DxfStated, Protection};
use crate::*;
use gridcraft_core::{Array, CellError, CellRef, DateSystem, MAX_COLS, MAX_ROWS, RangeRef, Value};
use std::sync::Arc;

macro_rules! fields {
    ($ty:path; C[$($c:ident),*]; I[$($i:ident),*]; D[$($d:ident),*]) => {
        const _: fn(&$ty) = |value| { let $ty { $($c: _,)* $($i: _,)* $($d: _,)* } = value; };
    };
}
fields!(WorkbookView; C[]; I[show_sheet_tabs, show_horizontal_scroll, show_vertical_scroll, tab_ratio]; D[]);
fields!(CalcSettings; C[max_change]; I[mode, iterative, max_iterations, precision_as_displayed, r1c1]; D[]);
fields!(DefinedName; C[name, scope]; I[formula, comment, hidden]; D[]);
fields!(DocProps; C[]; I[title, subject, author, company, keywords, description]; D[]);
fields!(PhoneticRun; C[start, end]; I[text]; D[]);
fields!(PhoneticProps; C[]; I[font_id, kind, alignment]; D[]);
fields!(PhoneticString; C[text, runs]; I[props]; D[]);
fields!(Workbook; C[sheets, names, styles, theme, calc, active_sheet, cell_styles, phonetics]; I[structure_password, date_system, show_objects, remove_personal_info, view, props, protected_structure, custom_lists, csv_delimiter]; D[macros, macros_read, macros_unread, vba_project, preserved, source_dir, source_path]);
fields!(Sheet; C[name, cells, cell_pictures, spill, spill_ranges, cols, rows, default_col_width, default_row_height, merges, freeze, tab_color, cond_formats, validations, tables, what_if_tables, autofilter, comments, hyperlinks, charts, images, shapes, sparklines, slicers, controls, pivots, edit_ranges, print, view_active, view_top_left]; I[protection, visibility, show_gridlines, show_headings, show_formulas, show_zeros, show_outline_symbols, summary_below, summary_right, right_to_left, zoom, view]; D[uid, file_id, preserved]);
fields!(Formula; C[array]; I[text, dynamic, eval_mode]; D[expr]);
fields!(CellPicture; C[]; I[mime, alt]; D[data]);
fields!(Cell; C[value, formula, style, py]; I[]; D[]);
fields!(Theme; C[colors]; I[name, major_font, minor_font, major_font_ea, minor_font_ea, major_font_cs, minor_font_cs, major_font_scripts, minor_font_scripts]; D[]);
fields!(Font; C[size, color]; I[name, bold, italic, underline, strike, vert, charset, family, minor]; D[]);
fields!(Fill; C[fg, bg]; I[pattern]; D[]);
fields!(BorderLine; C[color]; I[style]; D[]);
fields!(Borders; C[left, right, top, bottom, diag_down, diag_up]; I[]; D[]);
fields!(Alignment; C[rotation]; I[h, v, wrap, shrink, indent]; D[]);
fields!(Protection; C[]; I[locked, hidden]; D[]);
fields!(Style; C[font, fill, borders, align]; I[num_fmt, protection, dxf_stated]; D[]);
fields!(DxfStated; C[]; I[bold, italic, strike, underline, size, name, scheme, protection]; D[]);
fields!(WhatIfTable; C[anchor, range, input1, input2]; I[formula_text, two_variable, row_oriented, input1_deleted, input2_deleted, calculate]; D[]);
fields!(TableColumn; C[name]; I[totals, totals_label, formula, unique_name, xml]; D[]);
fields!(XmlColumnPr; C[]; I[map_id, xpath, data_type, denormalized]; D[]);
fields!(Table; C[name, range, columns, filter]; I[id, header_row, totals_row, style, banded_rows, banded_cols, first_col, last_col, filter_button, table_type, connection_id]; D[]);
fields!(CondFormat; C[ranges, rule]; I[priority, stop_if_true]; D[]);
fields!(Validation; C[ranges]; I[kind, op, f1, f2, allow_blank, in_cell_dropdown, input_title, input_message, show_input, error_title, error_message, error_style, show_error]; D[]);
fields!(Comment; C[note_box]; I[author, text, replies, threaded, resolved, visible, dates]; D[]);
fields!(NoteBox; C[size_pt]; I[from, to]; D[]);
fields!(NoteCorner; C[]; I[col, col_px, row, row_px]; D[]);
fields!(Hyperlink; C[]; I[target, tooltip, external]; D[]);
fields!(Anchor; C[cell, dx, dy, width, height]; I[mode]; D[]);
fields!(Series; C[color, trendlines]; I[name, categories, values, bubble_sizes, secondary, kind, smooth, quartiles]; D[]);
fields!(Trendline; C[forward, backward, intercept, color, width]; I[kind, name, display_equation, display_r_squared, dash]; D[]);
fields!(ChartDataTable; C[]; I[show_keys, horizontal_border, vertical_border, outline]; D[]);
fields!(Chart; C[id, anchor, series]; I[kind, title, legend, data_labels, data_table, gridlines, style, hole_size, first_slice_angle, x_title, y_title, source, by_rows, pivot]; D[]);
fields!(Image; C[id, anchor]; I[mime, alt]; D[data]);
fields!(Shape; C[id, anchor, fill, line, format]; I[kind, text]; D[math]);
fields!(FormControl; C[id, anchor, value, min, max, step, page]; I[kind, name, text, link, horizontal, print, locked]; D[action]);
fields!(ShapeFormat; C[font_size, text_color, text_outline]; I[no_line, bold, italic, font, align, anchor, rtl]; D[]);
fields!(Sparkline; C[cell, color]; I[source, kind, markers]; D[]);
fields!(AutoFilter; C[range, criteria]; I[]; D[]);
fields!(Slicer; C[id, anchor]; I[table, column, caption, selected, columns]; D[]);
fields!(PivotField; C[]; I[source_col, sort, date_group, collapsed_items]; D[]);
fields!(PivotValue; C[]; I[source_col, func, name, show_as, number_format]; D[]);
fields!(PivotFilter; C[]; I[source_col, selected]; D[]);
fields!(PivotTable; C[id, name, anchor, rows, columns, values, filters, last_range, calculated_fields, values_position]; I[source, layout, enable_show_details, grand_totals_rows, grand_totals_cols, subtotals, subtotals_top, style, show_headers, values_on_rows]; D[]);
fields!(PivotCalcField; C[name]; I[formula]; D[]);
fields!(PrintSettings; C[margins, print_area, title_rows, title_cols, row_breaks, col_breaks]; I[orientation, paper, scale, fit_to, gridlines, headings, center_h, center_v, header, footer, different_first, different_odd_even, first_header, first_footer, even_header, even_footer, hf_scale_with_doc, hf_align_with_margins, page_order]; D[]);
fields!(SheetProtection; C[]; I[password, legacy_password_hash, select_locked, select_unlocked, format_cells, format_columns, format_rows, insert_columns, insert_rows, delete_columns, delete_rows, sort, autofilter]; D[]);
fields!(EditRange; C[ranges]; I[password, title, security_descriptor]; D[]);
fields!(LineInfo; C[size, style]; I[hidden, outline, collapsed, custom]; D[]);
fields!(PyCode; C[code, output]; I[object, stale]; D[]);
fields!(PyOutput; C[value]; I[type_name, preview, convertible]; D[]);
fields!(Macro; C[]; I[]; D[v, name, description, shortcut, inputs, steps, output, permissions]);
fields!(NamedInput; C[]; I[]; D[name, kind, rows, cols, header, col_types, non_empty, default_ref]);
fields!(Step; C[]; I[]; D[kind, enabled, breakpoint, note]);
fields!(OutputRule; C[]; I[]; D[anchor, on_conflict]);
fields!(PreservedRel; C[]; I[]; D[id, rel_type, target, external]);
fields!(PreservedPart; C[]; I[]; D[name, content_type, data, rels]);
fields!(PreservedXml; C[]; I[]; D[slot, xml]);
fields!(PreservedSheet; C[]; I[]; D[name, visibility, rel_id, before, sheet_id]);
fields!(Preserved; C[]; I[]; D[package_rels, rels, parts, elements, vml, sheets]);
fields!(Array; C[rows, cols, data]; I[]; D[]);
fields!(CellRef; C[row, col]; I[]; D[]);
fields!(RangeRef; C[start, end]; I[]; D[]);

// Private/derived boundaries, explicitly not recursively compile-drift guarded here:
// CellStore.bands/len, Band.rows: C by validate_recovery_storage before any iteration;
// Band.accounting: D derived OnceLock<AllocationSummary>, unchanged (store owner tests).
// AllocationSummary.{owned,shared} (shared entries are (pointer,bytes) tuples): D retained accounting.
// StyleTable.styles: C by len+iter; StyleTable.index: D derived hash cache, never queried/rebuilt.
// ActiveSheet.0 atomic: C sampled once by structure; D capture coherence to engine.
// SheetUid.0: D runtime identity; Macros.0 Arc<Vec<Macro>>: D executable owner, never walked.
// EquationCache.{key,len,source,layout}: D equation adoption/layout; public get seam checks purity.
// Formula.expr Arc<Expr>: D parser/dependency/adoption owner; no AST walk/reparse/clone.
// NumFmt.0 Arc<str>: I; StyleId.0: C direct table bound; Color payloads: C P1a.
// CellMap is a serde module, not a stored wrapper; comments/hyperlinks are actual BTreeMaps.
// LineIndex.{default,max,lines}: D consumer-only projection, not reachable from stored Workbook.
// Arbitrary serde_json::{Value,Map,Number} action/future/read trees: D engine before helpers.
// Preserved XML/parts/relationships: D codec graph/byte/adoption owner; no XML/decode/writer.
// Vec/map/Arc/String/path implementation fields and core date internals are owner boundaries.
// Visibility below includes worksheets only; preserved chart-sheet namespace is codec-owned.

// Enum dispositions (all alternatives compile-fenced below): unit enums are I. Value::Number
// is C finite; Value::Array is C shape/scalar leaves; other Value payloads are I. Color::Rgb
// and Color::Theme slot/tint are C. CfValueKind::Number/Percent/Percentile are C finite;
// Formula text and Min/Max are I. CfRule: C embedded style in every style-bearing arm,
// ColorScale.stops values/colors, DataBar.{min,max,color}, IconSet.thresholds; I every other
// named payload shown below. FilterCriterion: C FillColor/FontColor; I all other payloads.
// TrendlineKind numeric degree/period are I, bounded by their consumer rather than this gate.
// PasswordHash::Legacy and Iterated.{algorithm,hash,salt,spin_count} are I without KDF.
// ControlAction::{Macro,Command}, MacroInput and ALL StepKind payloads are D(engine executable
// adoption/budgets); RepeatOf/CompareOp/InputKind/ColType/OnConflict are likewise D, not consent.
// HfPage is a selector, not stored data; each header/footer field is I and exercised below.

macro_rules! variants {
    ($ty:ident; $($pattern:pat_param)|+) => {
        const _: fn(&$ty) = |value| { match value { $($pattern)|+ => {} } };
    };
}
variants!(CalcMode; CalcMode::Automatic | CalcMode::AutomaticExceptTables | CalcMode::Manual);
variants!(ShowObjects; ShowObjects::All | ShowObjects::Placeholders | ShowObjects::None);
variants!(Visibility; Visibility::Visible | Visibility::Hidden | Visibility::VeryHidden);
variants!(SheetViewMode; SheetViewMode::Normal | SheetViewMode::PageLayout | SheetViewMode::PageBreakPreview);
variants!(FormulaEvalMode; FormulaEvalMode::Array | FormulaEvalMode::Legacy);
variants!(Color; Color::Auto | Color::Rgb(_) | Color::Theme(_, _));
variants!(Underline; Underline::None | Underline::Single | Underline::Double | Underline::SingleAccounting | Underline::DoubleAccounting);
variants!(VertAlign; VertAlign::Baseline | VertAlign::Superscript | VertAlign::Subscript);
variants!(PatternType; PatternType::None | PatternType::Solid | PatternType::Gray50 | PatternType::Gray75 | PatternType::Gray25 | PatternType::Gray125 | PatternType::Gray0625 | PatternType::DarkHorizontal | PatternType::DarkVertical | PatternType::DarkDown | PatternType::DarkUp | PatternType::DarkGrid | PatternType::DarkTrellis | PatternType::LightHorizontal | PatternType::LightVertical | PatternType::LightDown | PatternType::LightUp | PatternType::LightGrid | PatternType::LightTrellis);
variants!(BorderStyle; BorderStyle::None | BorderStyle::Thin | BorderStyle::Medium | BorderStyle::Thick | BorderStyle::Dashed | BorderStyle::Dotted | BorderStyle::Double | BorderStyle::Hair | BorderStyle::MediumDashed | BorderStyle::DashDot | BorderStyle::MediumDashDot | BorderStyle::DashDotDot | BorderStyle::MediumDashDotDot | BorderStyle::SlantDashDot);
variants!(HAlign; HAlign::General | HAlign::Left | HAlign::Center | HAlign::Right | HAlign::Fill | HAlign::Justify | HAlign::CenterAcross | HAlign::Distributed);
variants!(VAlign; VAlign::Top | VAlign::Center | VAlign::Bottom | VAlign::Justify | VAlign::Distributed);
variants!(TotalsFn; TotalsFn::None | TotalsFn::Average | TotalsFn::Count | TotalsFn::CountNums | TotalsFn::Max | TotalsFn::Min | TotalsFn::Sum | TotalsFn::StdDev | TotalsFn::Var | TotalsFn::Custom);
variants!(CfOperator; CfOperator::Between | CfOperator::NotBetween | CfOperator::Equal | CfOperator::NotEqual | CfOperator::Greater | CfOperator::Less | CfOperator::GreaterOrEqual | CfOperator::LessOrEqual);
variants!(CfValueKind; CfValueKind::Min | CfValueKind::Max | CfValueKind::Number(_) | CfValueKind::Percent(_) | CfValueKind::Percentile(_) | CfValueKind::Formula(_));
variants!(CfRule; CfRule::CellIs{ op: _, a: _, b: _, style: _ } | CfRule::Expression{ formula: _, style: _ } | CfRule::ContainsText{ text: _, style: _ } | CfRule::NotContainsText{ text: _, style: _ } | CfRule::BeginsWith{ text: _, style: _ } | CfRule::EndsWith{ text: _, style: _ } | CfRule::Blanks{ style: _ } | CfRule::NoBlanks{ style: _ } | CfRule::Errors{ style: _ } | CfRule::NoErrors{ style: _ } | CfRule::Duplicate{ style: _ } | CfRule::Unique{ style: _ } | CfRule::Top10{ bottom: _, percent: _, rank: _, style: _ } | CfRule::AboveAverage{ below: _, equal: _, std_dev: _, style: _ } | CfRule::TimePeriod{ period: _, style: _ } | CfRule::ColorScale{ stops: _ } | CfRule::DataBar{ min: _, max: _, color: _, gradient: _, show_value: _ } | CfRule::IconSet{ set: _, thresholds: _, reverse: _, show_value: _ });
variants!(ValidationKind; ValidationKind::Any | ValidationKind::Whole | ValidationKind::Decimal | ValidationKind::List | ValidationKind::Date | ValidationKind::Time | ValidationKind::TextLength | ValidationKind::Custom);
variants!(ErrorStyle; ErrorStyle::Stop | ErrorStyle::Warning | ErrorStyle::Information);
variants!(AnchorMode; AnchorMode::MoveAndSize | AnchorMode::MoveOnly | AnchorMode::Absolute);
variants!(ChartKind; ChartKind::ColumnClustered | ChartKind::ColumnStacked | ChartKind::ColumnStacked100 | ChartKind::BarClustered | ChartKind::BarStacked | ChartKind::BarStacked100 | ChartKind::Line | ChartKind::LineMarkers | ChartKind::LineStacked | ChartKind::Pie | ChartKind::Doughnut | ChartKind::Area | ChartKind::AreaStacked | ChartKind::AreaStacked100 | ChartKind::Scatter | ChartKind::ScatterLines | ChartKind::Bubble | ChartKind::Radar | ChartKind::Histogram | ChartKind::Waterfall | ChartKind::Funnel | ChartKind::Treemap | ChartKind::Sunburst | ChartKind::BoxWhisker | ChartKind::Stock | ChartKind::Combo);
variants!(LegendPos; LegendPos::None | LegendPos::Bottom | LegendPos::Top | LegendPos::Left | LegendPos::Right);
variants!(QuartileMethod; QuartileMethod::Exclusive | QuartileMethod::Inclusive);
variants!(TrendlineKind; TrendlineKind::Linear | TrendlineKind::Exponential | TrendlineKind::Logarithmic | TrendlineKind::Polynomial(_) | TrendlineKind::Power | TrendlineKind::MovingAverage(_));
variants!(LineDash; LineDash::Solid | LineDash::Dot | LineDash::Dash | LineDash::LgDash | LineDash::DashDot | LineDash::LgDashDot | LineDash::LgDashDotDot | LineDash::SysDash | LineDash::SysDot | LineDash::SysDashDot | LineDash::SysDashDotDot);
variants!(ShapeKind; ShapeKind::Rectangle | ShapeKind::RoundedRectangle | ShapeKind::Ellipse | ShapeKind::Triangle | ShapeKind::Line | ShapeKind::Arrow | ShapeKind::TextBox | ShapeKind::Icon | ShapeKind::Ink | ShapeKind::Equation);
variants!(ControlKind; ControlKind::Button | ControlKind::Slider | ControlKind::Spin);
variants!(ControlAction; ControlAction::Macro{ name: _, input: _, output: _ } | ControlAction::Command{ id: _, params: _ });
variants!(MacroInput; MacroInput::Selection | MacroInput::Fixed(_));
variants!(ShapeTextAlign; ShapeTextAlign::Left | ShapeTextAlign::Center | ShapeTextAlign::Right);
variants!(ShapeTextAnchor; ShapeTextAnchor::Top | ShapeTextAnchor::Middle | ShapeTextAnchor::Bottom);
variants!(SparklineKind; SparklineKind::Line | SparklineKind::Column | SparklineKind::WinLoss);
variants!(FilterCriterion; FilterCriterion::Values{ values: _, blanks: _ } | FilterCriterion::Custom{ a: _, b: _, and: _ } | FilterCriterion::Top10{ bottom: _, percent: _, count: _ } | FilterCriterion::AboveAverage(_) | FilterCriterion::FillColor(_) | FilterCriterion::FontColor(_) | FilterCriterion::Icon{ set: _, id: _ });
variants!(PivotLayout; PivotLayout::Compact | PivotLayout::Outline | PivotLayout::Tabular);
variants!(PivotSort; PivotSort::Asc | PivotSort::Desc | PivotSort::None);
variants!(PivotDateGroup; PivotDateGroup::None | PivotDateGroup::Years | PivotDateGroup::Quarters | PivotDateGroup::Months | PivotDateGroup::Days);
variants!(PivotFunc; PivotFunc::Sum | PivotFunc::Count | PivotFunc::Average | PivotFunc::Max | PivotFunc::Min | PivotFunc::Product | PivotFunc::CountNumbers | PivotFunc::StdDev | PivotFunc::StdDevP | PivotFunc::Var | PivotFunc::VarP);
variants!(PivotShowAs; PivotShowAs::Normal | PivotShowAs::PercentOfGrandTotal | PivotShowAs::PercentOfColumnTotal | PivotShowAs::PercentOfRowTotal | PivotShowAs::RunningTotal | PivotShowAs::Rank);
variants!(Orientation; Orientation::Portrait | Orientation::Landscape);
variants!(PageOrder; PageOrder::DownThenOver | PageOrder::OverThenDown);
variants!(HfPage; HfPage::Odd | HfPage::Even | HfPage::First);
variants!(InputKind; InputKind::Cell | InputKind::Range);
variants!(ColType; ColType::Number | ColType::Text | ColType::Date | ColType::Bool | ColType::Any);
variants!(StepKind; StepKind::Command{ id: _, params: _ } | StepKind::Repeat{ over: _, of: _, max: _, body: _ } | StepKind::If{ cell: _, op: _, value: _, then: _, otherwise: _ } | StepKind::Stop{ message: _, rollback: _ });
variants!(RepeatOf; RepeatOf::Rows | RepeatOf::Columns);
variants!(CompareOp; CompareOp::Eq | CompareOp::Ne | CompareOp::Gt | CompareOp::Ge | CompareOp::Lt | CompareOp::Le | CompareOp::Empty | CompareOp::NotEmpty | CompareOp::Contains);
variants!(OnConflict; OnConflict::Ask | OnConflict::Fail | OnConflict::Overwrite | OnConflict::ShiftDown | OnConflict::ShiftRight);
variants!(CellError; CellError::Null | CellError::Div0 | CellError::Value | CellError::Ref | CellError::Name | CellError::Num | CellError::NA | CellError::GettingData | CellError::Spill | CellError::Calc | CellError::Circ | CellError::Python | CellError::Busy | CellError::Timeout | CellError::Blocked);
variants!(Value; Value::Empty | Value::Number(_) | Value::Text(_) | Value::Bool(_) | Value::Error(_) | Value::Array(_));
variants!(DateSystem; DateSystem::D1900 | DateSystem::D1904);

variants!(PasswordHash; PasswordHash::Legacy(_) | PasswordHash::Iterated { algorithm: _, hash: _, salt: _, spin_count: _ });

mod cases;
mod fixtures;
