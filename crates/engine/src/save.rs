//! Revision-scoped save decisions. Low-level codecs deliberately do not enforce document policy.
use gridcraft_model::{CompatibilityEffect as Effect, CompatibilityIssue, Editability, Preserved};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{DocState, EngineError, Result, io::FileKind};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveAnalysis {
    pub document: u64,
    pub revision: u64,
    pub path: String,
    pub issues: Vec<CompatibilityIssue>,
}

/// JSON also survives the existing string-error transport used by CLI/control/MCP.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SaveFailure {
    pub error: String,
    pub analysis: SaveAnalysis,
}
impl std::fmt::Display for SaveFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match serde_json::to_string(self) {
            Ok(json) => f.write_str(&json),
            Err(_) => f.write_str("save analysis could not be encoded"),
        }
    }
}

pub fn destination_kind(doc: &DocState, path: &str) -> Option<FileKind> {
    FileKind::from_path(path).or(doc.content_kind.filter(|_| doc.path.as_deref() == Some(path)))
}

pub fn analyze_save(doc: &DocState, path: &str) -> Result<SaveAnalysis> {
    analyze_sheet(doc, path, doc.wb.active_sheet())
}

pub(crate) fn analyze_sheet(doc: &DocState, path: &str, sheet: usize) -> Result<SaveAnalysis> {
    let kind = destination_kind(doc, path)
        .ok_or_else(|| EngineError::Other("unknown file format; choose .xlsx, .xlsm, .json, .csv, .tsv or .html".into()))?;
    if matches!(kind, FileKind::Xls | FileKind::Xlsb | FileKind::Ods | FileKind::Sylk) {
        return Err(EngineError::Other("this format is supported for import only".into()));
    }
    let mut issues = doc.import_issues().to_vec();
    let mut add = |feature: &str, location: Option<String>, effect: Effect, editability: Editability, description: &str| {
        issues.push(CompatibilityIssue::new(feature, location, effect, editability, description));
    };
    let flat = matches!(kind, FileKind::Csv | FileKind::Tsv | FileKind::Html);
    let phonetic_loss = if flat {
        doc.wb.phonetics.iter().any(|p| !p.runs.is_empty() || p.props.is_some())
    } else if kind == FileKind::Xlsx {
        gridcraft_xlsx::xlsx_known_phonetic_loss(&doc.wb).map_err(|e| EngineError::Other(e.to_string()))?
    } else {
        false
    };
    if phonetic_loss {
        add(
            "phonetic annotations",
            None,
            Effect::Omitted,
            Editability::PreservedOnly,
            "This destination omits or changes known phonetic annotation data.",
        );
    }
    if doc.wb.vba_project.is_some() && (flat || (kind == FileKind::Xlsx && !gridcraft_xlsx::XlsxKind::from_path(path).is_macro_enabled())) {
        add(
            "VBA project",
            None,
            Effect::Omitted,
            Editability::PreservedOnly,
            "This destination omits the VBA project. Use .xlsm or .xltm to retain it.",
        );
    }
    for (location, kept) in
        std::iter::once((None, doc.wb.preserved.as_deref())).chain(doc.wb.sheets.iter().map(|sh| (Some(sh.name.clone()), sh.preserved.as_deref())))
    {
        if let Some(kept) = kept.filter(|p| !p.is_empty()) {
            if kind != FileKind::Xlsx {
                add(
                    "preserved package content",
                    location,
                    Effect::Omitted,
                    Editability::PreservedOnly,
                    "This destination cannot retain imported content that GridCraft keeps in its original form.",
                );
            } else if has_unadjusted_references(kept) {
                add(
                    "preserved references",
                    location,
                    Effect::ReferenceUnadjusted,
                    Editability::PreservedOnly,
                    "Cell references inside preserved parts are not adjusted when rows, columns or sheets change. Verify them in the original application.",
                );
            }
        }
    }
    for sh in &doc.wb.sheets {
        if kind == FileKind::Xlsx {
            for slicer in &sh.slicers {
                add(
                    "slicer",
                    Some(format!("{}!{}: {}", sh.name, slicer.anchor.cell.a1(), slicer.caption)),
                    Effect::Omitted,
                    Editability::Editable,
                    "This format cannot preserve this slicer; its filter widget will be lost.",
                );
            }
        }
    }
    if flat {
        // Flat formats deliberately publish one rendered sheet. Enumerate the supported
        // classes of loss rather than suggesting an arbitrary package round-trip check.
        if doc.wb.sheets.len() > 1 {
            add("other worksheets", None, Effect::Omitted, Editability::Editable, "Only the selected worksheet is exported.");
        }
        if let Some(sh) = doc.wb.sheet(sheet) {
            let loc = Some(sh.name.clone());
            if sh.cells.iter().any(|(_, c)| c.formula.is_some() || c.py.is_some()) {
                add("formulas", loc.clone(), Effect::Static, Editability::Editable, "Formulas and live calculations become displayed values.");
            }
            if (kind != FileKind::Html && (!sh.charts.is_empty() || !sh.images.is_empty()))
                || !sh.shapes.is_empty()
                || !sh.slicers.is_empty()
                || !sh.controls.is_empty()
                || !sh.cell_pictures.is_empty()
            {
                add(
                    "drawing objects",
                    loc.clone(),
                    Effect::Omitted,
                    Editability::Editable,
                    "This destination omits drawing objects it cannot represent (shapes, slicers, controls and cell pictures; text formats also omit charts and floating pictures).",
                );
            }
            if kind == FileKind::Html {
                for (present, feature, description) in [
                    (
                        html_omits_hidden_cell_data(sh),
                        "hidden cell data",
                        "This web page omits cell data in hidden rows, and in hidden columns when pictures or charts determine its layout.",
                    ),
                    (!sh.hyperlinks.is_empty(), "hyperlinks", "Link text is retained, but hyperlink targets are omitted."),
                    (!sh.cond_formats.is_empty(), "conditional formatting", "Conditional formatting rules and their appearance are omitted."),
                ] {
                    if present {
                        add(feature, loc.clone(), Effect::Omitted, Editability::Editable, description);
                    }
                }
            }
            if kind == FileKind::Html && !sh.charts.is_empty() {
                add("charts", loc.clone(), Effect::Static, Editability::Editable, "Charts become static SVG pictures without live source links.");
            }
            if !sh.comments.is_empty()
                || !sh.validations.is_empty()
                || !sh.tables.is_empty()
                || !sh.pivots.is_empty()
                || !sh.sparklines.is_empty()
                || sh.autofilter.is_some()
                || sh.protection.is_some()
                || !sh.edit_ranges.is_empty()
            {
                add(
                    "worksheet features",
                    loc.clone(),
                    Effect::Omitted,
                    Editability::Editable,
                    "Notes, validation, tables, pivots, sparklines, filters and protection are omitted.",
                );
            }
            if kind != FileKind::Html
                && (!sh.merges.is_empty()
                    || !sh.cols.is_empty()
                    || !sh.rows.is_empty()
                    || !sh.cond_formats.is_empty()
                    || !sh.hyperlinks.is_empty()
                    || sh.cells.iter().any(|(_, c)| c.style != gridcraft_model::StyleId(0)))
            {
                add(
                    "formatting and layout",
                    loc,
                    Effect::Omitted,
                    Editability::Editable,
                    "Text exports omit formatting, layout, merged cells and hyperlinks.",
                );
            }
        }
        if !doc.wb.names.is_empty() || !doc.wb.macros.is_empty() || !doc.wb.macros_unread.is_empty() || doc.wb.protected_structure {
            add("workbook features", None, Effect::Omitted, Editability::Editable, "Defined names, scripts and workbook protection are omitted.");
        }
    }
    Ok(SaveAnalysis { document: doc.uid, revision: doc.revision, path: path.into(), issues })
}

fn html_omits_hidden_cell_data(sheet: &gridcraft_model::Sheet) -> bool {
    // Match to_html: rows are always skipped; hidden columns only in its fixed layout.
    let fixed = !(sheet.images.is_empty() && sheet.charts.is_empty());
    sheet.cells.iter().any(|(cell, value)| value.has_content() && (sheet.is_row_hidden(cell.row) || (fixed && sheet.is_col_hidden(cell.col))))
}

fn has_unadjusted_references(kept: &Preserved) -> bool {
    kept.rels.iter().any(|r| matches!(r.rel_type.rsplit('/').next(), Some("queryTable" | "tableSingleCells" | "control" | "ctrlProp" | "oleObject")))
        || kept.elements.iter().any(|e| matches!(e.slot.as_str(), "controls" | "oleObjects"))
}

pub(crate) fn check(doc: &DocState, path: &str, params: &Value) -> Result<SaveAnalysis> {
    check_sheet(doc, path, params, doc.wb.active_sheet())
}

pub(crate) fn check_sheet(doc: &DocState, path: &str, params: &Value, sheet: usize) -> Result<SaveAnalysis> {
    let analysis = analyze_sheet(doc, path, sheet)?;
    let expected_revision = params.get("expectedRevision").and_then(Value::as_u64);
    let expected_document = params.get("expectedDocument").and_then(Value::as_u64);
    let supplied = params.get("confirmedLosses").and_then(Value::as_array);
    let stale = (params.get("expectedRevision").is_some() && expected_revision != Some(doc.revision))
        || (params.get("expectedDocument").is_some() && expected_document != Some(doc.uid));
    let confirmed = expected_revision == Some(doc.revision)
        && expected_document == Some(doc.uid)
        && supplied.is_some_and(|codes| {
            codes.iter().all(Value::is_string) && analysis.issues.iter().all(|issue| codes.iter().any(|c| c.as_str() == Some(&issue.code)))
        });
    if stale || (!analysis.issues.is_empty() && !confirmed) {
        return Err(EngineError::SaveBlocked(SaveFailure { error: if stale { "save.stale" } else { "save.losses" }.into(), analysis }));
    }
    Ok(analysis)
}
