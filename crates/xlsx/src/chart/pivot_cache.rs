//! The cached values of PivotCharts whose series references point at a PivotTable corner.

use std::collections::HashMap;

use gridcraft_core::{RangeRef, Value};
use gridcraft_formula::{Expr, MAX_FORMULA_LEN, RefKind, Reference, SheetSel};
use gridcraft_model::{CompatibilityEffect, CompatibilityIssue, Editability, Series, Sheet, Workbook};

use crate::ReadReport;
use crate::xml::{El, num};

// Charge fallback literals, comparison points and pending entries across the entire import. Sparse XML can
// otherwise expand into large arrays, especially when one chart part is referenced repeatedly.
const MAX_WORKBOOK_CACHE_BYTES: usize = 8 * 1024 * 1024;

pub(crate) struct PivotChartImport {
    pending: HashMap<u32, Vec<Pending>>,
    remaining: usize,
    dropped: bool,
    unverified: bool,
}

impl Default for PivotChartImport {
    fn default() -> Self {
        Self { pending: HashMap::new(), remaining: MAX_WORKBOOK_CACHE_BYTES, dropped: false, unverified: false }
    }
}

#[derive(Clone, Copy)]
enum Kind {
    Numbers,
    Categories,
}

struct Pending {
    series: usize,
    kind: Kind,
    data: CacheData,
}

enum CacheData {
    Literal(String),
    Compare(Vec<Value>),
}

impl Kind {
    fn reference(self, series: &Series) -> Option<&str> {
        match self {
            Self::Numbers => Some(&series.values),
            Self::Categories => series.categories.as_deref(),
        }
    }

    fn replace(self, series: &mut Series, literal: String) {
        match self {
            Self::Numbers => series.values = literal,
            Self::Categories => series.categories = Some(literal),
        }
    }
}

impl PivotChartImport {
    pub(super) fn defer(&mut self, id: u32, index: usize, source: &El, series: &Series) {
        let caches = [(Kind::Numbers, source.path(&["val", "numRef", "numCache"])), (Kind::Categories, source.path(&["cat", "strRef", "strCache"]))];
        for (kind, cache) in caches {
            let Some(cache) = cache else { continue };
            let Some(reference) = kind.reference(series).and_then(direct_reference) else { continue };
            let Ok(layout) = PointLayout::read(cache, kind) else {
                self.dropped = true;
                continue;
            };
            if layout.count == 0 {
                continue;
            }
            // Include amortized storage for the pending index and vector capacity.
            let overhead = 2 * (size_of::<Pending>() + size_of::<(u32, Vec<Pending>)>());
            let range_count = reference.range().count();
            let data = if let Some(literal) = self.bounded_literal(cache, kind, layout.count, overhead) {
                CacheData::Literal(literal)
            } else if layout.declared.map(u64::from) != Some(range_count) || range_count > super::MAX_CACHE {
                // These caches cannot be reproduced by the live range writer.
                self.dropped = true;
                continue;
            } else if let Some(points) = self.comparison_points(cache, kind, layout.count as usize, overhead) {
                CacheData::Compare(points)
            } else {
                self.unverified = true;
                continue;
            };
            self.pending.entry(id).or_default().push(Pending { series: index, kind, data });
        }
    }

    pub(super) fn read_literal(&mut self, source: &El, numeric: bool) -> Option<String> {
        let kind = if numeric { Kind::Numbers } else { Kind::Categories };
        let Ok(layout) = PointLayout::read(source, kind) else {
            self.dropped = true;
            return None;
        };
        if !layout.dense {
            if let Some(literal) = self.bounded_literal(source, kind, layout.count, 0) {
                return Some(literal);
            }
            self.dropped = true;
        }
        // Dense literals already had a lossless reader. Oversized sparse literals fall back
        // to its document order, retaining the existing values without expanding the gaps.
        Some(super::literal_in_document_order(source, numeric))
    }

    fn bounded_literal(&mut self, source: &El, kind: Kind, count: u32, overhead: usize) -> Option<String> {
        let limit = MAX_FORMULA_LEN.min(self.remaining.saturating_sub(overhead));
        let literal = indexed_literal(source, kind, count as usize, limit)?;
        self.remaining = self.remaining.checked_sub(literal.capacity() + overhead)?;
        Some(literal)
    }

    // A same-length cache can still differ from its cells. Hold values for comparison when the
    // formula limit prevents using a literal, without expanding beyond the writer's point cap.
    fn comparison_points(&mut self, source: &El, kind: Kind, count: usize, overhead: usize) -> Option<Vec<Value>> {
        let mut remaining = self.remaining.checked_sub(overhead)?.checked_sub(count.checked_mul(size_of::<Value>())?)?;
        let mut points = vec![Value::Empty; count];
        for (order, point) in source.kids("pt").enumerate() {
            let Some(value) = point.child("v") else { continue };
            let text = value.text.as_str();
            let cached = match kind {
                Kind::Numbers => cache_number(text).ok()?.map_or(Value::Empty, Value::Number),
                Kind::Categories if text.is_empty() => Value::Empty,
                Kind::Categories => {
                    // Arc counts and alignment in addition to the owned text.
                    let bytes = text.len().checked_add(3 * size_of::<usize>())?;
                    remaining = remaining.checked_sub(bytes)?;
                    Value::text(text)
                }
            };
            let index = point_index(point, order).ok()? as usize;
            *points.get_mut(index)? = cached;
        }
        self.remaining = remaining;
        Some(points)
    }

    pub(crate) fn finish(&mut self, wb: &mut Workbook, report: &mut ReadReport) {
        let replaced = self.apply_pending(wb);
        if self.dropped {
            report.warn(
                "Some PivotChart cache points were dropped because they exceeded supported limits or contained invalid values, indices or counts.",
            );
        }
        if self.unverified {
            let message =
                "The workbook import budget prevented verification of some PivotChart cache values. Their worksheet references were retained.";
            report.notice(message.into());
            report.issue(CompatibilityIssue::new("PivotChart cache", None, CompatibilityEffect::Unknown, Editability::Unknown, message));
        }
        if replaced {
            let message = "Some PivotChart series use saved cache values because their worksheet references disagree. Refreshing the PivotTable rebinds the series.";
            report.notice(message.into());
            report.issue(CompatibilityIssue::new("PivotChart series", None, CompatibilityEffect::Static, Editability::Editable, message));
        }
    }

    /// Compare against cells only after all sheets have been loaded. Traverse the chart list
    /// once and move only the replacements, so large chart lists are not searched or cloned.
    fn apply_pending(&mut self, wb: &mut Workbook) -> bool {
        if self.pending.is_empty() {
            return false;
        }
        let mut replaced = false;
        let sheets: HashMap<String, usize> = wb.sheets.iter().enumerate().map(|(i, sh)| (sh.name.to_ascii_lowercase(), i)).collect();
        for si in 0..wb.sheets.len() {
            for ci in 0..wb.sheets[si].charts.len() {
                let chart = &wb.sheets[si].charts[ci];
                let Some(caches) = self.pending.remove(&chart.id) else { continue };
                for Pending { series, kind, data } in caches {
                    let reference = wb.sheets[si].charts[ci].series.get(series).and_then(|s| kind.reference(s));
                    let Some((sheet, range)) = reference.and_then(|f| reference_target(wb, &sheets, si, f)) else { continue };
                    match data {
                        CacheData::Literal(literal) if literal_disagrees(sheet, range, &literal) => {
                            if let Some(target) = wb.sheet_mut(si).and_then(|sh| sh.charts[ci].series.get_mut(series)) {
                                kind.replace(target, literal);
                                replaced = true;
                            }
                        }
                        CacheData::Compare(points) => self.dropped |= points_disagree(sheet, range, kind, &points),
                        CacheData::Literal(_) => {}
                    }
                }
            }
        }
        replaced
    }
}

/// Counts the slots and checks the source points without expanding sparse indices.
struct PointLayout {
    declared: Option<u32>,
    count: u32,
    dense: bool,
}

impl PointLayout {
    fn read(source: &El, kind: Kind) -> Result<Self, ()> {
        let declared = source.child("ptCount").map(|n| n.attr("val").ok_or(())?.trim().parse::<u32>().map_err(|_| ())).transpose()?;
        let mut highest = 0;
        let mut dense = true;
        let mut ordered = true;
        for (order, point) in source.kids("pt").enumerate() {
            let index = point_index(point, order)?;
            if declared.is_some_and(|n| index >= n) {
                return Err(());
            }
            ordered &= index >= highest;
            highest = highest.max(index.checked_add(1).ok_or(())?);
            let value = point.child("v").map(|v| v.text.as_str());
            dense &= index as usize == order && value.is_some();
            if let (Kind::Numbers, Some(text)) = (kind, value) {
                dense &= !text.trim().is_empty();
                cache_number(text)?;
            }
        }
        if !ordered {
            // Unordered points are valid. Check their indices without expanding gaps.
            // The chart part's XML node limit bounds this scratch vector of u32s.
            let mut indices = source.kids("pt").enumerate().map(|(order, point)| point_index(point, order)).collect::<Result<Vec<_>, _>>()?;
            indices.sort_unstable();
            if indices.windows(2).any(|pair| pair[0] == pair[1]) {
                return Err(());
            }
        }
        let count = declared.unwrap_or(highest);
        Ok(Self { declared, count, dense: dense && count == highest })
    }
}

fn point_index(point: &El, order: usize) -> Result<u32, ()> {
    match point.attr("idx") {
        Some(index) => index.trim().parse().map_err(|_| ()),
        None => u32::try_from(order).map_err(|_| ()),
    }
}

fn cache_number(text: &str) -> Result<Option<f64>, ()> {
    let text = text.trim();
    if text.is_empty() || text == "#N/A" {
        return Ok(None);
    }
    text.parse::<f64>().ok().filter(|n| n.is_finite()).map(Some).ok_or(())
}

/// Build an array constant in point order. The byte bound also satisfies the formula parser's
/// character limit. Both sparse expansion and quote escaping are checked before growing the output.
fn indexed_literal(source: &El, kind: Kind, count: usize, limit: usize) -> Option<String> {
    if count == 0 || count > limit {
        return None;
    }
    let mut points = vec![None; count];
    for (order, point) in source.kids("pt").enumerate() {
        let index = point_index(point, order).ok()? as usize;
        *points.get_mut(index)? = point.child("v").map(|v| v.text.as_str());
    }
    let mut literal = String::from("{");
    for (i, point) in points.into_iter().enumerate() {
        if i > 0 {
            literal.push(',');
        }
        let remaining = limit.saturating_sub(literal.len());
        let value = match kind {
            Kind::Numbers => cache_number(point.unwrap_or("")).ok()?.map_or_else(|| "#N/A".to_owned(), num),
            Kind::Categories => {
                let text = point.unwrap_or("");
                if text.len() + 2 > remaining {
                    return None;
                }
                format!("\"{}\"", text.replace('"', "\"\""))
            }
        };
        // Leave room for the closing brace.
        if value.len() >= remaining {
            return None;
        }
        literal.push_str(&value);
    }
    literal.push('}');
    Some(literal)
}

/// Only simple local cell or range references are candidates for fallback.
fn direct_reference(formula: &str) -> Option<Reference> {
    let Expr::Ref(reference) = gridcraft_formula::parse(formula).ok()? else { return None };
    let local = match &reference.sheet {
        SheetSel::Current => true,
        SheetSel::Named(name) => !name.contains(['[', ']']),
        SheetSel::Span(..) => false,
    };
    (local && matches!(reference.kind, RefKind::Cell(_) | RefKind::Range(..))).then_some(reference)
}

fn reference_target<'a>(wb: &'a Workbook, sheets: &HashMap<String, usize>, home: usize, formula: &str) -> Option<(&'a Sheet, RangeRef)> {
    let reference = direct_reference(formula)?;
    let si = match &reference.sheet {
        SheetSel::Current => home,
        SheetSel::Named(name) => *sheets.get(&name.to_ascii_lowercase())?,
        SheetSel::Span(..) => return None,
    };
    Some((wb.sheet(si)?, reference.range()))
}

fn category_matches(cell: &Value, text: &str) -> bool {
    cell.as_text().map_or_else(|| cell.display() == text, |actual| actual == text)
}

fn points_disagree(sheet: &Sheet, range: RangeRef, kind: Kind, points: &[Value]) -> bool {
    range.count() != points.len() as u64
        || range.iter().zip(points).any(|(cell, point)| {
            let value = sheet.value_ref(cell).unwrap_or(&Value::Empty);
            match kind {
                Kind::Numbers => value.as_f64() != point.as_f64(),
                Kind::Categories => !category_matches(value, point.as_text().unwrap_or("")),
            }
        })
}

/// Exact numeric comparison matters here: the writer must reproduce the cached f64 values.
fn literal_disagrees(sheet: &Sheet, range: RangeRef, literal: &str) -> bool {
    let Ok(Expr::Array(rows)) = gridcraft_formula::parse(literal) else { return false };
    let [points] = rows.as_slice() else { return false };
    if range.count() != points.len() as u64 {
        return true;
    }
    range.iter().zip(points).any(|(cell, point)| {
        let value = sheet.value_ref(cell).unwrap_or(&Value::Empty);
        match point {
            Expr::Number(n) => value.as_f64() != Some(*n),
            Expr::Error(_) => value.is_number(),
            Expr::Text(text) => !category_matches(value, text),
            _ => true,
        }
    })
}
