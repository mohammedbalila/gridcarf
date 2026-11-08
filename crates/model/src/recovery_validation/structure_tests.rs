//! B1 contract fixtures; the complete model entry point is deliberately a later checkpoint.
use super::*;
use crate::{CalcMode, Cell, DefinedName, Formula, LineInfo, SheetViewMode, Style, StyleId, Visibility};
use gridcraft_core::{CellRef, MAX_ROWS, Value};
use std::sync::Arc;

fn rect(r0: u32, c0: u32, r1: u32, c1: u32) -> RangeRef {
    RangeRef { start: CellRef::new(r0, c0), end: CellRef::new(r1, c1) }
}
fn named(name: &str, scope: Option<usize>) -> DefinedName {
    DefinedName { name: name.into(), scope, formula: "[external.xlsx]missing!bad( recursive".into(), comment: "inert".into(), hidden: true }
}
fn overlaps() -> RecoveryModelError {
    RecoveryModelError::UnsupportedRepresentation(super::super::RecoveryRepresentation::OverlappingRectangles)
}
fn ambiguity() -> RecoveryModelError {
    RecoveryModelError::UnsupportedRepresentation(super::super::RecoveryRepresentation::AmbiguousIdentity)
}
fn ranges(ranges: &[RangeRef]) -> Result<(), RecoveryModelError> {
    validate_disjoint_rectangles(ranges.iter().copied())
}

#[test]
fn recovery_structure_default_multiple_manual_and_irregular_bounded_settings() {
    assert_eq!(validate_workbook_structure(&Workbook::default()), Ok(1));
    let mut wb = Workbook::new();
    wb.sheets.extend([Arc::new(Sheet::new("History")), Arc::new(Sheet::new("'odd:/[]?* very long imported name beyond thirty one'"))]);
    wb.active_sheet.set(2);
    wb.calc.mode = CalcMode::Manual;
    wb.calc.iterative = true;
    wb.calc.precision_as_displayed = true;
    wb.calc.r1c1 = true;
    wb.view.show_sheet_tabs = false;
    wb.view.show_horizontal_scroll = false;
    wb.view.show_vertical_scroll = false;
    wb.view.tab_ratio = u32::MAX;
    for change in [-f64::MAX, -0.0, 0.0, f64::MAX] {
        wb.calc.max_change = change;
        for iterations in [0, 1, u32::MAX] {
            wb.calc.max_iterations = iterations;
            assert_eq!(validate_workbook_structure(&wb), Ok(1));
            assert_eq!(wb.calc.max_change.to_bits(), change.to_bits());
            assert_eq!(wb.calc.max_iterations, iterations);
        }
    }
    assert_eq!(wb.active_sheet.get(), 2);
    assert_eq!(wb.view.tab_ratio, u32::MAX);
}

#[test]
fn recovery_structure_names_follow_actual_ascii_scope_and_no_editor_grammar() {
    let mut wb = Workbook::new();
    wb.sheets = ["", "Ä", "ä", " History "].into_iter().map(|name| Arc::new(Sheet::new(name))).collect();
    wb.names = vec![named("ODD NAME!", None), named("odd name!", Some(0)), named("odd name!", Some(1)), named("Ä", None), named("ä", None)];
    wb.cell_styles = ["Normal", "Ä", "ä", ""].into_iter().map(|name| (name.into(), Style::default())).collect();
    let before = wb.clone();
    assert_eq!(validate_workbook_structure(&wb), Ok(1));
    assert_eq!(wb, before);
    // These are the real lookup seams whose first-match semantics motivate ambiguity refusal.
    assert_eq!(wb.sheet_index(" HISTORY "), Some(3));
    assert_eq!(wb.sheet_index("ä"), Some(2));
    assert!(std::ptr::eq(wb.name("Odd Name!", 0).unwrap(), &wb.names[1]));
    assert!(std::ptr::eq(wb.name("Odd Name!", 3).unwrap(), &wb.names[0]));
    // Whitespace is significant in these namespaces, unlike later pivot calculated-field keys.
    wb.names.push(named(" ODD NAME!", None));
    wb.cell_styles.push((" Normal".into(), Style::default()));
    assert_eq!(validate_workbook_structure(&wb), Ok(1));
}

#[test]
fn recovery_structure_ambiguous_ascii_sheet_name_and_named_style_identities_refuse() {
    let mut wb = Workbook::new();
    wb.sheets.push(Arc::new(Sheet::new("sHeEt1")));
    assert_eq!(wb.sheet_index("sheet1"), Some(0));
    assert_eq!(validate_workbook_structure(&wb), Err(ambiguity()));
    for scope in [None, Some(0)] {
        let mut wb = Workbook::new();
        wb.names = vec![named("same", scope), named("SAME", scope)];
        assert_eq!(validate_workbook_structure(&wb), Err(ambiguity()));
    }
    let mut wb = Workbook::new();
    wb.cell_styles = vec![("Normal".into(), Style::default()), ("NORMAL".into(), Style::default())];
    assert_eq!(validate_workbook_structure(&wb), Err(ambiguity()));
    let entries = [(Some(1), "x"), (None, "X"), (Some(0), "X")];
    assert_eq!(validate_ascii_identities(entries.into_iter()), Ok(()));
}

#[test]
fn recovery_structure_empty_sheets_bad_active_and_bad_scope_are_malformed() {
    let mut wb = Workbook::new();
    wb.active_sheet.set(1);
    assert_eq!(validate_workbook_structure(&wb), Err(RecoveryModelError::Workbook));
    wb.active_sheet.set(0);
    for scope in [1, usize::MAX] {
        wb.names = vec![named("retained", Some(scope))];
        assert_eq!(validate_workbook_structure(&wb), Err(RecoveryModelError::Workbook));
    }
    wb.names.clear();
    wb.sheets.clear();
    assert_eq!(validate_workbook_structure(&wb), Err(RecoveryModelError::Workbook));
}

#[test]
fn recovery_structure_zero_hidden_edge_lines_views_and_maximum_freeze_are_retained() {
    let mut wb = Workbook::new();
    let sheet = wb.sheet_mut(0).unwrap();
    sheet
        .rows
        .insert(MAX_ROWS - 1, LineInfo { size: Some(-0.0), hidden: true, outline: 255, collapsed: true, style: Some(StyleId(0)), custom: true });
    sheet.cols.insert(MAX_COLS - 1, LineInfo { size: None, ..LineInfo::default() });
    sheet.rows.insert(0, LineInfo { size: Some(f32::MAX), ..LineInfo::default() });
    sheet.cols.insert(0, LineInfo { size: Some(0.0), ..LineInfo::default() });
    sheet.default_col_width = f32::MAX;
    sheet.default_row_height = 0.0;
    sheet.freeze = Some((MAX_ROWS, MAX_COLS));
    sheet.view_active = CellRef::new(MAX_ROWS - 1, MAX_COLS - 1);
    sheet.view_top_left = sheet.view_active;
    sheet.tab_color = Some(crate::Color::Theme(11, -1000));
    sheet.visibility = Visibility::VeryHidden;
    sheet.show_gridlines = false;
    sheet.show_headings = false;
    sheet.show_formulas = true;
    sheet.show_zeros = false;
    sheet.show_outline_symbols = false;
    sheet.summary_below = false;
    sheet.summary_right = false;
    sheet.right_to_left = true;
    for mode in [SheetViewMode::Normal, SheetViewMode::PageLayout, SheetViewMode::PageBreakPreview] {
        for zoom in [0, 1, u16::MAX] {
            let sheet = wb.sheet_mut(0).unwrap();
            sheet.view = mode;
            sheet.zoom = zoom;
            let before = wb.clone();
            assert_eq!(validate_workbook_structure(&wb), Ok(1));
            assert_eq!(wb, before);
            assert_eq!(wb.sheets[0].rows[&(MAX_ROWS - 1)].size.unwrap().to_bits(), (-0.0f32).to_bits());
        }
    }
    wb.sheet_mut(0).unwrap().freeze = Some((0, 0));
    wb.sheet_mut(0).unwrap().default_col_width = -0.0;
    wb.sheet_mut(0).unwrap().default_row_height = -0.0;
    assert_eq!(validate_workbook_structure(&wb), Ok(1));
    assert_eq!(wb.sheets[0].default_col_width.to_bits(), (-0.0f32).to_bits());
    assert_eq!(wb.sheets[0].default_row_height.to_bits(), (-0.0f32).to_bits());
}

#[test]
fn recovery_structure_bad_line_keys_geometry_and_style_refs_are_refused() {
    for is_row in [false, true] {
        let mut wb = Workbook::new();
        let limit = if is_row { MAX_ROWS } else { MAX_COLS };
        for key in [limit, u32::MAX] {
            let sheet = wb.sheet_mut(0).unwrap();
            let lines = if is_row { &mut sheet.rows } else { &mut sheet.cols };
            lines.clear();
            lines.insert(key, LineInfo::default());
            assert_eq!(validate_workbook_structure(&wb), Err(RecoveryModelError::Coordinate));
        }
        for size in [-1.0, f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            let sheet = wb.sheet_mut(0).unwrap();
            let lines = if is_row { &mut sheet.rows } else { &mut sheet.cols };
            lines.clear();
            lines.insert(limit - 1, LineInfo { size: Some(size), hidden: true, ..LineInfo::default() });
            assert_eq!(validate_workbook_structure(&wb), Err(RecoveryModelError::Geometry));
            let sheet = &wb.sheets[0];
            let lines = if is_row { &sheet.rows } else { &sheet.cols };
            assert_eq!(lines[&(limit - 1)].size.unwrap().to_bits(), size.to_bits());
        }
        let sheet = wb.sheet_mut(0).unwrap();
        let lines = if is_row { &mut sheet.rows } else { &mut sheet.cols };
        lines.clear();
        lines.insert(0, LineInfo { style: Some(StyleId(1)), ..LineInfo::default() });
        assert_eq!(validate_workbook_structure(&wb), Err(RecoveryModelError::Style));
    }
    for size in [-1.0, f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        for column in [false, true] {
            let mut wb = Workbook::new();
            let sheet = wb.sheet_mut(0).unwrap();
            if column {
                sheet.default_col_width = size;
            } else {
                sheet.default_row_height = size;
            }
            assert_eq!(validate_workbook_structure(&wb), Err(RecoveryModelError::Geometry));
        }
    }
}

#[test]
fn recovery_structure_out_of_grid_views_and_freeze_counts_and_tab_color_refuse() {
    let edits: [fn(&mut Sheet); 6] = [
        |s| s.view_active.row = MAX_ROWS,
        |s| s.view_top_left.col = MAX_COLS,
        |s| s.view_active.col = u32::MAX,
        |s| s.view_top_left.row = u32::MAX,
        |s| s.freeze = Some((MAX_ROWS + 1, 0)),
        |s| s.freeze = Some((0, MAX_COLS + 1)),
    ];
    for edit in edits {
        let mut wb = Workbook::new();
        edit(wb.sheet_mut(0).unwrap());
        assert_eq!(validate_workbook_structure(&wb), Err(RecoveryModelError::Coordinate));
    }
    let mut wb = Workbook::new();
    wb.sheet_mut(0).unwrap().tab_color = Some(crate::Color::Rgb(0x1000000));
    assert_eq!(validate_workbook_structure(&wb), Err(RecoveryModelError::Style));
}

#[test]
fn recovery_structure_embedded_styles_theme_and_calc_check_only_numeric_domains() {
    for size in [-f32::MAX, -0.0, 0.0, f32::MAX] {
        let mut wb = Workbook::new();
        let mut style = Style::default();
        style.font.size = size;
        wb.cell_styles = vec![("Normal".into(), style.clone()), ("other".into(), style.clone())];
        wb.styles = serde_json::from_value(serde_json::json!({"styles": [style.clone(), style]})).unwrap();
        wb.sheet_mut(0).unwrap().rows.insert(1, LineInfo { style: Some(StyleId(1)), ..LineInfo::default() });
        wb.theme.colors = [0xffffff; 12];
        wb.theme.major_font_scripts = vec![("".into(), "".into()), ("Arab".into(), "unknown".into()), ("Arab".into(), "different".into())];
        wb.theme.minor_font_scripts = wb.theme.major_font_scripts.clone();
        wb.theme.major_font_ea = "unknown".into();
        wb.theme.minor_font_cs = "unknown".into();
        let before = wb.clone();
        assert_eq!(validate_workbook_structure(&wb), Ok(2));
        assert_eq!(wb, before); // Includes the empty derived style index and duplicate equal IDs.
        assert_eq!(wb.cell_styles[0].1.font.size.to_bits(), size.to_bits());
    }
    for slot in 0..12 {
        let mut wb = Workbook::new();
        wb.theme.colors[slot] = 0x1000000;
        assert_eq!(validate_workbook_structure(&wb), Err(RecoveryModelError::Style));
    }
    for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        let mut wb = Workbook::new();
        wb.calc.max_change = value;
        assert_eq!(validate_workbook_structure(&wb), Err(RecoveryModelError::Value));
        assert_eq!(wb.calc.max_change.to_bits(), value.to_bits());
    }
    for value in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        let mut wb = Workbook::new();
        let mut style = Style::default();
        style.font.size = value;
        wb.cell_styles.push(("bad".into(), style));
        assert_eq!(validate_workbook_structure(&wb), Err(RecoveryModelError::Style));
    }
}

#[test]
fn recovery_structure_merge_sweep_word_grid_row_ties_and_full_grid_metadata() {
    for first in [0, 1, 63, 64, 65, MAX_COLS - 64, MAX_COLS - 1] {
        assert_eq!(ranges(&[rect(0, first, MAX_ROWS - 1, first)]), Ok(()));
        assert_eq!(ranges(&[rect(0, 0, 0, first), rect(1, 0, MAX_ROWS - 1, first)]), Ok(()));
        if first > 0 {
            assert_eq!(ranges(&[rect(0, 0, MAX_ROWS - 1, first - 1), rect(0, first, MAX_ROWS - 1, MAX_COLS - 1)]), Ok(()));
        }
    }
    assert_eq!(ranges(&[rect(0, 63, 2, 64), rect(3, 63, 4, 64), rect(0, 65, 4, MAX_COLS - 1)]), Ok(()));
    let mut wb = Workbook::new();
    wb.sheet_mut(0).unwrap().merges = vec![RangeRef::all()];
    assert_eq!(validate_workbook_structure(&wb), Ok(1));
    assert!(wb.sheets[0].cells.is_empty());
    assert_eq!(wb.sheets[0].merge_at(CellRef::new(MAX_ROWS - 1, MAX_COLS - 1)), Some(RangeRef::all()));
}

#[test]
fn recovery_structure_merge_inclusive_overlap_nested_and_identical_ranges_refuse() {
    for column in [0, 63, 64, MAX_COLS - 1] {
        assert_eq!(ranges(&[rect(0, column, 2, column), rect(2, column, 3, column)]), Err(overlaps()));
        assert_eq!(ranges(&[rect(0, column, 3, column), rect(1, column, 2, column)]), Err(overlaps()));
        assert_eq!(ranges(&[rect(MAX_ROWS - 1, column, MAX_ROWS - 1, column); 2]), Err(overlaps()));
    }
    assert_eq!(ranges(&[rect(0, 0, 0, 64), rect(0, 63, 0, MAX_COLS - 1)]), Err(overlaps()));
    assert_eq!(ranges(&[RangeRef::all(), rect(MAX_ROWS - 1, MAX_COLS - 1, MAX_ROWS - 1, MAX_COLS - 1)]), Err(overlaps()));
    // Removing the short rectangle must preserve its neighbor's bits in the same word.
    assert_eq!(ranges(&[rect(0, 0, 1, 10), rect(0, 11, 4, 20), rect(2, 11, 3, 20)]), Err(overlaps()));
    assert_eq!(ranges(&[rect(0, 0, 1, 10), rect(0, 11, 4, 20), rect(2, 0, 3, 10)]), Ok(()));
}

#[test]
fn recovery_structure_merge_bad_endpoints_precede_overlap_scratch_work() {
    for bad in [rect(2, 0, 1, 0), rect(0, 2, 0, 1), rect(0, 0, MAX_ROWS, 0), rect(0, MAX_COLS, 0, MAX_COLS)] {
        assert_eq!(ranges(&[RangeRef::all(), RangeRef::all(), bad]), super::super::validate_range(bad).map(|_| ()));
    }
}

#[test]
fn recovery_structure_many_sparse_merges_and_late_events_do_not_expand_areas() {
    let mut disjoint = (0..4096).map(|row| rect(row * 2, row % MAX_COLS, row * 2, row % MAX_COLS)).collect::<Vec<_>>();
    disjoint.push(rect(MAX_ROWS - 1, MAX_COLS - 1, MAX_ROWS - 1, MAX_COLS - 1));
    disjoint.reverse();
    assert_eq!(ranges(&disjoint), Ok(()));
    disjoint.push(rect(MAX_ROWS - 1, MAX_COLS - 1, MAX_ROWS - 1, MAX_COLS - 1));
    assert_eq!(ranges(&disjoint), Err(overlaps()));
    assert_eq!(ranges(&[]), Ok(()));
}

#[test]
fn recovery_structure_success_and_late_failure_preserve_shared_derived_and_atomic_state() {
    let mut wb = Workbook::new();
    wb.calc.mode = CalcMode::Manual;
    let formula = Arc::new(Formula::new("=SUM(1,2)"));
    wb.sheet_mut(0).unwrap().cells.set(CellRef::new(0, 0), Cell { formula: Some(formula.clone()), value: Value::Number(-0.0), ..Cell::default() });
    wb.sheets.push(Arc::new(Sheet::new("last")));
    wb.active_sheet.set(1);
    wb.macros_unread = Arc::new(vec![serde_json::json!({"unknown": ["retained"]})]);
    wb.vba_project = Some(Arc::new(vec![0, 1, 2]));
    for failing in [false, true] {
        if failing {
            wb.sheet_mut(1).unwrap().merges = vec![RangeRef::all(), RangeRef::all()];
        }
        let before = wb.clone();
        let sheet_pointer = Arc::as_ptr(&wb.sheets[0]);
        let counts = (
            Arc::strong_count(&wb.sheets[0]),
            Arc::strong_count(&wb.sheets[0].comments),
            Arc::strong_count(&formula),
            Arc::strong_count(&wb.macros_unread),
        );
        let uid = wb.sheets[0].uid.get();
        assert_eq!(validate_workbook_structure(&wb), if failing { Err(overlaps()) } else { Ok(1) });
        assert_eq!(wb, before);
        assert_eq!(Arc::as_ptr(&wb.sheets[0]), sheet_pointer);
        assert_eq!(
            (
                Arc::strong_count(&wb.sheets[0]),
                Arc::strong_count(&wb.sheets[0].comments),
                Arc::strong_count(&formula),
                Arc::strong_count(&wb.macros_unread)
            ),
            counts
        );
        assert_eq!(wb.sheets[0].uid.get(), uid);
        assert_eq!(wb.active_sheet.get(), 1);
        assert!(Arc::ptr_eq(&wb.macros_unread, &before.macros_unread));
        assert!(Arc::ptr_eq(wb.vba_project.as_ref().unwrap(), before.vba_project.as_ref().unwrap()));
        assert!(Arc::ptr_eq(wb.sheets[0].cells.get(CellRef::new(0, 0)).unwrap().formula.as_ref().unwrap(), &formula));
        let Value::Number(n) = wb.sheets[0].cells.get(CellRef::new(0, 0)).unwrap().value else {
            panic!("number");
        };
        assert_eq!(n.to_bits(), (-0.0f64).to_bits());
    }
}
