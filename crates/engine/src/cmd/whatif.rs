//! What-If Analysis (Data › Forecast): Goal Seek, Scenario Manager, Data Tables; and the Data
//! Tools that work on whole lists: Advanced Filter and Consolidate.
//!
//! Every what-if evaluation is a *trial*: the workbook is cloned (cheap — sheets and cell bands
//! are copy-on-write), the input cells are overwritten, and only their dependents are
//! recalculated with the document's dependency graph. The document itself only changes when a
//! command writes its final answer (one undo step).

use std::collections::{HashMap, HashSet};

use gridcraft_calc::Calc;
use gridcraft_core::{CellRef, RangeRef, Value};
use gridcraft_model::{CalcMode, Cell, DefinedName, Sheet, StyleId, Workbook};
use serde::{Deserialize, Serialize};
use serde_json::{Value as Json, json};

use super::{CommandSpec, DocState, EngineError, Result, Session, bad, bool_param, cmd, edit, f64_param, has_doc, split_sheet, str_param};

mod advanced_filter;
mod consolidate;
mod scenarios;

use advanced_filter::advanced_filter;
#[cfg(test)]
pub(crate) use advanced_filter::criterion_matches;
use consolidate::consolidate;
#[cfg(test)]
pub(crate) use scenarios::{SCENARIO_NAME, load_scenarios};
use scenarios::{scenario_add, scenario_delete, scenario_list, scenario_show, scenario_summary};

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "data.goalSeek",
            "Goal Seek…",
            ["Data", "Forecast", "What-If Analysis"],
            None,
            "{set: \"B5\" (formula cell), to: number, changing: \"B2\" (constant cell)} → {value, result, iterations, converged}. Secant search with bisection once the answer is bracketed; stops within Maximum Change (0.001) or after Maximum Iterations (100) from File › Options › Formulas. Writes the found value into `changing`.",
            has_doc,
            goal_seek
        ),
        cmd!(
            query "data.scenarioManager",
            "Scenario Manager…",
            ["Data", "Forecast", "What-If Analysis"],
            None,
            "{sheet?} → {scenarios: [{name, sheet, changing, values, comment}]} (scenarios of the active sheet; `sheet: \"*\"` lists all)",
            has_doc,
            scenario_list
        ),
        cmd!(
            "data.scenarioAdd",
            "Add Scenario",
            [],
            None,
            "{name, changing: \"B2:B4\" or \"B2,B4\" (≤32 cells, active sheet), values?: [..] (default: the cells' current values), comment?, replace?: bool}",
            has_doc,
            scenario_add
        ),
        cmd!("data.scenarioShow", "Show Scenario", [], None, "{name} writes the scenario's values into its changing cells", has_doc, scenario_show),
        cmd!("data.scenarioDelete", "Delete Scenario", [], None, "{name}", has_doc, scenario_delete),
        cmd!(
            "data.scenarioSummary",
            "Scenario Summary",
            [],
            None,
            "{resultCells?: \"B10\" or \"B10:B12,D4\"} inserts a \"Scenario Summary\" sheet: changing cells and result cells for the current values and each scenario",
            has_doc,
            scenario_summary
        ),
        cmd!(
            "data.dataTable",
            "Data Table…",
            ["Data", "Forecast", "What-If Analysis"],
            None,
            "{range: \"D2:E10\", rowInput?: \"B1\", columnInput?: \"B2\"} one-variable (values down the first column with columnInput, or across the top row with rowInput; formulas in the other edge) or two-variable (formula in the top-left corner). Computes every result by substitution and writes static values (a snapshot, not a live {=TABLE()}); ≤100,000 cells.",
            has_doc,
            data_table
        ),
        cmd!(
            "data.advancedFilter",
            "Advanced…",
            ["Data", "Sort & Filter"],
            None,
            "{range? (list with headers; default current region), criteria?: \"H1:I3\" (omitted/blank matches all rows; header row of field names; rows OR'ed, columns AND'ed; \">100\", \"=abc\", \"a*\", \"<>\"; blank-headed formula cells are computed criteria for the first data row), copyTo?: \"K1\" (else filter in place by hiding rows), unique?: bool}",
            has_doc,
            advanced_filter
        ),
        cmd!(
            "data.consolidate",
            "Consolidate…",
            ["Data", "Data Tools"],
            None,
            "{sources: [\"Sheet2!A1:C10\", ..], destination?: \"A1\" (default active cell), function?: sum|count|countNums|average|max|min|product, topRow?: bool, leftColumn?: bool (consolidate by labels; else by position)}",
            has_doc,
            consolidate
        ),
    ]
}

// ---------------------------------------------------------------- parameters

/// Resolves an optional sheet name (else `default`).
fn sheet_named(wb: &Workbook, name: Option<String>, default: usize, cmd: &str) -> Result<usize> {
    match name {
        Some(n) => wb.sheet_index(&n).ok_or_else(|| bad(cmd, format!("no sheet named `{n}`"))),
        None => Ok(default),
    }
}

/// A `"B5"` / `"Sheet2!B5"` parameter.
fn cell_arg(s: &Session, p: &Json, key: &str, cmd: &str) -> Result<(usize, CellRef)> {
    let d = s.doc()?;
    let t = str_param(p, key).ok_or_else(|| bad(cmd, format!("missing `{key}`")))?;
    let (sh, body) = split_sheet(t);
    let si = sheet_named(&d.wb, sh, d.wb.active_sheet(), cmd)?;
    let c = CellRef::parse(body.trim()).ok_or_else(|| bad(cmd, format!("`{key}` is not a cell: `{t}`")))?;
    Ok((si, c))
}

/// A range text (`"A1:C9"`, `"Sheet2!A:C"`) resolved against the workbook. Whole rows/columns
/// are trimmed to the used range.
pub(crate) fn parse_range(wb: &Workbook, t: &str, cmd: &str) -> Result<(usize, RangeRef)> {
    let (sh, body) = split_sheet(t);
    let si = sheet_named(wb, sh, wb.active_sheet(), cmd)?;
    let r = RangeRef::parse(body.trim()).ok_or_else(|| bad(cmd, format!("not a range: `{t}`")))?;
    Ok((si, trim_to_used(wb, si, r)))
}

fn trim_to_used(wb: &Workbook, si: usize, r: RangeRef) -> RangeRef {
    if !(r.is_full_cols() || r.is_full_rows()) {
        return r;
    }
    match wb.sheet(si).and_then(|s| s.used_range()) {
        Some(u) => r.intersection(&RangeRef::new(CellRef::new(0, 0), u.end)).unwrap_or(RangeRef::cell(r.start)),
        None => RangeRef::cell(r.start),
    }
}

fn range_arg(s: &Session, p: &Json, key: &str, cmd: &str) -> Result<(usize, RangeRef)> {
    let d = s.doc()?;
    let t = str_param(p, key).ok_or_else(|| bad(cmd, format!("missing `{key}`")))?;
    parse_range(&d.wb, t, cmd)
}

/// Cells of `"B2:B4,D1"` (row-major per area, duplicates removed), at most `cap`.
fn cell_list(wb: &Workbook, t: &str, cmd: &str, cap: usize) -> Result<(usize, Vec<CellRef>)> {
    let mut sheet = None;
    let mut out: Vec<CellRef> = Vec::new();
    for part in t.split(',').map(str::trim).filter(|x| !x.is_empty()) {
        let (si, r) = parse_range(wb, part, cmd)?;
        if *sheet.get_or_insert(si) != si {
            return Err(bad(cmd, "all cells must be on one sheet"));
        }
        if r.count() > cap as u64 {
            return Err(bad(cmd, format!("too many cells (at most {cap})")));
        }
        for c in r.iter() {
            if !out.contains(&c) {
                out.push(c);
            }
        }
        if out.len() > cap {
            return Err(bad(cmd, format!("too many cells (at most {cap})")));
        }
    }
    Ok((sheet.unwrap_or(wb.active_sheet()), out))
}

/// What a JSON value typed into a cell is, as a constant.
fn json_value(v: &Json, wb: &Workbook) -> Value {
    match v {
        Json::Null => Value::Empty,
        Json::Bool(b) => Value::Bool(*b),
        Json::Number(n) => n.as_f64().map(Value::number).unwrap_or_default(),
        Json::String(s) if s.is_empty() => Value::Empty,
        Json::String(s) => gridcraft_core::parse::parse_input(s, wb.date_system).value,
        other => Value::text(other.to_string()),
    }
}

fn value_json(v: &Value) -> Json {
    match v {
        Value::Empty => Json::Null,
        Value::Number(n) => json!(n),
        Value::Bool(b) => json!(b),
        Value::Text(t) => json!(t.as_ref()),
        Value::Error(e) => json!(e.as_str()),
        Value::Array(a) => value_json(&a.data.first().cloned().unwrap_or_default()),
    }
}

// ---------------------------------------------------------------- trials

/// Evaluates a workbook with some input cells replaced, without touching the document.
pub(crate) struct Trial {
    base: Workbook,
    calc: Calc,
}

impl Trial {
    pub(crate) fn new(d: &DocState) -> Trial {
        let mut base = (*d.wb).clone();
        // What-if analysis always recalculates, even in manual mode.
        base.calc.mode = CalcMode::Automatic;
        Trial { base, calc: d.calc.clone() }
    }

    /// Sets `sets` to constants, recalculates their dependents and reads `reads`.
    pub(crate) fn run(&mut self, sets: &[(usize, CellRef, Value)], reads: &[(usize, CellRef)]) -> Vec<Value> {
        let mut wb = self.base.clone();
        let mut keys = Vec::with_capacity(sets.len());
        for (si, c, v) in sets {
            if let Some(sh) = wb.sheet_mut(*si) {
                let style = sh.style_id(*c);
                sh.set_cell(*c, Cell { value: v.clone(), formula: None, style, py: None });
                keys.push((*si, *c));
            }
        }
        self.calc.cells_changed(&mut wb, &keys);
        reads.iter().map(|(si, c)| wb.sheet(*si).map(|s| s.value(*c)).unwrap_or_default()).collect()
    }
}

// ---------------------------------------------------------------- goal seek

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Seek {
    pub x: f64,
    pub fx: f64,
    pub iterations: u32,
    pub converged: bool,
}

/// Finds `x` with `|f(x)| <= tol`: secant steps (with growing steps while nothing is
/// bracketed), switching to secant-or-bisection inside a sign-change bracket. Returns the best
/// point seen. `f` returns NaN where the model errors.
pub(crate) fn seek(x0: f64, f: &mut dyn FnMut(f64) -> f64, max_iter: u32, tol: f64) -> Seek {
    let tol = if tol.is_finite() && tol > 0.0 { tol } else { 0.001 };
    let max_iter = max_iter.clamp(1, 10_000);
    let x0 = if x0.is_finite() { x0 } else { 0.0 };
    let f0 = f(x0);
    let mut best = (x0, f0);
    let consider = |best: &mut (f64, f64), x: f64, fx: f64| {
        if fx.is_finite() && (!best.1.is_finite() || fx.abs() < best.1.abs()) {
            *best = (x, fx);
        }
    };
    let done = |best: &(f64, f64), it: u32| Seek { x: best.0, fx: best.1, iterations: it, converged: best.1.is_finite() && best.1.abs() <= tol };
    if f0.is_finite() && f0.abs() <= tol {
        return done(&best, 0);
    }
    let step0 = if x0 == 0.0 { 0.01 } else { x0.abs() * 0.01 };
    let (mut xa, mut fa) = (x0, f0);
    let mut xb = x0 + step0;
    let mut fb = f(xb);
    let mut it = 1u32;
    consider(&mut best, xb, fb);
    // (lo, f(lo), hi, f(hi)) with a sign change.
    let mut bracket: Option<(f64, f64, f64, f64)> = None;
    let mut slow = 0u32;
    while it < max_iter {
        if fb.is_finite() && fb.abs() <= tol {
            break;
        }
        if bracket.is_none() && fa.is_finite() && fb.is_finite() && (fa < 0.0) != (fb < 0.0) {
            bracket = Some((xa, fa, xb, fb));
        }
        let secant = if fa.is_finite() && fb.is_finite() && fb != fa { xb - fb * (xb - xa) / (fb - fa) } else { f64::NAN };
        let next = match bracket {
            Some((lo, _, hi, _)) => {
                let (a, b) = (lo.min(hi), lo.max(hi));
                if slow >= 2 || !(secant > a && secant < b) { (lo + hi) / 2.0 } else { secant }
            }
            None => {
                let span = (xb - xa).abs().max(f64::MIN_POSITIVE);
                if !fb.is_finite() {
                    // The model errors here: step back towards the last good point.
                    (xa + xb) / 2.0
                } else if !secant.is_finite() {
                    xb + 2.0 * (xb - xa)
                } else if (secant - xb).abs() > 100.0 * span + 1.0 {
                    xb + (secant - xb).signum() * (100.0 * span + 1.0)
                } else {
                    secant
                }
            }
        };
        if !next.is_finite() {
            break;
        }
        let fnext = f(next);
        it += 1;
        consider(&mut best, next, fnext);
        if let Some((lo, flo, hi, fhi)) = bracket {
            let width = (hi - lo).abs();
            if fnext.is_finite() {
                bracket = Some(if (fnext < 0.0) == (flo < 0.0) { (next, fnext, hi, fhi) } else { (lo, flo, next, fnext) });
            } else {
                // An error inside the bracket: shrink towards the better end.
                bracket = Some(if flo.abs() < fhi.abs() { (lo, flo, next, fhi) } else { (next, flo, hi, fhi) });
            }
            if let Some((l, _, h, _)) = bracket {
                let nw = (h - l).abs();
                slow = if nw > width * 0.5 { slow + 1 } else { 0 };
                if nw <= f64::EPSILON * (1.0 + l.abs().max(h.abs())) {
                    break;
                }
            }
        }
        xa = xb;
        fa = fb;
        xb = next;
        fb = fnext;
    }
    done(&best, it)
}

fn goal_seek(s: &mut Session, p: &Json) -> Result<Json> {
    const CMD: &str = "data.goalSeek";
    let (ssi, set) = cell_arg(s, p, "set", CMD)?;
    let (csi, chg) = cell_arg(s, p, "changing", CMD)?;
    let target = f64_param(p, "to")
        .or_else(|| str_param(p, "to").and_then(gridcraft_core::parse::parse_number_text))
        .filter(|v| v.is_finite())
        .ok_or_else(|| bad(CMD, "`to` must be a number"))?;
    let d = s.doc()?;
    let has_formula = |si: usize, c: CellRef| d.wb.sheet(si).and_then(|sh| sh.cell(c)).is_some_and(|c| c.formula.is_some());
    if !has_formula(ssi, set) {
        return Err(EngineError::Other("Cell must contain a formula.".into()));
    }
    // A Python cell is a formula to Excel (`=PY(…)`): writing a number over it would lose its code.
    if has_formula(csi, chg) || d.wb.sheet(csi).and_then(|sh| sh.cell(chg)).is_some_and(|c| c.py.is_some()) {
        return Err(EngineError::Other("Cell must contain a value.".into()));
    }
    let x0 = d.wb.sheet(csi).map(|sh| sh.value(chg)).and_then(|v| v.as_f64()).unwrap_or(0.0);
    let (max_iter, tol) = (d.wb.calc.max_iterations, d.wb.calc.max_change);
    let mut trial = Trial::new(d);
    let mut f = |x: f64| -> f64 {
        trial.run(&[(csi, chg, Value::number(x))], &[(ssi, set)]).first().and_then(Value::as_f64).map(|v| v - target).unwrap_or(f64::NAN)
    };
    let r = seek(x0, &mut f, max_iter, tol);
    edit(s, |cx| {
        let sh = cx.sheet_mut(csi)?;
        let style = sh.style_id(chg);
        sh.set_cell(chg, Cell { value: Value::number(r.x), formula: None, style, py: None });
        cx.touch(csi, chg);
        Ok(())
    })?;
    let result = s.doc()?.wb.sheet(ssi).map(|sh| sh.value(set)).unwrap_or_default();
    Ok(json!({"value": r.x, "result": value_json(&result), "iterations": r.iterations, "converged": r.converged}))
}

// ---------------------------------------------------------------- data tables

const MAX_TABLE_CELLS: u64 = 100_000;

fn data_table(s: &mut Session, p: &Json) -> Result<Json> {
    const CMD: &str = "data.dataTable";
    let (si, r) = range_arg(s, p, "range", CMD)?;
    let row_in = if p.get("rowInput").and_then(Json::as_str).is_some_and(|t| !t.is_empty()) { Some(cell_arg(s, p, "rowInput", CMD)?) } else { None };
    let col_in =
        if p.get("columnInput").and_then(Json::as_str).is_some_and(|t| !t.is_empty()) { Some(cell_arg(s, p, "columnInput", CMD)?) } else { None };
    if row_in.is_none() && col_in.is_none() {
        return Err(bad(CMD, "give `rowInput`, `columnInput` or both"));
    }
    if r.height() < 2 || r.width() < 2 {
        return Err(bad(CMD, "the table needs at least two rows and two columns"));
    }
    let body = RangeRef::new(CellRef::new(r.start.row + 1, r.start.col + 1), r.end);
    if body.count() > MAX_TABLE_CELLS {
        return Err(bad(CMD, format!("the table body is too large ({MAX_TABLE_CELLS} cells max)")));
    }
    for input in [row_in, col_in].into_iter().flatten() {
        if input.0 == si && r.contains(input.1) {
            return Err(EngineError::Other("Input cell reference is not valid.".into()));
        }
    }
    let d = s.doc()?;
    let sh = d.wb.sheet(si).ok_or(EngineError::NoDocument)?;
    let (r0, c0) = (r.start.row, r.start.col);
    let mut trial = Trial::new(d);
    let mut out: Vec<(CellRef, Value)> = Vec::with_capacity(body.count() as usize);
    let mode = match (row_in, col_in) {
        (None, Some((isi, ic))) => {
            let reads: Vec<(usize, CellRef)> = (c0 + 1..=r.end.col).map(|c| (si, CellRef::new(r0, c))).collect();
            for row in r0 + 1..=r.end.row {
                let v = sh.value(CellRef::new(row, c0));
                let vals = trial.run(&[(isi, ic, v)], &reads);
                for (k, val) in vals.into_iter().enumerate() {
                    out.push((CellRef::new(row, c0 + 1 + k as u32), val));
                }
            }
            "column"
        }
        (Some((isi, ic)), None) => {
            let reads: Vec<(usize, CellRef)> = (r0 + 1..=r.end.row).map(|row| (si, CellRef::new(row, c0))).collect();
            for col in c0 + 1..=r.end.col {
                let v = sh.value(CellRef::new(r0, col));
                let vals = trial.run(&[(isi, ic, v)], &reads);
                for (k, val) in vals.into_iter().enumerate() {
                    out.push((CellRef::new(r0 + 1 + k as u32, col), val));
                }
            }
            "row"
        }
        (Some((rsi, rc)), Some((csi, cc))) => {
            let reads = [(si, CellRef::new(r0, c0))];
            for row in r0 + 1..=r.end.row {
                let cv = sh.value(CellRef::new(row, c0));
                for col in c0 + 1..=r.end.col {
                    let rv = sh.value(CellRef::new(r0, col));
                    let vals = trial.run(&[(rsi, rc, rv), (csi, cc, cv.clone())], &reads);
                    out.push((CellRef::new(row, col), vals.into_iter().next().unwrap_or_default()));
                }
            }
            "twoVariable"
        }
        (None, None) => return Err(bad(CMD, "give `rowInput`, `columnInput` or both")),
    };
    let n = out.len();
    edit(s, |cx| {
        for (c, v) in out {
            let sh = cx.sheet_mut(si)?;
            let style = sh.style_id(c);
            sh.set_cell(c, Cell { value: v, formula: None, style, py: None });
            cx.touch(si, c);
        }
        Ok(())
    })?;
    Ok(json!({"range": body.a1(), "cells": n, "mode": mode, "static": true}))
}

#[cfg(test)]
mod tests;

const IMPORTED_TABLE_EDIT: &str = "Imported what-if tables keep static cached results; edits to their results, headers or inputs and address-changing operations are not supported.";

/// A copied sheet has no previous owner with its new UID. Prove native text does not need
/// source-owner rebinding at the copy command itself, without changing ordinary formula copying.
pub(super) fn check_imported_table_copy(source: &Sheet, name: &str) -> Result<()> {
    for table in &source.what_if_tables {
        if table.formula_text.trim().is_empty() {
            continue;
        }
        let Ok(expr) = gridcraft_formula::parse(&table.formula_text) else {
            return Err(EngineError::Other(IMPORTED_TABLE_EDIT.into()));
        };
        if gridcraft_formula::adjust::rename_sheet(expr.clone(), &source.name, name) != expr {
            return Err(EngineError::Other(IMPORTED_TABLE_EDIT.into()));
        }
    }
    Ok(())
}

/// Cached data tables have no live evaluation yet. Refuse content/address changes before the
/// candidate workbook is published, using the same transactional boundary as array protection.
/// Outside precedents are intentionally not tracked; the import compatibility issue says so.
pub(super) fn check_imported_tables(
    old: &gridcraft_model::Workbook,
    new: &gridcraft_model::Workbook,
    edits: &[(String, gridcraft_formula::adjust::Edit)],
) -> Result<()> {
    use gridcraft_formula::adjust::{self, Edit};
    if old.sheets.iter().all(|sh| sh.what_if_tables.is_empty()) {
        return Ok(());
    }
    let renames: Vec<_> = old
        .sheets
        .iter()
        .filter_map(|prior| {
            let next = new.sheets.iter().find(|sh| sh.uid.get() == prior.uid.get())?;
            (prior.name != next.name).then_some((prior.name.as_str(), next.name.as_str()))
        })
        .collect();
    let removed: Vec<_> =
        old.sheets.iter().filter(|prior| !new.sheets.iter().any(|sh| sh.uid.get() == prior.uid.get())).map(|sh| sh.name.as_str()).collect();
    let old_names: Vec<_> = if removed.is_empty() { Vec::new() } else { old.sheets.iter().map(|sh| sh.name.clone()).collect() };
    let rewrite = |mut expr: gridcraft_formula::Expr, host: &str| {
        for (target, edit) in edits {
            expr = adjust::adjust(expr, host, target, edit);
        }
        for &(prior, next) in &renames {
            expr = adjust::rename_sheet(expr, prior, next);
        }
        expr
    };
    for before in &old.sheets {
        if before.what_if_tables.is_empty() {
            continue;
        }
        // A whole owner deletion removes its native definitions too. Sheet identity also
        // survives renaming/reordering, unlike its name or vector position.
        let Some(after) = new.sheets.iter().find(|sh| sh.uid.get() == before.uid.get()) else { continue };
        let refuse = || EngineError::Other(IMPORTED_TABLE_EDIT.into());
        if before.what_if_tables != after.what_if_tables {
            return Err(refuse());
        }
        let changed = before.cells.diff(&after.cells);
        let content_matches = |cell| {
            let formulas = before.cell(cell).and_then(|c| c.formula.as_ref()).zip(after.cell(cell).and_then(|c| c.formula.as_ref()));
            if formulas.is_some_and(|(a, b)| a.eval_mode != b.eval_mode || a.dynamic != b.dynamic) {
                return false;
            }
            if super::same_content(before.cell(cell), after.cell(cell)) {
                return true;
            }
            // Admit only the exact reference transformation recorded by the command, without
            // treating that rewrite as a direct header edit. Native coordinates stay guarded.
            let Some((a, b)) = formulas else { return false };
            let Some(expr) = a.expr() else { return false };
            (!edits.is_empty() || !renames.is_empty()) && a.array == b.array && b.expr().is_some_and(|actual| actual == rewrite(expr, &before.name))
        };
        for table in &before.what_if_tables {
            let ranges = protected_table_ranges(table);
            if edits.iter().any(|(sheet, edit)| {
                (sheet == &before.name && ranges.iter().any(|r| edit_affects_range(edit, *r)))
                    || matches!(edit, Edit::MoveToSheet { from, to_sheet, to_row, to_col }
                        if to_sheet == &before.name && ranges.iter().any(|r| move_destination(*from, *to_row, *to_col).intersects(r)))
            }) {
                return Err(refuse());
            }
            // Native text is inert, but may still contain addresses. Pure adjustment can prove
            // its references unchanged; an unparseable or changed expression is not such proof.
            // Keep the original text verbatim rather than inventing native TABLE rewrite rules.
            if !table.formula_text.trim().is_empty() && (!edits.is_empty() || !renames.is_empty() || !removed.is_empty()) {
                let Ok(expr) = gridcraft_formula::parse(&table.formula_text) else { return Err(refuse()) };
                let mut expected = rewrite(expr.clone(), &before.name);
                for name in &removed {
                    expected = adjust::delete_sheet(expected, name, &old_names);
                }
                if expected != expr {
                    return Err(refuse());
                }
            }
            for &cell in &changed {
                if ranges.iter().any(|r| r.contains(cell)) && !content_matches(cell) {
                    return Err(refuse());
                }
            }
            if before.cell_pictures != after.cell_pictures
                && before
                    .cell_pictures
                    .keys()
                    .chain(after.cell_pictures.keys())
                    .any(|c| ranges.iter().any(|r| r.contains(*c)) && before.cell_pictures.get(c) != after.cell_pictures.get(c))
            {
                return Err(refuse());
            }
        }
    }
    Ok(())
}

fn protected_table_ranges(table: &gridcraft_model::WhatIfTable) -> Vec<RangeRef> {
    let r = table.range;
    let mut ranges = vec![r, RangeRef::cell(table.anchor)];
    // Microsoft Support's one-variable layouts have formula headers above (column-oriented)
    // or left (row-oriented), and substitution values on the other edge. Both edges matter.
    // Two-variable tables also use the top-left corner as the formula. One-variable corners
    // are unused. At worksheet edges protect only existing cells, without wrapping indices.
    if let Some(row) = r.start.row.checked_sub(1) {
        ranges.push(RangeRef::new(CellRef::new(row, r.start.col), CellRef::new(row, r.end.col)));
    }
    if let Some(col) = r.start.col.checked_sub(1) {
        ranges.push(RangeRef::new(CellRef::new(r.start.row, col), CellRef::new(r.end.row, col)));
        if table.two_variable == Some(true)
            && let Some(row) = r.start.row.checked_sub(1)
        {
            ranges.push(RangeRef::cell(CellRef::new(row, col)));
        }
    }
    ranges.extend([table.input1, table.input2].into_iter().flatten().map(RangeRef::cell));
    ranges
}

fn edit_affects_range(edit: &gridcraft_formula::adjust::Edit, protected: RangeRef) -> bool {
    use gridcraft_formula::adjust::{Axis, Edit};
    match edit {
        Edit::Insert { axis, at, count } | Edit::Delete { axis, at, count } => {
            *count > 0
                && match axis {
                    Axis::Rows => protected.end.row >= *at,
                    Axis::Cols => protected.end.col >= *at,
                }
        }
        Edit::InsertCells { axis, range } | Edit::DeleteCells { axis, range } => {
            let shifted = match axis {
                Axis::Rows => RangeRef::new(range.start, CellRef::new(gridcraft_core::MAX_ROWS - 1, range.end.col)),
                Axis::Cols => RangeRef::new(range.start, CellRef::new(range.end.row, gridcraft_core::MAX_COLS - 1)),
            };
            shifted.intersects(&protected)
        }
        Edit::Move { from, to_row, to_col } => from.intersects(&protected) || move_destination(*from, *to_row, *to_col).intersects(&protected),
        Edit::MoveToSheet { from, .. } => from.intersects(&protected),
    }
}

fn move_destination(from: RangeRef, row: u32, col: u32) -> RangeRef {
    RangeRef::new(CellRef::new(row, col), CellRef::new(row.saturating_add(from.height() - 1), col.saturating_add(from.width() - 1)))
}
