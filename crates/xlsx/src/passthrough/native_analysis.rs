//! Keeps the imported pivot and native control graph together, or refuses a changed graph.

use std::collections::{HashMap, HashSet};
use std::fmt::{self, Write as _};
use std::sync::Arc;

use gridcraft_core::{MAX_COLS, RangeRef};
use gridcraft_model::{Preserved, PreservedAnalysisOwner, PreservedNativeAnalysis, Sheet, Workbook};
use sha2::{Digest, Sha256};

use crate::pivot::{CT_PIVOT_CACHE_DEF, CT_PIVOT_TABLE};
use crate::read::Ctx;
use crate::write::{Out, Rels};
use crate::{IoError, xml};

use super::native_analysis_read::{CapturedAnalysis, native_part_path};
use super::native_styles::NativeStyles;
use super::write::{NameMap, attach, part_rels_xml, remap};
use super::{KeptReader, kind, normalize, rels_part_name};

const MAX_GUARD_CELLS: u64 = 100_000;
const MAX_GUARD_BYTES: usize = 16 * 1024 * 1024;
const MAX_GRAPH_BYTES: usize = 32 * 1024 * 1024;
const MAX_PIVOTS: usize = 256;

fn invalid(message: &str) -> IoError {
    IoError::Format(format!("native PivotTable and control preservation: {message}"))
}

struct GuardHash {
    digest: Sha256,
    bytes_left: usize,
}

impl fmt::Write for GuardHash {
    fn write_str(&mut self, text: &str) -> fmt::Result {
        self.bytes_left = self.bytes_left.checked_sub(text.len()).ok_or(fmt::Error)?;
        self.digest.update(text.as_bytes());
        Ok(())
    }
}

fn hash_model(wb: &Workbook, graph: &PreservedNativeAnalysis) -> Result<[u8; 32], IoError> {
    let fail = |_: fmt::Error| IoError::TooLarge("native PivotTable identity comparison exceeds its budget".into());
    if wb.sheets.len() > MAX_PIVOTS || graph.sheets.len() != wb.sheets.len() {
        return Err(invalid("worksheet identities changed or exceed the supported limit"));
    }
    let mut hasher = GuardHash { digest: Sha256::new(), bytes_left: MAX_GUARD_BYTES };
    writeln!(hasher, "{:?} {:?} {:?}", wb.date_system, wb.names, wb.theme).map_err(fail)?;
    let mut pivot_count = 0usize;
    for sheet in &wb.sheets {
        let owner = graph.sheets.iter().find(|owner| owner.sheet_id == sheet.file_id).ok_or_else(|| invalid("a worksheet identity changed"))?;
        let expected = owner.owner.rels.iter().filter(|rel| kind(&rel.rel_type) == "pivotTable").count();
        pivot_count = pivot_count.saturating_add(expected);
        if pivot_count > MAX_PIVOTS || expected != sheet.pivots.len() {
            return Err(invalid("PivotTable ownership changed or is unsupported"));
        }
        writeln!(hasher, "{:?} {:?} {:?}", sheet.file_id, sheet.name, sheet.uid.get()).map_err(fail)?;
        for table in &sheet.tables {
            writeln!(hasher, "{table:?}").map_err(fail)?;
        }
        if !owner.anchors.is_empty() {
            writeln!(hasher, "{:?} {:?}", sheet.default_row_height, sheet.default_col_width).map_err(fail)?;
            for (&row, line) in &sheet.rows {
                if line.hidden || sheet.row_height(row) != sheet.default_row_height {
                    writeln!(hasher, "r{row}:{:?}:{}", sheet.row_height(row), line.hidden).map_err(fail)?;
                }
            }
            for (&col, line) in &sheet.cols {
                if line.hidden || sheet.col_width(col) != sheet.default_col_width {
                    writeln!(hasher, "c{col}:{:?}:{}", sheet.col_width(col), line.hidden).map_err(fail)?;
                }
            }
        }
        for pivot in &sheet.pivots {
            writeln!(hasher, "{pivot:?}").map_err(fail)?;
        }
    }
    let mut styles = HashSet::new();
    for (si, range) in protected_regions(wb)? {
        let sheet = wb.sheet(si).ok_or_else(|| invalid("a worksheet is missing"))?;
        if intersects_merge(sheet, &range) {
            return Err(invalid("a merged range intersects the native PivotTable source or report"));
        }
        writeln!(hasher, "{si}:{range:?}").map_err(fail)?;
        for at in range.iter() {
            if sheet.cell(at).is_some_and(|cell| cell.formula.is_some() || cell.py.is_some()) || sheet.cell_pictures.contains_key(&at) {
                return Err(invalid("the native PivotTable source or report contains live formulas, Python cells or cell pictures"));
            }
            let style = sheet.style_id(at);
            if styles.insert(style) {
                writeln!(hasher, "{style:?} {:?}", wb.styles.get(style)).map_err(fail)?;
            }
            writeln!(hasher, "{style:?} {:?}", sheet.value_ref(at)).map_err(fail)?;
        }
    }
    Ok(hasher.digest.finalize().into())
}

fn protected_regions(wb: &Workbook) -> Result<Vec<(usize, RangeRef)>, IoError> {
    let mut regions = Vec::new();
    let mut cells = MAX_GUARD_CELLS;
    for (si, sheet) in wb.sheets.iter().enumerate() {
        for pivot in &sheet.pivots {
            let report = pivot.last_range.ok_or_else(|| invalid("the PivotTable report has no supported range"))?;
            let mut source = wb
                .sheets
                .iter()
                .enumerate()
                .flat_map(|(i, sheet)| sheet.tables.iter().map(move |table| (i, table)))
                .filter(|(_, table)| table.name.eq_ignore_ascii_case(&pivot.source));
            let (source_sheet, table) = source.next().ok_or_else(|| invalid("the PivotTable source is not a local named table"))?;
            if source.next().is_some() || table.table_type.is_some() || table.connection_id.is_some() {
                return Err(invalid("the PivotTable source is ambiguous or external"));
            }
            for region in [(si, report), (source_sheet, table.range)] {
                if !regions.contains(&region) {
                    cells = cells
                        .checked_sub(region.1.count())
                        .ok_or_else(|| IoError::TooLarge("native PivotTable source and report cells exceed the guard budget".into()))?;
                    regions.push(region);
                }
            }
        }
    }
    Ok(regions)
}

fn intersects_merge(sheet: &Sheet, range: &RangeRef) -> bool {
    sheet.merges.iter().any(|merged| merged.intersects(range))
}

pub(super) fn check_cell_edit(before: &Workbook, candidate: &Workbook, pending_rows: &[(usize, u32)]) -> Result<(), String> {
    let message = "Preserved native PivotTables and controls cannot safely follow this source, report or automatic row-height edit.";
    if before.theme != candidate.theme
        || before.date_system != candidate.date_system
        || before.sheets.iter().zip(&candidate.sheets).any(|(a, b)| !Arc::ptr_eq(a, b) && (a.tables != b.tables || a.pivots != b.pivots))
    {
        return Err(message.into());
    }
    let regions = protected_regions(before).map_err(|error| error.to_string())?;
    for (si, range) in regions {
        let (Some(prior), Some(after)) = (before.sheets.get(si), candidate.sheets.get(si)) else { return Err(message.into()) };
        if !Arc::ptr_eq(prior, after)
            && ((prior.merges != after.merges && intersects_merge(after, &range))
                || range.iter().any(|at| {
                    prior.cell(at) != after.cell(at)
                        || prior.value_ref(at) != after.value_ref(at)
                        || prior.style_id(at) != after.style_id(at)
                        || prior.cell_pictures.get(&at) != after.cell_pictures.get(&at)
                }))
        {
            return Err(message.into());
        }
    }
    let Some(graph) = before.preserved.as_deref().and_then(|p| p.native_analysis.as_ref()) else { return Ok(()) };
    for &(si, row) in pending_rows {
        let Some(sheet) = candidate.sheet(si) else { continue };
        if !sheet.rows.get(&row).is_some_and(|line| line.custom)
            && graph.sheets.iter().any(|owner| owner.sheet_id == sheet.file_id && !owner.anchors.is_empty())
            && (sheet.row_height(row) != sheet.default_row_height
                || !sheet.cells.row(row, 0, MAX_COLS - 1).all(|(_, cell)| {
                    let style = candidate.styles.get(cell.style);
                    style.font.size <= gridcraft_model::DEFAULT_FONT_SIZE && !style.align.wrap
                }))
        {
            return Err(message.into());
        }
    }
    Ok(())
}

fn is_pivot_part(part_type: &str) -> bool {
    matches!(part_type, CT_PIVOT_TABLE | CT_PIVOT_CACHE_DEF)
}

fn source_styles(graph: &PreservedNativeAnalysis) -> Result<NativeStyles, IoError> {
    if graph.parts.is_empty() || graph.parts.len() > super::MAX_KEPT_PARTS || graph.byte_len() > MAX_GRAPH_BYTES {
        return Err(IoError::TooLarge("native PivotTable graph exceeds the retained byte or part budget".into()));
    }
    let mut count = 0;
    for part in graph.parts.iter().filter(|p| is_pivot_part(&p.content_type)) {
        let root = xml::parse(&part.data)?;
        let mut stack = vec![&root];
        while let Some(element) = stack.pop() {
            if let Some(id) = element.attr("dxfId") {
                let next = id.parse::<u32>().ok().and_then(|n| n.checked_add(1)).ok_or_else(|| invalid("a differential style ID is invalid"))?;
                count = count.max(next);
            }
            stack.extend(&element.children);
        }
    }
    NativeStyles::read(&graph.styles, count)
}

fn adopt(wb: &mut Workbook, cx: &mut Ctx<'_>, captured: CapturedAnalysis) -> Result<(), IoError> {
    let CapturedAnalysis { mut graph, drawing_locations, control_locations } = captured;
    let styles = source_styles(&graph)?;
    for sheet in &graph.sheets {
        first_drawing_id(&sheet.anchors, 0)?;
    }
    for part in graph.parts.iter().filter(|p| is_pivot_part(&p.content_type)) {
        // Validate the source references before advertising preservation. Real IDs are assigned
        // by the output style table; this dry pass only needs a representable replacement ID.
        styles.remap_pivot(&part.data, &|_| Some(164))?;
    }
    graph.signature = hash_model(wb, &graph)?;
    let mut kept = wb.preserved.take().unwrap_or_else(|| Arc::new(Preserved::default()));
    let owner = Arc::make_mut(&mut kept);
    if owner.byte_len().saturating_add(graph.byte_len()) > super::MAX_KEPT_TOTAL {
        wb.preserved = Some(kept);
        return Err(IoError::TooLarge("native PivotTable graph exceeds the combined retained byte budget".into()));
    }
    remove_saved_graph_losses(cx, &drawing_locations, &control_locations);
    owner.native_analysis = Some(Arc::new(graph));
    wb.preserved = Some(kept);
    cx.notice("Native PivotTables, slicers and timeline controls are preserved for XLSX save. Refreshing or changing their layout is unsupported.");
    Ok(())
}

fn remove_saved_graph_losses(cx: &mut Ctx<'_>, drawings: &HashSet<String>, controls: &HashSet<String>) {
    let known_warning = |warning: &str| {
        let filter = warning.starts_with("PivotTable \"") && warning.ends_with(": calculated items and label/value filters are not supported");
        let drawing = warning
            .strip_prefix("an embedded object (SmartArt, OLE…) was skipped (")
            .and_then(|rest| rest.strip_suffix(')'))
            .is_some_and(|loc| drawings.contains(loc));
        filter || drawing
    };
    cx.report.warnings.retain(|warning| !known_warning(warning));
    cx.report.issues.retain(|issue| {
        let retained_rel = matches!(issue.feature.as_str(), "native slicer" | "native slicer cache" | "native timeline" | "native timeline cache")
            && issue.location.as_deref().is_some_and(|loc| controls.contains(loc));
        let retained_ext = issue.location.as_deref().is_some_and(|loc| super::native_analysis_read::is_mount_issue(&issue.feature, loc));
        !(retained_rel || retained_ext || known_warning(&issue.description))
    });
}

/// Retained IDs may be used inside a native anchor. Put generated IDs above them.
pub(crate) fn first_drawing_id(anchors: &[String], objects: usize) -> Result<usize, IoError> {
    let mut last_id = 1;
    for anchor in anchors {
        let root = xml::parse(anchor.as_bytes())?;
        let mut stack = vec![&root];
        while let Some(element) = stack.pop() {
            if element.name == "cNvPr" {
                let id = element.attr("id").and_then(|id| id.parse::<u32>().ok()).ok_or_else(|| invalid("a drawing ID is invalid"))?;
                last_id = last_id.max(id);
            }
            stack.extend(&element.children);
        }
    }
    let first = last_id.checked_add(1).ok_or_else(|| invalid("native drawing IDs leave no room for other objects"))? as usize;
    if (last_id as usize).checked_add(objects).is_none_or(|last| last > u32::MAX as usize) {
        return Err(invalid("native drawing IDs leave no room for other objects"));
    }
    Ok(first)
}

pub(crate) struct NativeWrite {
    graph: Arc<PreservedNativeAnalysis>,
    names: NameMap,
    pub styles: NativeStyles,
}

impl NativeWrite {
    pub(crate) fn prepare(wb: &Workbook) -> Result<Option<Self>, IoError> {
        let Some(graph) = wb.preserved.as_ref().and_then(|owner| owner.native_analysis.as_ref()) else { return Ok(None) };
        if hash_model(wb, graph)? != graph.signature {
            return Err(invalid("a source cell, PivotTable report, worksheet identity or control geometry changed. Undo that change before saving."));
        }
        let styles = source_styles(graph)?;
        let mut names = HashMap::new();
        for part in &graph.parts {
            if !native_part_path(&part.name, &part.content_type) || names.insert(normalize(&part.name), part.name.clone()).is_some() {
                return Err(invalid("invalid or duplicate native part identity"));
            }
        }
        Ok(Some(Self { graph: Arc::clone(graph), names, styles }))
    }

    pub(crate) fn reserve(&self, kept: &mut super::KeptOut) -> Result<(), IoError> {
        for part in &self.graph.parts {
            if !kept.reserve(&part.name) || (!part.rels.is_empty() && !kept.reserve(&rels_part_name(&part.name))) {
                return Err(invalid("a native part name collides with another retained part"));
            }
        }
        Ok(())
    }

    fn mount(&self, owner: &PreservedAnalysisOwner, rels: &mut Rels) -> Result<(String, String), IoError> {
        let ids = attach(rels, &owner.rels, &self.names);
        if ids.len() != owner.rels.len() || owner.rels.iter().any(|rel| rel.external) {
            return Err(invalid("a retained native owner has a missing or duplicate relationship"));
        }
        let mut pivot_caches = String::new();
        let mut extensions = String::new();
        for fragment in &owner.elements {
            let target = match fragment.slot.as_str() {
                "pivotCaches" => &mut pivot_caches,
                "ext" => &mut extensions,
                _ => return Err(invalid("unknown retained native XML mount")),
            };
            target.push_str(&remap(&fragment.xml, &ids).ok_or_else(|| invalid("a retained native XML mount cannot be mapped"))?);
        }
        Ok((pivot_caches, extensions))
    }

    pub(crate) fn workbook_mount(&self, rels: &mut Rels) -> Result<(String, String), IoError> {
        self.mount(&self.graph.workbook, rels)
    }

    pub(crate) fn sheet_extensions(&self, sheet_id: u32, rels: &mut Rels) -> Result<String, IoError> {
        let sheet = self.graph.sheets.iter().find(|sheet| sheet.sheet_id == sheet_id).ok_or_else(|| invalid("a native sheet owner is missing"))?;
        let (caches, extensions) = self.mount(&sheet.owner, rels)?;
        if !caches.is_empty() {
            return Err(invalid("a worksheet cannot own a workbook PivotTable cache list"));
        }
        Ok(extensions)
    }

    pub(crate) fn anchors(&self, sheet_id: u32) -> &[String] {
        self.graph.sheets.iter().find(|sheet| sheet.sheet_id == sheet_id).map_or(&[], |sheet| sheet.anchors.as_slice())
    }

    pub(crate) fn emit(&self, out: &mut Out, format: &dyn Fn(&str) -> Option<u32>) -> Result<(), IoError> {
        for part in &self.graph.parts {
            if part.rels.iter().any(|rel| rel.external || !self.names.contains_key(&normalize(&rel.target))) {
                return Err(invalid("native graph contains an external or missing dependent part"));
            }
            let bytes = if is_pivot_part(&part.content_type) { self.styles.remap_pivot(&part.data, format)? } else { part.data.to_vec() };
            out.part(&part.name, Some(&part.content_type), bytes);
            if !part.rels.is_empty() {
                out.part(&rels_part_name(&part.name), None, part_rels_xml(&part.rels, &self.names).into_bytes());
            }
        }
        Ok(())
    }
}

pub(crate) fn import(cx: &mut Ctx<'_>, kept: &KeptReader, workbook_part: &str, book: &xml::El, file_to_model: &[Option<usize>], wb: &mut Workbook) {
    match super::native_analysis_read::capture(cx, kept, workbook_part, book, file_to_model) {
        Ok(Some(graph)) => {
            if let Err(error) = adopt(wb, cx, graph) {
                cx.warn(format!("Native slicers and timeline controls could not be preserved: {error}"));
            }
        }
        Ok(None) => {}
        Err(error) => cx.warn(format!("Native slicers and timeline controls could not be preserved: {error}")),
    }
}
