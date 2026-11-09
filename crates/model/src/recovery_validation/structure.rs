//! Structural envelope only: features, objects and metadata retain their later owner gates.
use super::{
    RecoveryModelError, RecoveryRepresentation, validate_cell_ref, validate_color, validate_range, validate_style, validate_style_id,
    validate_style_table,
};
use crate::{CalcSettings, DefinedName, LineInfo, Sheet, Theme, Workbook, WorkbookView};
use gridcraft_core::{MAX_COLS, MAX_ROWS, RangeRef};

/// Borrowed scoped keys follow actual ASCII lookup semantics, without allocating folded strings.
/// Comparison work is O(N log N * longest key bytes), not byte-independent O(N log N).
pub(super) fn validate_ascii_identities<'a>(entries: impl Clone + Iterator<Item = (Option<usize>, &'a str)>) -> Result<(), RecoveryModelError> {
    let count = entries.clone().try_fold(0usize, |n, _| n.checked_add(1).ok_or(RecoveryModelError::Capacity))?;
    if count.checked_mul(std::mem::size_of::<(Option<usize>, &str)>()).is_none_or(|bytes| bytes > isize::MAX as usize) {
        return Err(RecoveryModelError::Capacity);
    }
    let mut keys = Vec::new();
    keys.try_reserve_exact(count).map_err(|_| RecoveryModelError::Capacity)?;
    keys.extend(entries);
    keys.sort_unstable_by(|(scope_a, a), (scope_b, b)| {
        scope_a.cmp(scope_b).then_with(|| a.bytes().map(|b| b.to_ascii_lowercase()).cmp(b.bytes().map(|b| b.to_ascii_lowercase())))
    });
    if keys.windows(2).any(|pair| pair[0].0 == pair[1].0 && pair[0].1.eq_ignore_ascii_case(pair[1].1)) {
        return Err(RecoveryModelError::UnsupportedRepresentation(RecoveryRepresentation::AmbiguousIdentity));
    }
    Ok(())
}

#[derive(Clone, Copy)]
struct RectangleEvent {
    row: u32,
    start: bool,
    first: u32,
    last: u32,
}

/// Separate sweeps for merges and tables; metadata from different families may overlap.
/// Fixed 256 words (2 KiB), <=256 touched words/event; O(R log R + 256R), no area walk.
pub(super) fn validate_disjoint_rectangles(ranges: impl Clone + ExactSizeIterator<Item = RangeRef>) -> Result<(), RecoveryModelError> {
    // Validate every endpoint/order before allocating or considering operational ambiguity.
    for range in ranges.clone() {
        validate_range(range)?;
    }
    let event_count = ranges.len().checked_mul(2).ok_or(RecoveryModelError::Capacity)?;
    if event_count.checked_mul(std::mem::size_of::<RectangleEvent>()).is_none_or(|bytes| bytes > isize::MAX as usize) {
        return Err(RecoveryModelError::Capacity);
    }
    let mut events = Vec::new();
    events.try_reserve_exact(event_count).map_err(|_| RecoveryModelError::Capacity)?;
    for range in ranges {
        events.push(RectangleEvent { row: range.start.row, start: true, first: range.start.col, last: range.end.col });
        events.push(RectangleEvent {
            row: range.end.row.checked_add(1).ok_or(RecoveryModelError::Range)?,
            start: false,
            first: range.start.col,
            last: range.end.col,
        });
    }
    // The internal MAX_ROWS sentinel is permitted; end-before-start allows row adjacency.
    events.sort_unstable_by_key(|event| (event.row, event.start));
    let mut occupied = [0u64; 256];
    for event in events {
        let first_word = usize::try_from(event.first / 64).map_err(|_| RecoveryModelError::Capacity)?;
        let last_word = usize::try_from(event.last / 64).map_err(|_| RecoveryModelError::Capacity)?;
        let words = occupied.get_mut(first_word..=last_word).ok_or(RecoveryModelError::Coordinate)?;
        for (offset, word) in words.iter_mut().enumerate() {
            let left = if offset == 0 { u64::MAX << (event.first % 64) } else { u64::MAX };
            let right = if offset == last_word - first_word { u64::MAX >> (63 - event.last % 64) } else { u64::MAX };
            let mask = left & right;
            if event.start {
                if *word & mask != 0 {
                    return Err(RecoveryModelError::UnsupportedRepresentation(RecoveryRepresentation::OverlappingRectangles));
                }
                *word |= mask;
            } else {
                // Every accepted active start is disjoint. Thus this end clears only its own
                // bits, even when another rectangle remains active in the same machine word.
                *word &= !mask;
            }
        }
    }
    Ok(())
}

fn validate_line_size(size: f32) -> Result<(), RecoveryModelError> {
    if !size.is_finite() || size < 0.0 {
        return Err(RecoveryModelError::Geometry);
    }
    Ok(())
}

fn validate_line(line: &LineInfo, style_count: usize) -> Result<(), RecoveryModelError> {
    let LineInfo { size, style, hidden: _, outline: _, collapsed: _, custom: _ } = line;
    // Hidden/zero/default lines and every outline u8 are retained, without editor clamps.
    if let Some(size) = size {
        validate_line_size(*size)?;
    }
    if let Some(style) = style {
        validate_style_id(*style, style_count)?;
    }
    Ok(())
}

fn validate_sheet_structure(sheet: &Sheet, style_count: usize) -> Result<(), RecoveryModelError> {
    let Sheet {
        name: _, // Checked in the workbook's actual sheet namespace.
        cols,
        rows,
        default_col_width,
        default_row_height,
        merges,
        freeze,
        tab_color,
        view_active,
        view_top_left,
        // P1a primitives/storage/spills are orchestrated by the later complete entry point.
        cells: _,
        spill: _,
        spill_ranges: _,
        // B2 regions; B3 objects; B4 print/protection metadata. All retained without work here.
        cond_formats: _,
        validations: _,
        tables: _,
        what_if_tables: _,
        autofilter: _,
        pivots: _,
        cell_pictures: _,
        comments: _,
        hyperlinks: _,
        charts: _,
        chart_groups: _,
        images: _,
        shapes: _,
        sparklines: _,
        slicers: _,
        controls: _,
        protection: _,
        edit_ranges: _,
        print: _,
        // Inert typed display settings, including raw zoom u16 and all visibility alternatives.
        visibility: _,
        show_gridlines: _,
        show_headings: _,
        show_formulas: _,
        show_zeros: _,
        show_outline_symbols: _,
        summary_below: _,
        summary_right: _,
        right_to_left: _,
        zoom: _,
        view: _,
        // Runtime identity/capture belongs to P2, package graph/sidecars to P3 (0 file_id valid).
        uid: _,
        file_id: _,
        preserved: _,
    } = sheet;
    validate_line_size(*default_col_width)?;
    validate_line_size(*default_row_height)?;
    for (lines, max) in [(rows, MAX_ROWS), (cols, MAX_COLS)] {
        for (&key, line) in lines {
            if key >= max {
                return Err(RecoveryModelError::Coordinate);
            }
            validate_line(line, style_count)?;
        }
    }
    // At most MAX_ROWS/MAX_COLS finite f32 sizes accumulate in f64: no f32 extent ceiling.
    // Never construct LineIndex/layout or expand sparse line metadata to validate it.
    if freeze.is_some_and(|(rows, cols)| rows > MAX_ROWS || cols > MAX_COLS) {
        return Err(RecoveryModelError::Coordinate);
    }
    validate_cell_ref(*view_active)?;
    validate_cell_ref(*view_top_left)?;
    if let Some(color) = tab_color {
        validate_color(*color)?;
    }
    validate_disjoint_rectangles(merges.iter().copied())
}

/// Checks only the structural envelope; returns the real validated StyleTable length for
/// later orchestration. This is deliberately neither a complete model gate nor public API.
pub(super) fn validate_workbook_structure(workbook: &Workbook) -> Result<usize, RecoveryModelError> {
    let Workbook {
        sheets,
        names,
        styles,
        theme,
        calc,
        active_sheet,
        view,
        cell_styles,
        // Inert content: all flags/strings/lists are retained; no editor or save normalization.
        date_system: _,
        show_objects: _,
        remove_personal_info: _,
        props: _,
        protected_structure: _,
        custom_lists: _,
        csv_delimiter: _,
        // B4 owns typed protection/phonetics. Engine owns macro/JSON/executable and byte budgets.
        structure_password: _,
        phonetics: _,
        macros: _,
        macros_read: _,
        macros_unread: _,
        vba_project: _,
        // P2 native-path metadata/capture; P3 opaque package graph/sidecars.
        preserved: _,
        source_dir: _,
        source_path: _,
    } = workbook;
    let active = active_sheet.get(); // Sample the exceptional atomic once; never update it.
    if sheets.is_empty() || active >= sheets.len() {
        return Err(RecoveryModelError::Workbook);
    }
    let CalcSettings { max_change, mode: _, iterative: _, max_iterations: _, precision_as_displayed: _, r1c1: _ } = calc;
    if !max_change.is_finite() {
        return Err(RecoveryModelError::Value);
    }
    // Iteration work is capped by calc's consumer; finite negative change and 0/large counts stay.
    let WorkbookView { show_sheet_tabs: _, show_horizontal_scroll: _, show_vertical_scroll: _, tab_ratio: _ } = view;
    let Theme {
        colors,
        name: _,
        major_font: _,
        minor_font: _,
        major_font_ea: _,
        minor_font_ea: _,
        major_font_cs: _,
        minor_font_cs: _,
        major_font_scripts: _,
        minor_font_scripts: _, // Ordered strings, not identity namespaces.
    } = theme;
    for &rgb in colors {
        validate_color(crate::Color::Rgb(rgb))?;
    }
    let style_count = validate_style_table(styles)?;
    for (_, style) in cell_styles {
        // P1a checks finite-only font size, colors and rotation; other style fields are inert.
        let crate::Style { font: _, fill: _, borders: _, align: _, num_fmt: _, protection: _, dxf_stated: _ } = style;
        validate_style(style)?;
    }
    for name in names {
        let DefinedName { scope, name: _, formula: _, comment: _, hidden: _ } = name;
        if scope.is_some_and(|scope| scope >= sheets.len()) {
            return Err(RecoveryModelError::Workbook);
        }
    }
    validate_ascii_identities(sheets.iter().map(|sheet| (None, sheet.name.as_str())))?;
    validate_ascii_identities(names.iter().map(|name| (name.scope, name.name.as_str())))?;
    validate_ascii_identities(cell_styles.iter().map(|(name, _)| (None, name.as_str())))?;
    for sheet in sheets {
        validate_sheet_structure(sheet, style_count)?;
    }
    Ok(style_count)
}

#[cfg(test)]
#[path = "structure_tests.rs"]
mod tests;
