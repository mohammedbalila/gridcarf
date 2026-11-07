//! The deliberately small static-checkbox graph; geometry is validated, never rewritten.

use std::collections::HashSet;
use std::sync::Arc;

use gridcraft_model::{Preserved, Sheet};

use super::xml::{self, Field, Patch, Token};
use super::{Changes, Checked, DRAWING, EXCEL, MAIN, REL, VML, X14, require, unsupported};

#[derive(Default)]
struct Control {
    id: u64,
    rel: String,
    link: Option<Field>,
    markers: [Option<u32>; 8],
    marker_elements: usize,
    sides: usize,
    properties: bool,
    anchor: bool,
}

#[derive(Default)]
struct Shape {
    id: u64,
    link: Option<Field>,
    anchor: Option<[u32; 8]>,
    fields: HashSet<String>,
}

fn number(text: &str) -> Checked<u32> {
    text.parse().map_err(|_| unsupported())
}
fn boolean(text: &str) -> Checked<()> {
    require(matches!(text, "1" | "0" | "true" | "false"))
}
fn text_elsewhere(f: &Field) -> Checked<()> {
    require(f.value.trim().is_empty())
}

fn current<T>(values: &mut [T]) -> Checked<&mut T> {
    values.last_mut().ok_or_else(unsupported)
}

fn entries(bytes: &[u8]) -> Checked<Vec<Control>> {
    let mut entries = Vec::new();
    xml::scan(bytes, true, |path, token| {
        let names: Vec<_> = path.iter().map(|n| n.local.as_str()).collect();
        for (i, name) in path.iter().enumerate() {
            require(name.ns == if i == 5 { DRAWING } else { MAIN })?;
        }
        match token {
            Token::Start(tag) => match names.as_slice() {
                ["controls"] => tag.only(&[])?,
                ["controls", "control"] => {
                    tag.only(&[("", "shapeId"), (REL, "id"), ("", "name")])?;
                    require(entries.len() < super::super::MAX_RELS)?;
                    let id = tag.value("shapeId")?.parse().map_err(|_| unsupported())?;
                    let rel = tag.field(REL, "id").ok_or_else(unsupported)?.value.clone();
                    entries.push(Control { id, rel, ..Default::default() });
                }
                ["controls", "control", "controlPr"] => {
                    let c = current(&mut entries)?;
                    require(!c.properties)?;
                    c.properties = true;
                    tag.only(&[("", "linkedCell"), ("", "defaultSize"), ("", "autoPict"), ("", "print"), ("", "locked")])?;
                    c.link = tag.field("", "linkedCell").cloned();
                    for (n, f) in &tag.attrs {
                        if n.local != "linkedCell" {
                            boolean(&f.value)?;
                        }
                    }
                }
                ["controls", "control", "controlPr", "anchor"] => {
                    let c = current(&mut entries)?;
                    require(!c.anchor)?;
                    c.anchor = true;
                    tag.only(&[("", "moveWithCells"), ("", "sizeWithCells")])?;
                    for (_, f) in &tag.attrs {
                        boolean(&f.value)?;
                    }
                }
                ["controls", "control", "controlPr", "anchor", side @ ("from" | "to")] => {
                    tag.only(&[])?;
                    let c = current(&mut entries)?;
                    let side = usize::from(*side == "to");
                    require(c.sides == side && c.marker_elements == side * 4)?;
                    c.sides += 1;
                }
                ["controls", "control", "controlPr", "anchor", "from" | "to", component @ ("col" | "row" | "colOff" | "rowOff")] => {
                    tag.only(&[])?;
                    let c = current(&mut entries)?;
                    require(["col", "colOff", "row", "rowOff"].get(c.marker_elements % 4) == Some(component))?;
                    require(c.marker_elements < c.sides * 4)?;
                    c.marker_elements += 1;
                }
                _ => return Err(unsupported()),
            },
            Token::Text(f) => {
                if let ["controls", "control", "controlPr", "anchor", side, component] = names.as_slice() {
                    let index = match *component {
                        "col" => 0,
                        "colOff" => 1,
                        "row" => 2,
                        "rowOff" => 3,
                        _ => return Err(unsupported()),
                    } + if *side == "to" { 4 } else { 0 };
                    let slot = current(&mut entries)?.markers.get_mut(index).ok_or_else(unsupported)?;
                    require(slot.is_none())?;
                    *slot = Some(number(f.value.trim())?);
                } else {
                    text_elsewhere(&f)?;
                }
            }
            Token::End => {}
        }
        Ok(())
    })?;
    let mut ids = HashSet::new();
    let mut rels = HashSet::new();
    for c in &entries {
        require(ids.insert(c.id) && rels.insert(&c.rel) && c.properties && c.anchor && c.markers.iter().all(Option::is_some))?;
        geometry(c.markers.map(|v| v.unwrap_or(u32::MAX)))?;
    }
    require(!entries.is_empty())?;
    Ok(entries)
}

fn geometry(a: [u32; 8]) -> Checked<()> {
    require(
        a[0] <= a[4] && a[2] <= a[6] && a[4] < gridcraft_core::MAX_COLS && a[6] < gridcraft_core::MAX_ROWS && [1, 3, 5, 7].iter().all(|&i| a[i] == 0),
    )
}

fn style(text: &str) -> Checked<()> {
    let mut seen = HashSet::new();
    for part in text.split(';').filter(|s| !s.trim().is_empty()) {
        let (name, value) = part.trim().split_once(':').ok_or_else(unsupported)?;
        require(seen.insert(name))?;
        if name == "position" {
            require(value == "absolute")?;
        } else {
            require(matches!(name, "margin-left" | "margin-top" | "width" | "height"))?;
            let value: f64 = value.strip_suffix("pt").ok_or_else(unsupported)?.parse().map_err(|_| unsupported())?;
            require(value.is_finite() && (0.0..=1_000_000.0).contains(&value))?;
        }
    }
    require(seen.len() == 5)
}

fn shapes(bytes: &[u8]) -> Checked<Vec<Shape>> {
    let mut shapes = Vec::new();
    xml::scan(bytes, false, |path, token| {
        let names: Vec<_> = path.iter().map(|n| n.local.as_str()).collect();
        for (i, name) in path.iter().enumerate() {
            require(
                name.ns
                    == match i {
                        0 => "",
                        1 => VML,
                        _ => EXCEL,
                    },
            )?;
        }
        match token {
            Token::Start(tag) => match names.as_slice() {
                ["xml"] => tag.only(&[])?,
                ["xml", "shape"] => {
                    tag.only(&[("", "id"), ("", "style")])?;
                    style(tag.value("style")?)?;
                    let id = tag.value("id")?.strip_prefix("_x0000_s").ok_or_else(unsupported)?.parse().map_err(|_| unsupported())?;
                    require(shapes.len() < super::super::MAX_RELS)?;
                    shapes.push(Shape { id, ..Default::default() });
                }
                ["xml", "shape", "ClientData"] => {
                    tag.only(&[("", "ObjectType")])?;
                    require(tag.value("ObjectType")? == "Checkbox" && current(&mut shapes)?.fields.insert("ClientData".into()))?;
                }
                ["xml", "shape", "ClientData", "Anchor" | "FmlaLink" | "MoveWithCells" | "SizeWithCells"] => {
                    tag.only(&[])?;
                    let field = names.last().ok_or_else(unsupported)?;
                    require(current(&mut shapes)?.fields.insert((*field).into()))?;
                }
                _ => return Err(unsupported()),
            },
            Token::Text(f) => match names.as_slice() {
                ["xml", "shape", "ClientData", "FmlaLink"] => {
                    let sh = current(&mut shapes)?;
                    require(sh.link.is_none())?;
                    sh.link = Some(f);
                }
                ["xml", "shape", "ClientData", "Anchor"] => {
                    let mut components = f.value.split(',');
                    let mut values = [0; 8];
                    for value in &mut values {
                        *value = number(components.next().ok_or_else(unsupported)?.trim())?;
                    }
                    require(components.next().is_none())?;
                    geometry(values)?;
                    let sh = current(&mut shapes)?;
                    require(sh.anchor.is_none())?;
                    sh.anchor = Some(values);
                }
                ["xml", "shape", "ClientData", "MoveWithCells" | "SizeWithCells"] => {
                    require(matches!(f.value.trim(), "" | "1" | "0" | "true" | "false" | "True" | "False"))?;
                }
                _ => text_elsewhere(&f)?,
            },
            Token::End => {}
        }
        Ok(())
    })?;
    let mut ids = HashSet::new();
    require(!shapes.is_empty() && shapes.iter().all(|s| ids.insert(s.id) && s.anchor.is_some() && s.link.is_some()))?;
    Ok(shapes)
}

fn property(bytes: &[u8]) -> Checked<Field> {
    let mut link = None;
    xml::scan(bytes, false, |path, token| {
        require(xml::path_is(path, &[(X14, "formControlPr")]))?;
        match token {
            Token::Start(tag) => {
                tag.only(&[("", "objectType"), ("", "fmlaLink"), ("", "lockText"), ("", "checked")])?;
                require(tag.value("objectType")? == "CheckBox")?;
                if let Some(f) = tag.field("", "lockText") {
                    boolean(&f.value)?;
                }
                if let Some(f) = tag.field("", "checked") {
                    require(matches!(f.value.as_str(), "Checked" | "Unchecked" | "Mixed"))?;
                }
                link = Some(tag.field("", "fmlaLink").ok_or_else(unsupported)?.clone());
            }
            Token::Text(f) => text_elsewhere(&f)?,
            Token::End => {}
        }
        Ok(())
    })?;
    link.ok_or_else(unsupported)
}

fn replacement(field: &Field, before: &Sheet, after: &Sheet, changes: &Changes<'_>, attribute: bool) -> Checked<Option<Patch>> {
    let value = changes.rewrite(&field.value, before, after)?;
    // Text links use a deliberately entity-free lexical subset. Do not write a new link
    // that a subsequent structural edit could no longer validate in this same handler.
    require(attribute || !value.contains(['&', '<', '>']))?;
    Ok((value != field.value)
        .then(|| Patch { span: field.span.clone(), value: if attribute { crate::xml::esc_attr(&value).replace('\'', "&apos;") } else { value } }))
}

pub(super) fn prepare(p: &Preserved, before: &Sheet, after: &Sheet, index: usize, changes: &Changes<'_>) -> Checked<Option<Preserved>> {
    require(p.package_rels.is_empty() && p.sheets.is_empty() && p.elements.len() == 1)?;
    let fragment = p.elements.first().ok_or_else(unsupported)?;
    require(fragment.slot == "controls" && !changes.owner_edited(before) && !super::metrics_changed(before, after))?;
    let entries = entries(fragment.xml.as_bytes())?;
    let vml = p.vml.as_ref().ok_or_else(unsupported)?;
    require(vml.content_type == super::super::CT_VML && vml.rels.is_empty())?;
    let shapes = shapes(&vml.data)?;
    require(entries.len() == shapes.len() && entries.len() == p.rels.len() && entries.len() == p.parts.len())?;
    let mut ids = HashSet::new();
    let mut targets = HashSet::new();
    for rel in &p.rels {
        require(ids.insert(&rel.id) && targets.insert(super::normalize(&rel.target)) && !rel.external && rel.rel_type == format!("{REL}/ctrlProp"))?;
    }
    let bottom = shapes.iter().filter_map(|s| s.anchor.map(|a| a[6])).max().ok_or_else(unsupported)?;
    require(!changes.pending_rows.iter().any(|&(si, row)| si == index && row <= bottom && !after.rows.get(&row).is_some_and(|r| r.custom)))?;
    let mut result = p.clone();
    let mut changed = false;
    let mut fragment_patches = Vec::new();
    let mut vml_patches = Vec::new();
    for entry in entries {
        let shape = shapes.iter().find(|s| s.id == entry.id).ok_or_else(unsupported)?;
        require(shape.anchor == Some(entry.markers.map(|v| v.unwrap_or(u32::MAX))))?;
        let rel = p.rels.iter().find(|r| r.id == entry.rel).ok_or_else(unsupported)?;
        let pi = p.parts.iter().position(|part| super::normalize(&part.name) == super::normalize(&rel.target)).ok_or_else(unsupported)?;
        let part = p.parts.get(pi).ok_or_else(unsupported)?;
        require(part.content_type == "application/vnd.ms-excel.controlproperties+xml" && part.rels.is_empty())?;
        let field = property(&part.data)?;
        let (_, uid, cell) = changes.reference(&field.value, &before.name, changes.before)?;
        for other in shape.link.iter().chain(entry.link.iter()) {
            let (_, other_uid, other_cell) = changes.reference(&other.value, &before.name, changes.before)?;
            require(uid == other_uid && cell == other_cell)?;
        }
        if let Some(patch) = replacement(&field, before, after, changes, true)? {
            let bytes = xml::patch(&part.data, vec![patch])?.ok_or_else(unsupported)?;
            result.parts.get_mut(pi).ok_or_else(unsupported)?.data = Arc::from(bytes);
            changed = true;
        }
        if let Some(field) = &shape.link
            && let Some(patch) = replacement(field, before, after, changes, false)?
        {
            vml_patches.push(patch);
        }
        if let Some(field) = &entry.link
            && let Some(patch) = replacement(field, before, after, changes, true)?
        {
            fragment_patches.push(patch);
        }
    }
    if let Some(bytes) = xml::patch(&vml.data, vml_patches)? {
        result.vml.as_mut().ok_or_else(unsupported)?.data = Arc::from(bytes);
        changed = true;
    }
    if let Some(bytes) = xml::patch(fragment.xml.as_bytes(), fragment_patches)? {
        require(bytes.len() <= super::super::MAX_FRAGMENT)?;
        result.elements.first_mut().ok_or_else(unsupported)?.xml = String::from_utf8(bytes).map_err(|_| unsupported())?;
        changed = true;
    }
    Ok(changed.then_some(result))
}
