//! Single XML cells have an owner-local A1 coordinate, not a formula or a query-table range.

use std::collections::HashSet;
use std::sync::Arc;

use gridcraft_model::{Preserved, Sheet};

use super::xml::{self, Patch, Token};
use super::{Changes, Checked, MAIN, REL, require, unsupported};

pub(super) fn prepare(p: &Preserved, before: &Sheet, after: &Sheet, changes: &Changes<'_>) -> Checked<Option<Preserved>> {
    require(
        p.package_rels.is_empty()
            && p.sheets.is_empty()
            && p.elements.is_empty()
            && p.vml.is_none()
            && !p.rels.is_empty()
            && p.rels.len() == p.parts.len(),
    )?;
    let mut ids = HashSet::new();
    let mut targets = HashSet::new();
    let mut result = p.clone();
    let mut changed = false;
    for rel in &p.rels {
        require(
            !rel.external
                && rel.rel_type == format!("{REL}/tableSingleCells")
                && ids.insert(&rel.id)
                && targets.insert(super::normalize(&rel.target)),
        )?;
        let index = p.parts.iter().position(|p| super::normalize(&p.name) == super::normalize(&rel.target)).ok_or_else(unsupported)?;
        let part = p.parts.get(index).ok_or_else(unsupported)?;
        require(part.rels.is_empty() && part.content_type == "application/vnd.openxmlformats-officedocument.spreadsheetml.tableSingleCells+xml")?;
        let mut patches = Vec::new();
        let mut cell_ids = HashSet::new();
        let mut cell_properties = false;
        let mut mapping = false;
        xml::scan(&part.data, false, |path, token| {
            require(path.iter().all(|n| n.ns == MAIN))?;
            let names: Vec<_> = path.iter().map(|n| n.local.as_str()).collect();
            match token {
                Token::Start(tag) => match names.as_slice() {
                    ["singleXmlCells"] => tag.only(&[])?,
                    ["singleXmlCells", "singleXmlCell"] => {
                        tag.only(&[("", "id"), ("", "r"), ("", "connectionId")])?;
                        let id: u32 = tag.value("id")?.parse().map_err(|_| unsupported())?;
                        require(cell_ids.len() < super::super::MAX_RELS && cell_ids.insert(id))?;
                        if let Some(f) = tag.field("", "connectionId") {
                            f.value.parse::<u32>().map_err(|_| unsupported())?;
                        }
                        cell_properties = false;
                        mapping = false;
                        let field = tag.field("", "r").ok_or_else(unsupported)?;
                        let cell = gridcraft_core::CellRef::parse(&field.value).ok_or_else(unsupported)?;
                        require(cell.a1() == field.value)?;
                        let value = changes.rewrite(&field.value, before, after)?;
                        // r cannot represent a different owner, a deleted cell, or a formula.
                        let destination = gridcraft_core::CellRef::parse(&value).ok_or_else(unsupported)?;
                        require(destination.a1() == value)?;
                        if value != field.value {
                            patches.push(Patch { span: field.span.clone(), value });
                        }
                    }
                    ["singleXmlCells", "singleXmlCell", "xmlCellPr"] => {
                        require(!cell_properties)?;
                        cell_properties = true;
                        tag.only(&[("", "id"), ("", "uniqueName")])?;
                        tag.value("id")?.parse::<u32>().map_err(|_| unsupported())?;
                    }
                    ["singleXmlCells", "singleXmlCell", "xmlCellPr", "xmlPr"] => {
                        require(!mapping)?;
                        mapping = true;
                        tag.only(&[("", "mapId"), ("", "xpath"), ("", "xmlDataType")])?;
                        tag.value("mapId")?.parse::<u32>().map_err(|_| unsupported())?;
                        require(!tag.value("xpath")?.is_empty() && matches!(tag.value("xmlDataType")?, "string" | "integer" | "boolean"))?;
                    }
                    _ => return Err(unsupported()),
                },
                Token::Text(f) => require(f.value.trim().is_empty())?,
                Token::End if names.as_slice() == ["singleXmlCells", "singleXmlCell"] => require(cell_properties && mapping)?,
                Token::End => {}
            }
            Ok(())
        })?;
        require(!cell_ids.is_empty())?;
        if let Some(bytes) = xml::patch(&part.data, patches)? {
            result.parts.get_mut(index).ok_or_else(unsupported)?.data = Arc::from(bytes);
            changed = true;
        }
    }
    Ok(changed.then_some(result))
}
