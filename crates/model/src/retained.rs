//! Allocation estimates for undo retention, not a hard process-RSS ceiling.
//!
//! Owned buffers use capacities. Tree/hash storage includes conservative node/slack overhead;
//! allocator metadata is platform dependent. Shared allocations are identified by address, never
//! by equal content. Keep one context for the live workbook and the entire history pass.

use crate::style::{Color, DxfStated, Font, NumFmt, Style, StyleId, Theme};
use crate::workbook::{DefinedName, DocProps, PhoneticProps, PhoneticRun, PhoneticString, Workbook};
use crate::{
    AutoFilter, Cell, CellPicture, CfRule, CfValueKind, Chart, ColType, Comment, CondFormat, ControlAction, EditRange, FilterCriterion, FormControl,
    Formula, Hyperlink, Image, LineInfo, Macro, MacroInput, NamedInput, OutputRule, PasswordHash, PivotCalcField, PivotField, PivotFilter,
    PivotTable, PivotValue, Preserved, PreservedPart, PreservedRel, PreservedSheet, PreservedXml, PrintSettings, PyCode, PyOutput, Series, Shape,
    ShapeFormat, Sheet, SheetProtection, Slicer, Sparkline, Step, StepKind, Table, TableColumn, Trendline, Validation, XmlColumnPr,
};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::hash::Hash;
use std::mem::{size_of, size_of_val};
use std::sync::Arc;

/// One accounting pass. Seed with the live workbook, then visit snapshots newest first.
/// The returned bytes are allocations not visited earlier in this pass. Drop the context
/// before dropping/mutating any visited workbook: its pointer identities are not persistent.
#[derive(Default)]
pub struct RetainedBytes<'a> {
    seen: HashSet<usize>,
    bytes: usize,
    depth: usize,
    seeding: bool,
    live_bands: Vec<&'a AllocationSummary>,
    last_live_band: usize,
    collecting: Option<AllocationSummary>,
}

impl<'a> RetainedBytes<'a> {
    /// Excludes allocations reachable from the live workbook without rewalking cached bands.
    pub fn seed_workbook(&mut self, workbook: &'a Workbook) {
        self.seeding = true;
        self.workbook(workbook);
        self.seeding = false;
    }

    pub fn workbook(&mut self, workbook: &'a Workbook) -> usize {
        self.bytes = 0;
        if self.seen.insert(std::ptr::from_ref(workbook) as usize) {
            self.add(size_of::<Workbook>().saturating_add(2 * size_of::<usize>()));
            self.visit(workbook);
        }
        self.bytes
    }

    pub(crate) fn add(&mut self, bytes: usize) {
        self.bytes = self.bytes.saturating_add(bytes);
        if let Some(summary) = &mut self.collecting {
            summary.owned = summary.owned.saturating_add(bytes);
        }
    }

    pub(crate) fn visit<T: HeapBytes + ?Sized>(&mut self, value: &'a T) {
        // Model callers can construct deeply nested arrays/JSON/ASTs even without a file reader.
        // An unmeasurably deep value consumes the whole policy budget, without recursing forever.
        if self.depth >= 256 {
            self.add(usize::MAX);
            return;
        }
        self.depth += 1;
        value.heap_bytes(self);
        self.depth -= 1;
    }

    pub(crate) fn against<T: HeapBytes>(older: &'a T, newer: &'a T) -> usize {
        if std::ptr::eq(older, newer) {
            return 0;
        }
        let mut context = Self { seeding: true, ..Default::default() };
        context.visit(newer);
        context.seeding = false;
        context.bytes = 0;
        context.visit(older);
        context.bytes
    }

    fn claim(&mut self, id: usize) -> bool {
        if self.seen.contains(&id) {
            return false;
        }
        if !self.seeding && self.collecting.is_none() {
            if self.live_bands.get(self.last_live_band).is_some_and(|s| s.contains(id)) {
                return false;
            }
            if let Some(index) = self.live_bands.iter().position(|s| s.contains(id)) {
                self.last_live_band = index;
                return false;
            }
        }
        self.seen.insert(id)
    }

    fn shared<T: HeapBytes + ?Sized>(&mut self, value: &'a Arc<T>) {
        let id = Arc::as_ptr(value).cast::<()>() as usize;
        if !self.claim(id) {
            return;
        }
        // In collection mode, separate this allocation's exclusive bytes from its parent's.
        let parent_bytes = self.collecting.as_mut().map(|s| std::mem::take(&mut s.owned));
        self.add(size_of_val(value.as_ref()).saturating_add(2 * size_of::<usize>()));
        self.visit(value.as_ref());
        if let (Some(parent_bytes), Some(summary)) = (parent_bytes, &mut self.collecting) {
            summary.shared.push((id, summary.owned));
            summary.owned = parent_bytes;
        }
    }

    pub(crate) fn band(&mut self, summary: &'a AllocationSummary) {
        if self.seeding {
            self.live_bands.push(summary);
            return;
        }
        self.add(summary.owned);
        self.add(summary.storage_bytes());
        for &(id, bytes) in &summary.shared {
            if self.claim(id) {
                self.add(bytes);
            }
        }
    }
}

/// Flat inventory below one immutable band. No pointer here owns a payload. The inventory
/// cannot escape its band; every pass borrows it while the owning workbook remains alive.
#[derive(Debug, Default)]
pub(crate) struct AllocationSummary {
    owned: usize,
    shared: Vec<(usize, usize)>,
}

impl AllocationSummary {
    pub(crate) fn new<T: HeapBytes>(value: &T) -> Self {
        let mut context = RetainedBytes { collecting: Some(Self::default()), ..Default::default() };
        context.visit(value);
        let mut summary = context.collecting.unwrap_or_default();
        summary.shared.sort_unstable_by_key(|&(id, _)| id);
        summary
    }
    fn contains(&self, id: usize) -> bool {
        // Most absent identities lie outside a band's small allocation interval. This exact
        // rejection keeps replacing a formula cheap even with many unrelated live bands.
        let (Some(&(first, _)), Some(&(last, _))) = (self.shared.first(), self.shared.last()) else {
            return false;
        };
        id >= first && id <= last && self.shared.binary_search_by_key(&id, |&(key, _)| key).is_ok()
    }
    pub(crate) fn storage_bytes(&self) -> usize {
        self.shared.capacity().saturating_mul(size_of::<(usize, usize)>())
    }
}

/// Private derived state: cloning a mutable band never clones an inventory, and populating
/// one never changes workbook equality or JSON. Dropping the band frees its inventory.
#[derive(Debug, Default)]
pub(crate) struct BandAccounting(pub(crate) std::sync::OnceLock<AllocationSummary>);
impl Clone for BandAccounting {
    fn clone(&self) -> Self {
        Self::default()
    }
}
impl PartialEq for BandAccounting {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}

pub(crate) trait HeapBytes {
    const HAS_HEAP: bool = true;
    /// Only allocations below this value: the containing allocation pays for its inline size.
    fn heap_bytes<'a>(&'a self, context: &mut RetainedBytes<'a>);
}

impl HeapBytes for String {
    fn heap_bytes<'a>(&'a self, c: &mut RetainedBytes<'a>) {
        c.add(self.capacity());
    }
}
impl HeapBytes for str {
    fn heap_bytes<'a>(&'a self, _: &mut RetainedBytes<'a>) {}
}
impl<T: HeapBytes> HeapBytes for Vec<T> {
    fn heap_bytes<'a>(&'a self, c: &mut RetainedBytes<'a>) {
        c.add(self.capacity().saturating_mul(size_of::<T>()));
        if T::HAS_HEAP {
            for value in self {
                c.visit(value);
            }
        }
    }
}
impl<T: HeapBytes> HeapBytes for [T] {
    fn heap_bytes<'a>(&'a self, c: &mut RetainedBytes<'a>) {
        if T::HAS_HEAP {
            for value in self {
                c.visit(value);
            }
        }
    }
}
impl<T: HeapBytes> HeapBytes for Option<T> {
    fn heap_bytes<'a>(&'a self, c: &mut RetainedBytes<'a>) {
        if let Some(value) = self {
            c.visit(value);
        }
    }
}
impl<T: HeapBytes + ?Sized> HeapBytes for Arc<T> {
    fn heap_bytes<'a>(&'a self, c: &mut RetainedBytes<'a>) {
        c.shared(self);
    }
}
impl<T: HeapBytes> HeapBytes for Box<T> {
    fn heap_bytes<'a>(&'a self, c: &mut RetainedBytes<'a>) {
        c.add(size_of::<T>());
        c.visit(self.as_ref());
    }
}
// Rust's BTreeMap nodes hold up to 11 entries and non-root nodes at least 5.
// Charge full internal-node storage (including pointers) even for leaves, including one
// node for an empty map whose last removal may have left an allocated root behind.
fn tree_bytes<K, V>(len: usize) -> usize {
    let nodes = len.div_ceil(5).max(1);
    nodes.saturating_mul(size_of::<(K, V)>().saturating_mul(11).saturating_add(16 * size_of::<usize>()))
}

impl<K: HeapBytes, V: HeapBytes> HeapBytes for BTreeMap<K, V> {
    fn heap_bytes<'a>(&'a self, c: &mut RetainedBytes<'a>) {
        c.add(tree_bytes::<K, V>(self.len()));
        if K::HAS_HEAP || V::HAS_HEAP {
            for (key, value) in self {
                c.visit(key);
                c.visit(value);
            }
        }
    }
}
impl<K: HeapBytes + Eq + Hash, V: HeapBytes> HeapBytes for HashMap<K, V> {
    fn heap_bytes<'a>(&'a self, c: &mut RetainedBytes<'a>) {
        c.add(self.capacity().saturating_mul(size_of::<(K, V)>().saturating_add(1)).saturating_mul(2));
        for (key, value) in self {
            c.visit(key);
            c.visit(value);
        }
    }
}
impl<A: HeapBytes, B: HeapBytes> HeapBytes for (A, B) {
    fn heap_bytes<'a>(&'a self, c: &mut RetainedBytes<'a>) {
        c.visit(&self.0);
        c.visit(&self.1);
    }
}
impl HeapBytes for std::path::PathBuf {
    fn heap_bytes<'a>(&'a self, c: &mut RetainedBytes<'a>) {
        c.add(self.capacity());
    }
}
macro_rules! inline {
    ($($ty:ty),* $(,)?) => { $(impl HeapBytes for $ty { const HAS_HEAP: bool = false; fn heap_bytes<'a>(&'a self, _: &mut RetainedBytes<'a>) {} })* };
}
inline!(
    u8,
    u32,
    usize,
    gridcraft_core::CellRef,
    gridcraft_core::RangeRef,
    LineInfo,
    Color,
    StyleId,
    ColType,
    gridcraft_formula::StructItem,
    gridcraft_math::PathCmd
);

// Exhaustive destructuring makes new fields a compile-time reminder to decide their ownership.
macro_rules! fields {
    ($ty:path, [$($heap:ident),*], [$($inline:ident),*]) => {
        impl HeapBytes for $ty {
            fn heap_bytes<'a>(&'a self, c: &mut RetainedBytes<'a>) {
                let Self { $($heap,)* $($inline: _,)* } = self;
                $(c.visit($heap);)*
            }
        }
    };
}
fields!(
    Sheet,
    [
        name,
        cells,
        cell_pictures,
        spill,
        spill_ranges,
        cols,
        rows,
        merges,
        cond_formats,
        validations,
        tables,
        what_if_tables,
        autofilter,
        comments,
        hyperlinks,
        charts,
        chart_groups,
        images,
        shapes,
        sparklines,
        slicers,
        controls,
        pivots,
        protection,
        edit_ranges,
        print,
        preserved
    ],
    [
        uid,
        file_id,
        default_col_width,
        default_row_height,
        freeze,
        tab_color,
        visibility,
        show_gridlines,
        show_headings,
        show_formulas,
        show_zeros,
        show_outline_symbols,
        summary_below,
        summary_right,
        right_to_left,
        zoom,
        view_active,
        view_top_left,
        view
    ]
);
fields!(crate::WhatIfTable, [formula_text], [anchor, range, input1, input2, two_variable, row_oriented, input1_deleted, input2_deleted, calculate]);
fields!(
    Workbook,
    [
        sheets,
        names,
        styles,
        theme,
        props,
        structure_password,
        custom_lists,
        cell_styles,
        phonetics,
        vba_project,
        macros,
        macros_read,
        macros_unread,
        preserved,
        source_dir,
        source_path
    ],
    [date_system, calc, show_objects, remove_personal_info, active_sheet, view, protected_structure, csv_delimiter]
);
fields!(Font, [name], [size, bold, italic, underline, strike, color, vert, charset, family, minor]);
fields!(Style, [font, num_fmt, dxf_stated], [fill, borders, align, protection]);
fields!(DxfStated, [scheme], [bold, italic, strike, underline, size, name, protection]);
fields!(
    Theme,
    [name, major_font, minor_font, major_font_ea, minor_font_ea, major_font_cs, minor_font_cs, major_font_scripts, minor_font_scripts],
    [colors]
);
fields!(DefinedName, [name, formula, comment], [scope, hidden]);
fields!(DocProps, [title, subject, author, company, keywords, description], []);
fields!(PhoneticRun, [text], [start, end]);
fields!(PhoneticProps, [kind, alignment], [font_id]);
fields!(PhoneticString, [text, runs, props], []);
fields!(TableColumn, [name, totals_label, formula, unique_name, xml], [totals]);
fields!(XmlColumnPr, [xpath, data_type], [map_id, denormalized]);
fields!(
    Table,
    [name, columns, style, filter, table_type],
    [id, range, header_row, totals_row, banded_rows, banded_cols, first_col, last_col, filter_button, connection_id]
);
fields!(CondFormat, [ranges, rule], [priority, stop_if_true]);
fields!(
    Validation,
    [ranges, f1, f2, input_title, input_message, error_title, error_message],
    [kind, op, allow_blank, in_cell_dropdown, show_input, error_style, show_error]
);
fields!(Comment, [author, text, replies, dates], [threaded, resolved, visible, note_box]);
fields!(Hyperlink, [target, tooltip], [external]);
fields!(Series, [name, categories, values, bubble_sizes, trendlines], [color, secondary, kind, smooth, quartiles]);
fields!(Trendline, [name], [kind, forward, backward, intercept, display_equation, display_r_squared, color, width, dash]);
fields!(
    Chart,
    [title, series, x_title, y_title, source, pivot],
    [id, kind, anchor, legend, data_labels, data_table, gridlines, style, hole_size, first_slice_angle, by_rows]
);
fields!(crate::ChartGroup, [name, chart_ids], []);
fields!(Image, [data, mime, alt], [id, anchor]);
fields!(Shape, [text, format, math], [id, kind, anchor, fill, line]);
fields!(ShapeFormat, [font], [no_line, font_size, bold, italic, text_color, text_outline, align, anchor, rtl]);
fields!(Sparkline, [source], [cell, kind, color, markers]);
fields!(Slicer, [table, column, caption, selected], [id, anchor, columns]);
fields!(FormControl, [name, text, link, action], [id, kind, anchor, value, min, max, step, page, horizontal, print, locked]);
fields!(AutoFilter, [criteria], [range]);
fields!(PivotField, [source_col, collapsed_items], [sort, date_group]);
fields!(PivotValue, [source_col, name, number_format], [func, show_as]);
fields!(PivotFilter, [source_col, selected], []);
fields!(
    PivotTable,
    [name, source, rows, columns, values, filters, style, calculated_fields],
    [
        id,
        anchor,
        layout,
        enable_show_details,
        grand_totals_rows,
        grand_totals_cols,
        subtotals,
        subtotals_top,
        show_headers,
        last_range,
        values_on_rows,
        values_position
    ]
);
fields!(PivotCalcField, [name, formula], []);
fields!(
    PrintSettings,
    [paper, header, footer, row_breaks, col_breaks, first_header, first_footer, even_header, even_footer],
    [
        orientation,
        margins,
        print_area,
        title_rows,
        title_cols,
        scale,
        fit_to,
        gridlines,
        headings,
        center_h,
        center_v,
        different_first,
        different_odd_even,
        hf_scale_with_doc,
        hf_align_with_margins,
        page_order
    ]
);
fields!(
    SheetProtection,
    [password],
    [
        legacy_password_hash,
        select_locked,
        select_unlocked,
        format_cells,
        format_columns,
        format_rows,
        insert_columns,
        insert_rows,
        delete_columns,
        delete_rows,
        sort,
        autofilter
    ]
);
fields!(EditRange, [title, ranges, password, security_descriptor], []);
fields!(PreservedRel, [id, rel_type, target], [external]);
fields!(PreservedPart, [name, content_type, data, rels], []);
fields!(PreservedXml, [slot, xml], []);
fields!(PreservedSheet, [name, rel_id], [visibility, before, sheet_id]);
fields!(Preserved, [package_rels, rels, parts, elements, vml, sheets], []);
fields!(Cell, [value, formula, py], [style]);
fields!(Formula, [text, expr], [array, dynamic, eval_mode]);
fields!(CellPicture, [data, mime, alt], []);
fields!(PyCode, [code, output], [object, stale]);
fields!(PyOutput, [value, type_name, preview], [convertible]);
fields!(Macro, [name, description, shortcut, inputs, steps, output, permissions], [v]);
fields!(NamedInput, [name, col_types, default_ref], [kind, rows, cols, header, non_empty]);
fields!(Step, [kind, note], [enabled, breakpoint]);
fields!(OutputRule, [anchor], [on_conflict]);

impl HeapBytes for NumFmt {
    fn heap_bytes<'a>(&'a self, c: &mut RetainedBytes<'a>) {
        c.visit(&self.0);
    }
}
impl HeapBytes for gridcraft_core::Value {
    fn heap_bytes<'a>(&'a self, c: &mut RetainedBytes<'a>) {
        match self {
            Self::Text(s) => c.visit(s),
            Self::Array(a) => c.visit(a),
            Self::Empty | Self::Number(_) | Self::Bool(_) | Self::Error(_) => {}
        }
    }
}
impl HeapBytes for gridcraft_core::Array {
    fn heap_bytes<'a>(&'a self, c: &mut RetainedBytes<'a>) {
        c.visit(&self.data);
    }
}
impl HeapBytes for gridcraft_formula::Expr {
    fn heap_bytes<'a>(&'a self, c: &mut RetainedBytes<'a>) {
        use gridcraft_formula::SheetSel;
        // Valid flat formulas can form thousands-deep operator trees. Use the model's
        // iterative walker, charging containers here and their children when visited.
        self.walk(&mut |expr| match expr {
            Self::Text(s) => c.visit(s),
            Self::Array(rows) => {
                c.add(rows.capacity().saturating_mul(size_of::<Vec<Self>>()));
                for row in rows {
                    c.add(row.capacity().saturating_mul(size_of::<Self>()));
                }
            }
            Self::Name(s) => c.visit(s),
            Self::Ref(r) => match &r.sheet {
                SheetSel::Current => {}
                SheetSel::Named(s) => c.visit(s),
                SheetSel::Span(a, b) => {
                    c.visit(a);
                    c.visit(b);
                }
            },
            Self::Struct(s) => {
                c.visit(&s.table);
                c.visit(&s.specifiers);
                c.visit(&s.col_start);
                c.visit(&s.col_end);
            }
            Self::Unary(_, _) | Self::Paren(_) => c.add(size_of::<Self>()),
            Self::Binary(_, _, _) => c.add(size_of::<Self>().saturating_mul(2)),
            Self::Call(name, args) => {
                c.visit(name);
                c.add(args.capacity().saturating_mul(size_of::<Self>()));
            }
            Self::Invoke(_, args) => {
                c.add(size_of::<Self>());
                c.add(args.capacity().saturating_mul(size_of::<Self>()));
            }
            Self::Number(_) | Self::Bool(_) | Self::Error(_) | Self::Missing => {}
        });
    }
}
impl HeapBytes for serde_json::Value {
    fn heap_bytes<'a>(&'a self, c: &mut RetainedBytes<'a>) {
        match self {
            Self::String(s) => c.visit(s),
            Self::Array(a) => c.visit(a),
            Self::Object(m) => {
                c.add(tree_bytes::<String, Self>(m.len()));
                for (k, v) in m {
                    c.visit(k);
                    c.visit(v);
                }
            }
            Self::Null | Self::Bool(_) | Self::Number(_) => {}
        }
    }
}
impl HeapBytes for PasswordHash {
    fn heap_bytes<'a>(&'a self, c: &mut RetainedBytes<'a>) {
        match self {
            Self::Legacy(_) => {}
            Self::Iterated { algorithm, hash, salt, spin_count: _ } => {
                c.visit(algorithm);
                c.visit(hash);
                c.visit(salt);
            }
        }
    }
}
impl HeapBytes for CfValueKind {
    fn heap_bytes<'a>(&'a self, c: &mut RetainedBytes<'a>) {
        match self {
            Self::Formula(s) => c.visit(s),
            Self::Min | Self::Max | Self::Number(_) | Self::Percent(_) | Self::Percentile(_) => {}
        }
    }
}
impl HeapBytes for CfRule {
    fn heap_bytes<'a>(&'a self, c: &mut RetainedBytes<'a>) {
        match self {
            Self::CellIs { op: _, a, b, style } => {
                c.visit(a);
                c.visit(b);
                c.visit(style);
            }
            Self::Expression { formula, style } => {
                c.visit(formula);
                c.visit(style);
            }
            Self::ContainsText { text, style }
            | Self::NotContainsText { text, style }
            | Self::BeginsWith { text, style }
            | Self::EndsWith { text, style } => {
                c.visit(text);
                c.visit(style);
            }
            Self::Blanks { style }
            | Self::NoBlanks { style }
            | Self::Errors { style }
            | Self::NoErrors { style }
            | Self::Duplicate { style }
            | Self::Unique { style }
            | Self::Top10 { style, bottom: _, percent: _, rank: _ }
            | Self::AboveAverage { style, below: _, equal: _, std_dev: _ } => c.visit(style),
            Self::TimePeriod { period, style } => {
                c.visit(period);
                c.visit(style);
            }
            Self::ColorScale { stops } => c.visit(stops),
            Self::DataBar { min, max, color: _, gradient: _, show_value: _ } => {
                c.visit(min);
                c.visit(max);
            }
            Self::IconSet { set, thresholds, reverse: _, show_value: _ } => {
                c.visit(set);
                c.visit(thresholds);
            }
        }
    }
}
impl HeapBytes for FilterCriterion {
    fn heap_bytes<'a>(&'a self, c: &mut RetainedBytes<'a>) {
        match self {
            Self::Values { values, blanks: _ } => c.visit(values),
            Self::Custom { a, b, and: _ } => {
                c.visit(a);
                c.visit(b);
            }
            Self::Icon { set, id: _ } => c.visit(set),
            Self::Top10 { bottom: _, percent: _, count: _ } | Self::AboveAverage(_) | Self::FillColor(_) | Self::FontColor(_) => {}
        }
    }
}
impl HeapBytes for ControlAction {
    fn heap_bytes<'a>(&'a self, c: &mut RetainedBytes<'a>) {
        match self {
            Self::Macro { name, input, output } => {
                c.visit(name);
                c.visit(input);
                c.visit(output);
            }
            Self::Command { id, params } => {
                c.visit(id);
                c.visit(params);
            }
        }
    }
}
impl HeapBytes for MacroInput {
    fn heap_bytes<'a>(&'a self, c: &mut RetainedBytes<'a>) {
        match self {
            Self::Selection => {}
            Self::Fixed(s) => c.visit(s),
        }
    }
}
impl HeapBytes for StepKind {
    fn heap_bytes<'a>(&'a self, c: &mut RetainedBytes<'a>) {
        match self {
            Self::Command { id, params } => {
                c.visit(id);
                c.visit(params);
            }
            Self::Repeat { over, of: _, max: _, body } => {
                c.visit(over);
                c.visit(body);
            }
            Self::If { cell, op: _, value, then, otherwise } => {
                c.visit(cell);
                c.visit(value);
                c.visit(then);
                c.visit(otherwise);
            }
            Self::Stop { message, rollback: _ } => c.visit(message),
        }
    }
}
impl HeapBytes for gridcraft_math::MathList {
    fn heap_bytes<'a>(&'a self, c: &mut RetainedBytes<'a>) {
        c.visit(&self.items);
    }
}
impl HeapBytes for gridcraft_math::Item {
    fn heap_bytes<'a>(&'a self, c: &mut RetainedBytes<'a>) {
        match self {
            Self::Glyph(_) | Self::Rule(_) => {}
            Self::Path(p) => c.visit(&p.commands),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Anchor, StyleTable};
    use gridcraft_core::{Array, CellRef, Value};

    #[test]
    fn distinct_equal_notes_are_charged_but_live_shared_notes_are_not() {
        let mut old = Workbook::new();
        let comment = Comment {
            author: String::new(),
            text: "n".repeat(64 * 1024),
            replies: vec![],
            threaded: false,
            resolved: false,
            visible: false,
            dates: vec![],
            note_box: None,
        };
        old.sheet_mut(0).unwrap().comments_mut().insert(CellRef::new(0, 0), comment);
        let mut current = old.clone();
        let mut shared = RetainedBytes::default();
        shared.seed_workbook(&current);
        assert!(shared.workbook(&old) < 64 * 1024);
        current.sheet_mut(0).unwrap().comments_mut();
        let mut separate = RetainedBytes::default();
        separate.seed_workbook(&current);
        assert!(separate.workbook(&old) >= 64 * 1024);
        assert_eq!(old, current);
    }

    #[test]
    fn nested_arrays_and_cross_sheet_images_deduplicate_by_allocation() {
        let text: Arc<str> = "s".repeat(64 * 1024).into();
        let image = Arc::new(vec![0; 128 * 1024]);
        let nested = Value::Array(Arc::new(Array::row(vec![Value::Array(Arc::new(Array::row(vec![Value::Text(text.clone())])))])));
        let mut old = Workbook::new();
        old.sheet_mut(0).unwrap().set_value(CellRef::new(0, 0), nested);
        old.sheet_mut(0).unwrap().images.push(Image {
            id: 1,
            anchor: Anchor::default(),
            data: image.clone(),
            mime: String::new(),
            alt: String::new(),
        });
        let mut other = Sheet::new("Other");
        other.set_value(CellRef::new(0, 0), Value::Text(text));
        other.images.push(Image { id: 2, anchor: Anchor::default(), data: image, mime: String::new(), alt: String::new() });
        old.sheets.push(Arc::new(other));
        let current = Workbook::new();
        let mut context = RetainedBytes::default();
        context.seed_workbook(&current);
        let bytes = context.workbook(&old);
        assert!(bytes >= 192 * 1024, "nested payloads omitted: {bytes}");
        assert!(bytes < 220 * 1024, "shared payloads counted twice: {bytes}");
        assert_eq!(context.workbook(&old), 0);
    }

    #[test]
    fn unused_owned_capacity_and_style_index_are_charged() {
        let mut old = Sheet::new("Old");
        old.print.header = String::with_capacity(64 * 1024);
        old.merges = Vec::with_capacity(4096);
        assert!(old.retained_bytes_against(&Sheet::new("New")) >= 64 * 1024 + 4096 * size_of::<gridcraft_core::RangeRef>());
        let mut styles = StyleTable::default();
        styles.intern(Style { font: Font { name: "x".repeat(32 * 1024), ..Default::default() }, ..Default::default() });
        // The index owns another Font.name, even though the Style values compare equal.
        assert!(styles.retained_bytes_against(&StyleTable::default()) >= 64 * 1024);
    }

    #[test]
    fn what_if_definition_capacity_is_charged() {
        let mut old = Sheet::new("Old");
        old.what_if_tables = Vec::with_capacity(4096);
        old.what_if_tables.push(crate::WhatIfTable {
            anchor: CellRef::new(1, 1),
            range: gridcraft_core::RangeRef::cell(CellRef::new(1, 1)),
            formula_text: String::with_capacity(64 * 1024),
            input1: None,
            input2: None,
            two_variable: None,
            row_oriented: None,
            input1_deleted: None,
            input2_deleted: None,
            calculate: None,
        });
        assert!(old.retained_bytes_against(&Sheet::new("New")) >= 4096 * size_of::<crate::WhatIfTable>() + 64 * 1024);
    }

    #[test]
    fn arithmetic_saturates_and_hostile_nesting_consumes_the_budget() {
        let mut context = RetainedBytes::default();
        context.add(usize::MAX);
        context.add(1);
        assert_eq!(context.bytes, usize::MAX);
        let mut value = serde_json::Value::Null;
        for _ in 0..300 {
            value = serde_json::Value::Array(vec![value]);
        }
        let mut context = RetainedBytes::default();
        context.visit(&value);
        assert_eq!(context.bytes, usize::MAX);
    }
}

#[cfg(test)]
mod band_inventory_tests {
    use super::*;
    use gridcraft_core::{CellRef, Value};

    #[test]
    fn a_shared_payload_in_another_live_band_or_sheet_is_excluded() {
        let text: Arc<str> = "x".repeat(128 * 1024).into();
        let mut old = Workbook::new();
        old.sheet_mut(0).unwrap().set_value(CellRef::new(0, 0), Value::Text(text.clone()));
        let mut current = Workbook::new();
        current.sheet_mut(0).unwrap().set_value(CellRef::new(1000, 0), Value::Text(text.clone()));
        let mut other = Sheet::new("Other");
        other.set_value(CellRef::new(2000, 0), Value::Text(text.clone()));
        current.sheets.push(Arc::new(other));
        let mut context = RetainedBytes::default();
        context.seed_workbook(&current);
        assert!(context.workbook(&old) < 128 * 1024);
        // A feature payload can also be shared with a cell inventory, in either direction.
        let mut feature = Workbook::new();
        feature.sheet_mut(0).unwrap().spill.insert(CellRef::new(0, 0), Value::Text(text));
        assert!(context.workbook(&feature) < 128 * 1024);
        let mut reverse = RetainedBytes::default();
        reverse.seed_workbook(&feature);
        assert!(reverse.workbook(&old) < 128 * 1024);
    }
}
