//! Prepare bounded reference adjustments before the engine publishes a candidate workbook.
//! Unproved graphs stay byte-identical for unrelated edits and refuse invalidating operations.

use std::collections::HashMap;
use std::sync::Arc;

use gridcraft_formula::adjust::Edit;
use gridcraft_formula::{Expr, RefKind, SheetSel};
use gridcraft_model::{Preserved, Sheet, Workbook};

use super::package::normalize;

mod controls;
mod mappings;
pub(crate) mod xml;

const MAIN: &str = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";
const REL: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const X14: &str = "http://schemas.microsoft.com/office/spreadsheetml/2009/9/main";
const VML: &str = "urn:schemas-microsoft-com:vml";
const EXCEL: &str = "urn:schemas-microsoft-com:office:excel";
const DRAWING: &str = "http://schemas.openxmlformats.org/drawingml/2006/spreadsheetDrawing";
type Checked<T> = Result<T, String>;

fn unsupported() -> String {
    "Preserved XLSX content cannot safely follow this edit; its references or object geometry are unsupported.".into()
}
fn require(ok: bool) -> Checked<()> {
    if ok { Ok(()) } else { Err(unsupported()) }
}

/// A sheet's kept graph cannot be silently omitted or copied with conflicting identities.
pub fn check_preserved_sheet_copy(source: &Sheet) -> Checked<()> {
    require(source.preserved.as_deref().is_none_or(Preserved::is_empty))
}

struct Changes<'a> {
    before: &'a Workbook,
    candidate: &'a Workbook,
    edits: &'a [(String, Edit)],
    pending_rows: &'a [(usize, u32)],
    candidate_by_uid: HashMap<u64, &'a Sheet>,
}

impl Changes<'_> {
    fn reference(&self, text: &str, host: &str, wb: &Workbook) -> Checked<(Expr, u64, gridcraft_core::CellRef)> {
        require(text.len() <= 1024)?;
        let expr = gridcraft_formula::parse(text).map_err(|_| unsupported())?;
        let Expr::Ref(r) = &expr else { return Err(unsupported()) };
        let RefKind::Cell(a) = r.kind else { return Err(unsupported()) };
        let name = match &r.sheet {
            SheetSel::Current => host,
            SheetSel::Named(n) => n,
            _ => return Err(unsupported()),
        };
        require(!name.contains(['[', ']']))?;
        let sh = wb.sheet_index(name).and_then(|i| wb.sheet(i)).ok_or_else(unsupported)?;
        require(a.row < gridcraft_core::MAX_ROWS && a.col < gridcraft_core::MAX_COLS)?;
        Ok((expr, sh.uid.get(), a.cell()))
    }

    fn rewrite(&self, text: &str, before: &Sheet, after: &Sheet) -> Checked<String> {
        let (expr, target, _) = self.reference(text, &before.name, self.before)?;
        require(self.candidate_by_uid.contains_key(&target))?;
        let mut adjusted = expr.clone();
        for (target, edit) in self.edits {
            adjusted = gridcraft_formula::adjust::adjust(adjusted, &before.name, target, edit);
        }
        for old in &self.before.sheets {
            if let Some(new) = self.candidate_by_uid.get(&old.uid.get())
                && old.name != new.name
            {
                adjusted = gridcraft_formula::adjust::rename_sheet(adjusted, &old.name, &new.name);
            }
        }
        let result = if adjusted == expr { text.to_string() } else { gridcraft_formula::print(&adjusted) };
        self.reference(&result, &after.name, self.candidate)?;
        Ok(result)
    }

    fn owner_edited(&self, owner: &Sheet) -> bool {
        self.edits.iter().any(|(target, e)| {
            target.eq_ignore_ascii_case(&owner.name) || matches!(e, Edit::MoveToSheet { to_sheet, .. } if to_sheet.eq_ignore_ascii_case(&owner.name))
        })
    }
}

fn metadata_changed(before: &Workbook, after: &Workbook) -> bool {
    if before.names != after.names || before.sheets.len() != after.sheets.len() {
        return true;
    }
    before.sheets.iter().zip(&after.sheets).any(|(a, b)| {
        a.uid.get() != b.uid.get()
            || a.name != b.name
            || a.tables.len() != b.tables.len()
            || a.tables.iter().zip(&b.tables).any(|(a, b)| {
                a.id != b.id || a.name != b.name || a.range != b.range || !a.columns.iter().map(|c| &c.name).eq(b.columns.iter().map(|c| &c.name))
            })
    })
}

fn metrics_changed(before: &Sheet, after: &Sheet) -> bool {
    before.default_row_height != after.default_row_height
        || before.default_col_width != after.default_col_width
        || before.rows.keys().chain(after.rows.keys()).any(|&r| before.row_height(r) != after.row_height(r))
        || before.cols.keys().chain(after.cols.keys()).any(|&c| before.col_width(c) != after.col_width(c))
}

fn bounded(p: &Preserved) -> Checked<()> {
    require(
        p.parts.len() <= super::MAX_KEPT_PARTS
            && p.rels.len() <= super::MAX_RELS
            && p.package_rels.len() <= super::MAX_RELS
            && p.elements.len() <= super::MAX_RELS,
    )?;
    let mut names = std::collections::HashSet::new();
    for part in p.parts.iter().chain(p.vml.iter()) {
        require(
            part.name.len() <= 8192
                && part.data.len() <= super::MAX_KEPT_PART as usize
                && part.rels.len() <= super::MAX_RELS
                && names.insert(normalize(&part.name)),
        )?;
    }
    for rel in p.rels.iter().chain(&p.package_rels).chain(p.parts.iter().chain(p.vml.iter()).flat_map(|part| &part.rels)) {
        require(!rel.id.is_empty() && rel.id.len() <= 1024 && rel.rel_type.len() <= 1024 && rel.target.len() <= 8192)?;
    }
    require(p.elements.iter().all(|e| e.xml.len() <= super::MAX_FRAGMENT))
}

/// Returns a fully prepared candidate only when preserved bytes change. All refusals happen
/// before this returns; callers must invoke it before calculation, edit logs or publication.
pub fn prepare_preserved_edit(
    before: &Workbook,
    candidate: &Workbook,
    edits: &[(String, Edit)],
    pending_auto_height: &[(usize, u32)],
) -> Checked<Option<Workbook>> {
    if before.preserved.is_none() && candidate.preserved.is_none() && before.sheets.iter().chain(&candidate.sheets).all(|s| s.preserved.is_none()) {
        return Ok(None);
    }
    let address_change = !edits.is_empty() || metadata_changed(before, candidate);
    let changes = Changes {
        before,
        candidate,
        edits,
        pending_rows: pending_auto_height,
        candidate_by_uid: candidate.sheets.iter().map(|s| (s.uid.get(), s.as_ref())).collect(),
    };
    if address_change && let Some(p) = before.preserved.as_deref() {
        bounded(p)?;
        // Import retains these generated metadata fragments without cell references. Anything
        // else at workbook/package scope may refer to any sheet, regardless of relationship owner.
        require(
            p.parts.is_empty()
                && p.rels.is_empty()
                && p.package_rels.is_empty()
                && p.vml.is_none()
                && p.sheets.is_empty()
                && p.elements.iter().all(|e| e.slot == "fileVersion" || e.slot == "ext"),
        )?;
        // Known metadata still needs exact schema proof, not merely a trusted slot label.
        for e in &p.elements {
            metadata_fragment(e)?;
        }
    }
    let mut prepared = None;
    let mut total = candidate.preserved.as_deref().map_or(0, Preserved::byte_len);
    let mut owned_parts = std::collections::HashSet::new();
    let before_by_uid: HashMap<_, _> = before.sheets.iter().map(|s| (s.uid.get(), s.as_ref())).collect();
    for (index, after) in candidate.sheets.iter().enumerate() {
        let Some(prior) = before_by_uid.get(&after.uid.get()) else {
            require(after.preserved.as_deref().is_none_or(Preserved::is_empty))?;
            continue;
        };
        let Some(p) = prior.preserved.as_deref() else { continue };
        let visual = p.vml.is_some() || p.elements.iter().any(|e| matches!(e.slot.as_str(), "controls" | "oleObjects" | "legacyDrawingHF"));
        // auto_row_height returns immediately for custom rows, even for unknown object schemas.
        let geometry_risk = visual
            && (metrics_changed(prior, after)
                || pending_auto_height.iter().any(|&(i, row)| i == index && !after.rows.get(&row).is_some_and(|line| line.custom)));
        let updated = if address_change || geometry_risk {
            bounded(p)?;
            require(prior.preserved == after.preserved)?;
            // A shared source part is not independent per-sheet data. The initial handlers
            // require one surviving owner rather than guessing how to split its identities.
            for part in p.parts.iter().chain(p.vml.iter()) {
                require(owned_parts.len() < super::MAX_KEPT_PARTS && owned_parts.insert(normalize(&part.name)))?;
            }
            if p.is_empty() {
                None
            } else if p.vml.is_some() {
                controls::prepare(p, prior, after, index, &changes)?
            } else {
                mappings::prepare(p, prior, after, &changes)?
            }
        } else {
            None
        };
        total = total.saturating_add(updated.as_ref().unwrap_or(p).byte_len());
        require(total <= super::MAX_KEPT_TOTAL)?;
        if let Some(p) = updated {
            let wb = prepared.get_or_insert_with(|| candidate.clone());
            let sh = wb.sheet_mut(index).ok_or_else(unsupported)?;
            sh.preserved = Some(Arc::new(p));
        }
    }
    Ok(prepared)
}

fn metadata_fragment(e: &gridcraft_model::PreservedXml) -> Checked<()> {
    xml::scan(e.xml.as_bytes(), true, |path, token| match token {
        xml::Token::Start(tag) if e.slot == "fileVersion" && xml::path_is(path, &[(MAIN, "fileVersion")]) => {
            tag.only(&super::FILE_VERSION_ATTRS.iter().map(|n| ("", *n)).collect::<Vec<_>>())
        }
        xml::Token::Start(tag) if e.slot == "ext" && xml::path_is(path, &[(MAIN, "ext")]) => {
            tag.only(&[("", "uri")])?;
            require(tag.value("uri")? == super::CALC_FEATURES_URI)
        }
        xml::Token::Start(tag) if xml::path_is(path, &[(MAIN, "ext"), (super::NS_CALC_FEATURES, "calcFeatures")]) => tag.only(&[]),
        xml::Token::Start(tag)
            if xml::path_is(path, &[(MAIN, "ext"), (super::NS_CALC_FEATURES, "calcFeatures"), (super::NS_CALC_FEATURES, "feature")]) =>
        {
            tag.only(&[("", "name")])
        }
        xml::Token::Text(f) => require(f.value.trim().is_empty()),
        xml::Token::End => Ok(()),
        _ => Err(unsupported()),
    })
}
