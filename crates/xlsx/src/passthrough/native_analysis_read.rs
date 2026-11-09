//! Admission of a complete native worksheet PivotTable and filter-control graph.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use gridcraft_core::{MAX_COLS, MAX_ROWS, RangeRef};
use gridcraft_model::{PreservedAnalysisOwner, PreservedAnalysisSheet, PreservedNativeAnalysis, PreservedPart, PreservedRel, PreservedXml};
use quick_xml::Reader;
use quick_xml::events::Event;

use super::read::{raw_rels, with_decls};
use super::references::xml::{Failure, Name, RecoveryToken, Tag, scan_recovery};
use super::{KeptReader, MAX_FRAGMENT, MAX_RELS, NS_REL_STRICT, is_kind, rels_part_name};
use crate::package::{self, normalize};
use crate::pivot::{CT_PIVOT_CACHE_DEF, CT_PIVOT_CACHE_REC, CT_PIVOT_TABLE};
use crate::read::Ctx;
use crate::write::{NS_MAIN, NS_REL};
use crate::xml;

const X14: &str = "http://schemas.microsoft.com/office/spreadsheetml/2009/9/main";
const X15: &str = "http://schemas.microsoft.com/office/spreadsheetml/2010/11/main";
const XDR: &str = "http://schemas.openxmlformats.org/drawingml/2006/spreadsheetDrawing";
const DRAW: &str = "http://schemas.openxmlformats.org/drawingml/2006/main";
const MC: &str = "http://schemas.openxmlformats.org/markup-compatibility/2006";
const PIVOT_DEFAULT: &str = "http://schemas.microsoft.com/office/spreadsheetml/2016/pivotdefaultlayout";
const SLICER_DRAW: &str = "http://schemas.microsoft.com/office/drawing/2010/slicer";
const TIMELINE_DRAW: &str = "http://schemas.microsoft.com/office/drawing/2012/timeslicer";
const RELS: &str = "http://schemas.openxmlformats.org/package/2006/relationships";
const XML_NS: &str = "http://www.w3.org/XML/1998/namespace";
const PIVOT: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/pivotTable";
const CACHE: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/pivotCacheDefinition";
const RECORDS: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/pivotCacheRecords";
const STYLES: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles";
const SHEET: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet";
const CHART_SHEET: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/chartsheet";
const CONTROL_KINDS: &[&str] = &["slicer", "slicerCache", "timeline", "timelineCache"];
const TABLE: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/table";
const DRAWING: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/drawing";
const SLICER: &str = "http://schemas.microsoft.com/office/2007/relationships/slicer";
const SLICER_CACHE: &str = "http://schemas.microsoft.com/office/2007/relationships/slicerCache";
const TIMELINE: &str = "http://schemas.microsoft.com/office/2011/relationships/timeline";
const TIMELINE_CACHE: &str = "http://schemas.microsoft.com/office/2011/relationships/timelineCache";
const CACHE_EXTENSION: &str = "{725AE2AE-9491-48be-B2B4-4EB974FC3084}";
const MAX_BYTES: usize = 32 * 1024 * 1024;
const MAX_PARTS: usize = 256;
const MAX_NODES: usize = 100_000;

struct Mount {
    uri: &'static str,
    ns: &'static str,
    list: &'static str,
    entry: &'static str,
    rel: &'static str,
}

const WORKBOOK_MOUNTS: &[Mount] = &[
    Mount { uri: "{BBE1A952-AA13-448e-AADC-164F8A28A991}", ns: X14, list: "slicerCaches", entry: "slicerCache", rel: SLICER_CACHE },
    Mount { uri: "{D0CA8CA8-9F24-4464-BF8E-62219DCF47F9}", ns: X15, list: "timelineCacheRefs", entry: "timelineCacheRef", rel: TIMELINE_CACHE },
];
const SHEET_MOUNTS: &[Mount] = &[
    Mount { uri: "{A8765BA9-456A-4dab-B4F3-ACF838C121DE}", ns: X14, list: "slicerList", entry: "slicer", rel: SLICER },
    Mount { uri: "{7E03D99C-DC04-49d9-9315-930204A7B6E9}", ns: X15, list: "timelineRefs", entry: "timelineRef", rel: TIMELINE },
];

/// Identifies the generic reader's report entries for extensions this graph retained.
pub(super) fn is_mount_issue(feature: &str, location: &str) -> bool {
    let Some((_, suffix)) = location.rsplit_once(": ") else { return false };
    match feature {
        "workbook extension" => WORKBOOK_MOUNTS.iter().any(|mount| suffix.eq_ignore_ascii_case(mount.uri)),
        "worksheet extension" => suffix
            .strip_suffix(')')
            .and_then(|suffix| suffix.split_once(" ("))
            .is_some_and(|(list, uri)| SHEET_MOUNTS.iter().any(|mount| list == mount.list && uri.eq_ignore_ascii_case(mount.uri))),
        _ => false,
    }
}

type Checked<T> = Result<T, String>;

fn require(ok: bool, why: &str) -> Checked<()> {
    if ok { Ok(()) } else { Err(why.into()) }
}

struct Node {
    name: Name,
    tag: Tag,
    end: usize,
    children: Vec<Node>,
    text: String,
}

impl Node {
    fn is(&self, ns: &str, local: &str) -> bool {
        self.name.ns == ns && self.name.local == local
    }

    fn attr(&self, ns: &str, local: &str) -> Option<&str> {
        self.tag.field(ns, local).map(|f| f.value.as_str())
    }

    fn value(&self, local: &str) -> Checked<&str> {
        self.attr("", local).filter(|value| !value.is_empty()).ok_or_else(|| format!("{} has no {local}", self.name.local))
    }

    fn number(&self, local: &str) -> Checked<u32> {
        self.value(local)?.parse().map_err(|_| format!("{} has an invalid {local}", self.name.local))
    }

    fn optional(&self, ns: &str, local: &str) -> Checked<Option<&Node>> {
        let mut matching = self.children.iter().filter(|child| child.is(ns, local));
        let child = matching.next();
        require(matching.next().is_none(), &format!("duplicate {local}"))?;
        Ok(child)
    }

    fn child(&self, ns: &str, local: &str) -> Checked<&Node> {
        self.optional(ns, local)?.ok_or_else(|| format!("{} has no {local}", self.name.local))
    }

    fn visit(&self, f: &mut impl FnMut(&Node) -> Checked<()>) -> Checked<()> {
        f(self)?;
        for child in &self.children {
            child.visit(f)?;
        }
        Ok(())
    }

    fn empty(&self) -> bool {
        self.children.is_empty() && self.text.trim().is_empty()
    }

    fn independent(&self, allowed_rel: Option<&Node>) -> Checked<()> {
        self.visit(&mut |node| {
            for (name, field) in &node.tag.attrs {
                let relationship = matches!(name.ns.as_str(), NS_REL | NS_REL_STRICT);
                require(
                    !relationship || allowed_rel.is_some_and(|owner| std::ptr::eq(owner, node)) && name.ns == NS_REL && name.local == "id",
                    "unexpected relationship attribute",
                )?;
                require(!(name.ns == XML_NS && name.local == "base"), "unexpected XML base URI")?;
                require(
                    !matches!(name.local.as_str(), "macro" | "textlink") || field.value.is_empty(),
                    "a native analysis object depends on a macro or external text",
                )?;
            }
            Ok(())
        })
    }
}

/// Use the existing namespace-aware bounded scanner, but retain just the selected metadata subtrees.
fn document(bytes: &[u8], selected: impl Fn(&Name) -> bool) -> Checked<Node> {
    let mut stack: Vec<Option<Node>> = Vec::new();
    let mut root = None;
    let mut nodes = 0;
    scan_recovery(bytes, false, false, |path, token| {
        match token {
            RecoveryToken::Start(tag) => {
                let name = path.last().ok_or(Failure::Malformed)?;
                let keep = match stack.as_slice() {
                    [] => true,
                    [_] => selected(name),
                    _ => stack.last().is_some_and(Option::is_some),
                };
                if keep {
                    nodes += 1;
                    if nodes > MAX_NODES {
                        return Err(Failure::Limit);
                    }
                }
                stack.push(keep.then(|| Node { name: name.clone(), tag, end: 0, children: vec![], text: String::new() }));
            }
            RecoveryToken::Text(text, _) => {
                if let Some(Some(node)) = stack.last_mut() {
                    node.text.push_str(text);
                }
            }
            RecoveryToken::Reference(c) => {
                if let Some(Some(node)) = stack.last_mut() {
                    node.text.push(c);
                }
            }
            RecoveryToken::End(span) => {
                if let Some(mut node) = stack.pop().ok_or(Failure::Malformed)? {
                    node.end = span.end;
                    if let Some(Some(parent)) = stack.last_mut() {
                        parent.children.push(node);
                    } else {
                        root = Some(node);
                    }
                }
            }
        }
        Ok(())
    })
    .map_err(|error| format!("native analysis XML is malformed or outside its bounds ({error:?})"))?;
    root.ok_or_else(|| "missing native analysis XML root".into())
}

fn read_bytes(cx: &mut Ctx<'_>, name: &str, limit: usize) -> Checked<Vec<u8>> {
    cx.pkg.read_optional_limited(name, limit as u64).map_err(|e| e.to_string())?.ok_or_else(|| format!("missing {name}"))
}

fn root_is(root: &Node, ns: &str, name: &str) -> Checked<()> {
    require(root.is(ns, name), &format!("expected {name} in {ns}"))
}

fn content_type(kept: &KeptReader, name: &str, expected: &str) -> Checked<()> {
    require(kept.content_type(name).as_deref() == Some(expected), &format!("{name} does not have its expected content type"))
}

fn standard_type(name: &str) -> String {
    format!("application/vnd.openxmlformats-officedocument.spreadsheetml.{name}+xml")
}

/// Strict relationship inventory for graph admission. The generic pass-through reader tolerates
/// malformed rows to salvage independent owners, while this group requires complete closure.
fn relationships(cx: &mut Ctx<'_>, owner: &str, budget: &mut usize) -> Checked<Vec<PreservedRel>> {
    let name = rels_part_name(owner);
    if !cx.pkg.has(&name) {
        return Ok(vec![]);
    }
    let bytes = part_bytes(cx, &name, MAX_FRAGMENT, budget)?;
    let root = document(&bytes, |_| true)?;
    root_is(&root, RELS, "Relationships")?;
    require(root.children.len() <= MAX_RELS, "too many relationships")?;
    let mut ids = HashSet::new();
    let (dir, _) = package::split_dir(owner);
    root.children
        .iter()
        .map(|node| {
            root_is(node, RELS, "Relationship")?;
            require(node.empty(), "invalid relationship children")?;
            node.tag.only(&[("", "Id"), ("", "Type"), ("", "Target"), ("", "TargetMode")])?;
            let id = node.value("Id")?;
            require(ids.insert(id), "duplicate relationship ID")?;
            let target = node.value("Target")?;
            let rel_type = node.value("Type")?;
            require(id.len() <= 255 && target.len() <= 4096 && rel_type.len() <= 4096, "relationship metadata is too large")?;
            let external = match node.attr("", "TargetMode") {
                None | Some("Internal") => false,
                Some("External") => true,
                _ => return Err("invalid relationship target mode".into()),
            };
            if !external
                && matches!(
                    rel_type,
                    PIVOT | CACHE | RECORDS | STYLES | SHEET | CHART_SHEET | TABLE | DRAWING | SLICER | SLICER_CACHE | TIMELINE | TIMELINE_CACHE
                )
            {
                require(internal_target(dir, target), "invalid internal analysis relationship target")?;
            }
            Ok(PreservedRel {
                id: id.to_string(),
                rel_type: rel_type.to_string(),
                target: if external { target.to_string() } else { package::resolve(dir, target) },
                external,
            })
        })
        .collect()
}

fn internal_target(dir: &str, target: &str) -> bool {
    if target.contains(['\\', '#', '?', ':', '%']) || target.chars().any(char::is_whitespace) {
        return false;
    }
    let (path, mut depth) =
        target.strip_prefix('/').map_or_else(|| (target, dir.split('/').filter(|segment| !segment.is_empty()).count()), |path| (path, 0));
    for segment in path.split('/') {
        match segment {
            "" | "." => return false,
            ".." if depth > 0 => depth -= 1,
            ".." => return false,
            _ => depth += 1,
        }
    }
    depth != 0 && !path.ends_with("/..") && path != ".."
}

fn linked<'a>(rels: &'a [PreservedRel], node: &Node, rel_type: &str) -> Checked<&'a PreservedRel> {
    let id = node.attr(NS_REL, "id").ok_or_else(|| format!("missing relationship ID on {}", node.name.local))?;
    rels.iter().find(|rel| rel.id == id && rel.rel_type == rel_type && !rel.external).ok_or_else(|| format!("{id} does not link to {rel_type}"))
}

fn all_mounted(rels: &[PreservedRel], mounted: &[PreservedRel], kinds: &[&str]) -> Checked<()> {
    require(
        rels.iter().filter(|r| is_kind(r, kinds)).all(|r| mounted.iter().any(|entry| entry.id == r.id)),
        "unmounted or foreign analysis relationship",
    )
}

fn part_bytes(cx: &mut Ctx<'_>, name: &str, limit: usize, budget: &mut usize) -> Checked<Vec<u8>> {
    let bytes = read_bytes(cx, name, limit.min(*budget))?;
    *budget -= bytes.len();
    Ok(bytes)
}

/// Retained native parts keep their original names in the generated package. Bound each content
/// type to its own family so an admitted target cannot replace a regenerated part such as styles.
pub(super) fn native_part_path(path: &str, content_type: &str) -> bool {
    let (folder, prefix) = match content_type {
        CT_PIVOT_TABLE => ("xl/pivotTables", "pivotTable"),
        CT_PIVOT_CACHE_DEF => ("xl/pivotCache", "pivotCacheDefinition"),
        CT_PIVOT_CACHE_REC => ("xl/pivotCache", "pivotCacheRecords"),
        "application/vnd.ms-excel.slicer+xml" => ("xl/slicers", "slicer"),
        "application/vnd.ms-excel.slicerCache+xml" => ("xl/slicerCaches", "slicerCache"),
        "application/vnd.ms-excel.timeline+xml" => ("xl/timelines", "timeline"),
        "application/vnd.ms-excel.timelineCache+xml" => ("xl/timelineCaches", "timelineCache"),
        _ => return false,
    };
    let Some((dir, name)) = path.rsplit_once('/') else { return false };
    dir.eq_ignore_ascii_case(folder)
        && name.get(..prefix.len()).is_some_and(|name| name.eq_ignore_ascii_case(prefix))
        && name
            .get(prefix.len()..)
            .and_then(|suffix| suffix.rsplit_once('.'))
            .is_some_and(|(id, ext)| ext.eq_ignore_ascii_case("xml") && !id.is_empty() && id.bytes().all(|byte| byte.is_ascii_digit()))
}

struct Parts<'a> {
    kept: &'a KeptReader,
    bytes_left: usize,
    scan_left: usize,
    parts: Vec<PreservedPart>,
    names: HashSet<String>,
}

impl Parts<'_> {
    fn add(&mut self, cx: &mut Ctx<'_>, link: &PreservedRel, ct: &str) -> Checked<usize> {
        require(self.parts.len() < MAX_PARTS, "too many native analysis parts")?;
        require(!link.external, "external native analysis relationship")?;
        require(native_part_path(&link.target, ct), "native analysis part is outside its standard package directory")?;
        require(self.names.insert(normalize(&link.target)), "a native analysis part has multiple owners")?;
        content_type(self.kept, &link.target, ct)?;
        let data = part_bytes(cx, &link.target, MAX_BYTES, &mut self.bytes_left)?;
        let rels = relationships(cx, &link.target, &mut self.scan_left)?;
        let index = self.parts.len();
        self.parts.push(PreservedPart { name: link.target.clone(), content_type: ct.into(), data: Arc::from(data), rels });
        Ok(index)
    }

    fn part(&self, index: usize) -> Checked<&PreservedPart> {
        self.parts.get(index).ok_or_else(|| "missing native analysis part".into())
    }

    fn charge(&mut self, bytes: usize) -> Checked<()> {
        self.bytes_left = self.bytes_left.checked_sub(bytes).ok_or_else(|| "native analysis exceeds its retained byte limit".to_string())?;
        Ok(())
    }
}

fn decls(bytes: &[u8], ancestors: &[&Node]) -> Checked<Vec<(String, String)>> {
    let mut out = vec![(String::new(), String::new())];
    for node in ancestors {
        require(node.attr(XML_NS, "base").is_none(), "native analysis fragment inherits an XML base URI")?;
        let tag = bytes.get(node.tag.span.clone()).ok_or_else(|| "invalid XML fragment context".to_string())?;
        let mut reader = Reader::from_reader(tag);
        let Event::Start(start) = reader.read_event().map_err(|e| e.to_string())? else { return Err("invalid XML fragment ancestor".into()) };
        for attr in start.attributes() {
            let attr = attr.map_err(|e| e.to_string())?;
            let key = attr.key.as_ref();
            let prefix = if key == b"xmlns" { Some(b"".as_slice()) } else { key.strip_prefix(b"xmlns:") };
            if let Some(prefix) = prefix {
                let prefix = std::str::from_utf8(prefix).map_err(|e| e.to_string())?;
                let value = std::str::from_utf8(&attr.value).map_err(|e| e.to_string())?;
                require(!value.contains('"'), "namespace declaration cannot be copied to the analysis fragment")?;
                out.retain(|(key, _)| key != prefix);
                out.push((prefix.to_string(), value.to_string()));
            }
        }
    }
    Ok(out)
}

fn fragment(bytes: &[u8], node: &Node, ancestors: &[&Node], parts: &mut Parts<'_>) -> Checked<String> {
    let span = node.tag.span.start..node.end;
    require(span.len() <= MAX_FRAGMENT, "native analysis fragment is too large")?;
    let text = std::str::from_utf8(bytes.get(span).ok_or_else(|| "invalid XML fragment span".to_string())?).map_err(|e| e.to_string())?;
    let raw = with_decls(text, &decls(bytes, ancestors)?);
    require(raw.len() <= MAX_FRAGMENT, "native analysis fragment is too large")?;
    parts.charge(raw.len())?;
    // The writer remaps relationship attributes on owner fragments. Reject spellings that
    // require decoding or would change meaning in the regenerated owner.
    scan_recovery(raw.as_bytes(), true, true, |_, _| Ok(())).map_err(|e| format!("invalid native analysis fragment ({e:?})"))?;
    Ok(raw)
}

fn mounts(bytes: &[u8], root: &Node, rels: &[PreservedRel], mounts: &[Mount], parts: &mut Parts<'_>) -> Checked<PreservedAnalysisOwner> {
    let mut owner = PreservedAnalysisOwner::default();
    if let Some(extensions) = root.optional(NS_MAIN, "extLst")? {
        let mut seen = HashSet::new();
        for ext in &extensions.children {
            let Some(mount) = mounts.iter().find(|mount| ext.attr("", "uri").is_some_and(|uri| uri.eq_ignore_ascii_case(mount.uri))) else {
                ext.visit(&mut |node| {
                    require(
                        !WORKBOOK_MOUNTS.iter().chain(SHEET_MOUNTS).any(|mount| node.is(mount.ns, mount.list) || node.is(mount.ns, mount.entry))
                            && !node.attr(NS_REL, "id").is_some_and(|id| rels.iter().any(|r| r.id == id && is_kind(r, CONTROL_KINDS))),
                        "unexpected extension refers to a native control",
                    )
                })?;
                continue;
            };
            root_is(ext, NS_MAIN, "ext")?;
            require(seen.insert(mount.uri), "duplicate analysis extension")?;
            ext.tag.only(&[("", "uri")])?;
            let list = ext.child(mount.ns, mount.list)?;
            require(ext.children.len() == 1 && !list.children.is_empty() && list.tag.attrs.is_empty(), "invalid native analysis extension list")?;
            for entry in &list.children {
                root_is(entry, mount.ns, mount.entry)?;
                require(entry.empty(), "invalid native analysis extension entry")?;
                entry.tag.only(&[(NS_REL, "id")])?;
                let rel = linked(rels, entry, mount.rel)?;
                require(!owner.rels.iter().any(|r| r.id == rel.id), "duplicate analysis mount")?;
                owner.rels.push(rel.clone());
            }
            owner.elements.push(PreservedXml { slot: "ext".into(), xml: fragment(bytes, ext, &[root, extensions], parts)? });
        }
    }
    all_mounted(rels, &owner.rels, CONTROL_KINDS)?;
    Ok(owner)
}

struct CacheField {
    slicer_item_count: usize,
    has_dates: bool,
}

struct Cache {
    id: u32,
    extension_id: u32,
    part: String,
    source: String,
    fields: HashMap<String, CacheField>,
}

fn read_cache(cx: &mut Ctx<'_>, parts: &mut Parts<'_>, link: &PreservedRel, id: u32) -> Checked<Cache> {
    let index = parts.add(cx, link, CT_PIVOT_CACHE_DEF)?;
    let part = parts.part(index)?;
    let root = document(&part.data, |_| true)?;
    root_is(&root, NS_MAIN, "pivotCacheDefinition")?;
    root.independent(Some(&root))?;
    let source = root.child(NS_MAIN, "cacheSource")?;
    require(source.value("type")? == "worksheet" && source.children.len() == 1, "unsupported native pivot cache source")?;
    source.tag.only(&[("", "type")])?;
    let source = source.child(NS_MAIN, "worksheetSource")?;
    source.tag.only(&[("", "name")])?;
    require(source.empty(), "unsupported pivot cache source child")?;
    let source = source.value("name")?.to_string();
    let fields = root.child(NS_MAIN, "cacheFields")?;
    require(fields.number("count")? as usize == fields.children.len() && !fields.children.is_empty(), "invalid pivot cache field count")?;
    let mut field_items = HashMap::new();
    let mut record_items = Vec::new();
    for field in &fields.children {
        root_is(field, NS_MAIN, "cacheField")?;
        let shared = field.optional(NS_MAIN, "sharedItems")?;
        let grouped = field.optional(NS_MAIN, "fieldGroup")?.map(|group| group.optional(NS_MAIN, "groupItems")).transpose()?.flatten();
        for items in [shared, grouped].into_iter().flatten() {
            if let Some(count) = items.attr("", "count") {
                require(count.parse::<usize>().ok() == Some(items.children.len()), "pivot cache item count does not match its field")?;
            }
        }
        record_items.push(shared.map_or(0, |items| items.children.len()));
        let field_info = CacheField {
            slicer_item_count: grouped.or(shared).map_or(0, |items| items.children.len()),
            has_dates: shared.is_some_and(|items| items.children.iter().any(|item| item.is(NS_MAIN, "d"))),
        };
        require(field_items.insert(field.value("name")?.to_string(), field_info).is_none(), "duplicate pivot cache field")?;
    }
    let extensions = root.child(NS_MAIN, "extLst")?;
    let ext = extensions.child(NS_MAIN, "ext")?;
    require(
        extensions.children.len() == 1 && ext.children.len() == 1 && ext.value("uri")?.eq_ignore_ascii_case(CACHE_EXTENSION),
        "unsupported pivot cache extension",
    )?;
    let x14 = ext.child(X14, "pivotCacheDefinition")?;
    let extension_id = x14.number("pivotCacheId")?;
    x14.tag.only(&[("", "pivotCacheId"), ("", "slicerData")])?;
    require(x14.empty() && matches!(x14.attr("", "slicerData"), None | Some("0" | "1" | "false" | "true")), "unsupported x14 pivot cache state")?;
    require(
        root.children.iter().all(|child| child.is(NS_MAIN, "cacheSource") || child.is(NS_MAIN, "cacheFields") || child.is(NS_MAIN, "extLst")),
        "unsupported native worksheet cache dependency",
    )?;
    require(extension_id != 0, "invalid native pivot cache extension ID")?;
    let records_link = linked(&part.rels, &root, RECORDS)?.clone();
    require(part.rels.len() == 1, "pivot cache has an independent dependency")?;
    let record_count = root.number("recordCount")?;
    let records = parts.add(cx, &records_link, CT_PIVOT_CACHE_REC)?;
    let records = parts.part(records)?;
    require(records.rels.is_empty(), "pivot cache records have an independent dependency")?;
    validate_records(&records.data, record_count, &record_items)?;
    Ok(Cache { id, extension_id, part: link.target.clone(), source, fields: field_items })
}

fn validate_records(bytes: &[u8], expected_count: u32, fields: &[usize]) -> Checked<()> {
    let mut count = 0u32;
    let mut width = 0;
    scan_recovery(bytes, false, false, |path, token| {
        let invalid = Failure::Malformed;
        let name = path.last().ok_or(invalid)?;
        if name.ns != NS_MAIN {
            return Err(invalid);
        }
        match token {
            RecoveryToken::Start(tag) => {
                if tag.attrs.iter().any(|(name, _)| !(name.ns.is_empty() || path.len() == 1 && name.ns == MC && name.local == "Ignorable")) {
                    return Err(invalid);
                }
                match path.len() {
                    1 if name.local == "pivotCacheRecords"
                        && tag.value("count").ok().and_then(|value| value.parse().ok()) == Some(expected_count) => {}
                    2 if name.local == "r" => {
                        width = 0;
                        count = count.checked_add(1).ok_or(invalid)?;
                    }
                    3 if matches!(name.local.as_str(), "x" | "n" | "s" | "b" | "d" | "e" | "m") => {
                        let shared_count = fields.get(width).ok_or(invalid)?;
                        if name.local == "x"
                            && tag.value("v").ok().and_then(|index| index.parse::<usize>().ok()).is_none_or(|index| index >= *shared_count)
                        {
                            return Err(invalid);
                        }
                        width += 1;
                    }
                    _ => return Err(invalid),
                }
            }
            RecoveryToken::End(_) if path.len() == 2 && width != fields.len() => return Err(invalid),
            _ => {}
        }
        Ok(())
    })
    .map_err(|_| "invalid native pivot cache records".to_string())?;
    require(count == expected_count, "native pivot cache record count does not match its definition")
}

fn validate_pivot_filters(root: &Node, field_count: usize) -> Checked<()> {
    let Some(filters) = root.optional(NS_MAIN, "filters")? else { return Ok(()) };
    require(filters.number("count")? as usize == filters.children.len(), "invalid native PivotTable filter count")?;
    let data_fields = root.optional(NS_MAIN, "dataFields")?.map_or(0, |fields| fields.children.len());
    let mut ids = HashSet::new();
    for filter in &filters.children {
        root_is(filter, NS_MAIN, "filter")?;
        require(ids.insert(filter.number("id")?), "duplicate native PivotTable filter ID")?;
        require((filter.number("fld")? as usize) < field_count, "native PivotTable filter refers to a missing field")?;
        let kind = filter.value("type")?;
        if filter.attr("", "iMeasureFld").is_some() || matches!(kind, "count" | "percent" | "sum") || kind.starts_with("value") {
            let measure =
                filter.attr("", "iMeasureFld").unwrap_or("0").parse::<usize>().map_err(|_| "invalid native PivotTable filter measure field")?;
            require(measure < data_fields, "native PivotTable filter refers to a missing measure field")?;
        }
    }
    Ok(())
}

fn pivot_name(cx: &mut Ctx<'_>, parts: &mut Parts<'_>, link: &PreservedRel, cache: &Cache) -> Checked<String> {
    let index = parts.add(cx, link, CT_PIVOT_TABLE)?;
    let part = parts.part(index)?;
    let root = document(&part.data, |_| true)?;
    root_is(&root, NS_MAIN, "pivotTableDefinition")?;
    root.independent(None)?;
    root.visit(&mut |node| require(matches!(node.name.ns.as_str(), NS_MAIN | X14 | PIVOT_DEFAULT), "unsupported PivotTable XML dependency"))?;
    require(root.number("cacheId")? == cache.id, "PivotTable uses a different workbook cache ID")?;
    require(
        matches!(part.rels.as_slice(), [r] if !r.external && r.rel_type == CACHE && normalize(&r.target) == normalize(&cache.part)),
        "PivotTable does not link to the shared cache definition",
    )?;
    let location = root.child(NS_MAIN, "location")?;
    require(RangeRef::parse(location.value("ref")?).is_some(), "invalid native PivotTable report range")?;
    let fields = root.child(NS_MAIN, "pivotFields")?;
    require(
        fields.children.len() == cache.fields.len()
            && fields.number("count")? as usize == fields.children.len()
            && fields.children.iter().all(|field| field.is(NS_MAIN, "pivotField")),
        "native PivotTable fields do not match the cache",
    )?;
    validate_pivot_filters(&root, cache.fields.len())?;
    Ok(root.value("name")?.to_string())
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Control {
    Slicer,
    Timeline,
}

impl Control {
    fn namespace(self) -> &'static str {
        match self {
            Self::Slicer => X14,
            Self::Timeline => X15,
        }
    }

    fn builtin_style(self, name: &str) -> bool {
        let prefixes: &[&str] = match self {
            Self::Slicer => &["SlicerStyle"],
            // Excel also writes TimeSlicerStyle, including in the pinned dashboard.
            Self::Timeline => &["TimelineStyle", "TimeSlicerStyle"],
        };
        prefixes.iter().any(|prefix| {
            let Some(style) = name.strip_prefix(*prefix) else { return false };
            if self == Self::Slicer && matches!(style, "Other1" | "Other2") {
                return true;
            }
            style.strip_prefix("Light").or_else(|| style.strip_prefix("Dark")).is_some_and(|index| matches!(index, "1" | "2" | "3" | "4" | "5" | "6"))
        })
    }

    fn names(self) -> (&'static str, &'static str, &'static str) {
        match self {
            Self::Slicer => ("slicer", "slicers", "slicerCacheDefinition"),
            Self::Timeline => ("timeline", "timelines", "timelineCacheDefinition"),
        }
    }
}

type PivotNames = HashMap<u32, HashSet<String>>;

fn control_cache(cx: &mut Ctx<'_>, parts: &mut Parts<'_>, link: &PreservedRel, cache: &Cache, pivots: &PivotNames) -> Checked<(String, Control)> {
    let (control, ct) = match link.rel_type.as_str() {
        SLICER_CACHE => (Control::Slicer, "application/vnd.ms-excel.slicerCache+xml"),
        TIMELINE_CACHE => (Control::Timeline, "application/vnd.ms-excel.timelineCache+xml"),
        _ => return Err("unrecognized analysis cache".into()),
    };
    let ns = control.namespace();
    let index = parts.add(cx, link, ct)?;
    let part = parts.part(index)?;
    require(part.rels.is_empty(), "control cache has an independent dependency")?;
    let root = document(&part.data, |_| true)?;
    root_is(&root, ns, control.names().2)?;
    root.independent(None)?;
    let source = root.value("sourceName")?;
    let field = cache.fields.get(source).ok_or_else(|| format!("missing pivot cache field {source}"))?;
    let connections = root.child(ns, "pivotTables")?;
    require(!connections.children.is_empty(), "unconnected analysis control cache")?;
    let mut connected = HashSet::new();
    for pivot in &connections.children {
        root_is(pivot, ns, "pivotTable")?;
        pivot.tag.only(&[("", "tabId"), ("", "name")])?;
        let tab = pivot.number("tabId")?;
        let name = pivot.value("name")?;
        require(
            pivot.children.is_empty() && pivots.get(&tab).is_some_and(|names| names.contains(name)) && connected.insert((tab, name)),
            "control cache references a missing or duplicate worksheet PivotTable",
        )?;
    }
    let state = match control {
        Control::Slicer => {
            require(root.children.len() == 2, "unsupported slicer cache extension")?;
            let data = root.child(ns, "data")?;
            require(data.children.len() == 1, "unsupported slicer cache source")?;
            let tabular = data.child(ns, "tabular")?;
            let items = tabular.child(ns, "items")?;
            require(tabular.children.len() == 1 && items.children.len() == items.number("count")? as usize, "invalid slicer cache items")?;
            let mut indices = HashSet::new();
            for item in &items.children {
                root_is(item, ns, "i")?;
                let index = item.number("x")? as usize;
                require(
                    item.children.is_empty() && index < field.slicer_item_count && indices.insert(index),
                    "slicer item references a missing or duplicate pivot cache item",
                )?;
            }
            tabular
        }
        Control::Timeline => {
            require(field.has_dates, "timeline cache does not refer to a native date field")?;
            let state = root.child(ns, "state")?;
            require(
                root.children.iter().all(|node| ["pivotTables", "state"].iter().any(|local| node.is(ns, local))),
                "unsupported timeline cache extension",
            )?;
            let bounds = state.child(ns, "bounds")?;
            let selection = state.optional(ns, "selection")?;
            require(state.children.len() == 1 + usize::from(selection.is_some()), "unsupported timeline state")?;
            for range in std::iter::once(bounds).chain(selection) {
                range.tag.only(&[("", "startDate"), ("", "endDate")])?;
                require(range.empty(), "invalid timeline date range")?;
                let start = crate::sheet_read::parse_iso_datetime(range.value("startDate")?, cx.date_system);
                let end = crate::sheet_read::parse_iso_datetime(range.value("endDate")?, cx.date_system);
                require(start.zip(end).is_some_and(|(start, end)| start <= end), "invalid timeline date range")?;
            }
            state
        }
    };
    require(state.number("pivotCacheId")? == cache.extension_id, "control cache uses a different x14 pivot cache ID")?;
    Ok((root.value("name")?.to_string(), control))
}

fn control_names(
    cx: &mut Ctx<'_>,
    parts: &mut Parts<'_>,
    link: &PreservedRel,
    caches: &HashMap<String, Control>,
    controls: &mut HashMap<String, Control>,
    used: &mut HashSet<String>,
) -> Checked<()> {
    let (control, ct) = match link.rel_type.as_str() {
        SLICER => (Control::Slicer, "application/vnd.ms-excel.slicer+xml"),
        TIMELINE => (Control::Timeline, "application/vnd.ms-excel.timeline+xml"),
        _ => return Err("unrecognized worksheet control".into()),
    };
    let ns = control.namespace();
    let (item, list, _) = control.names();
    let index = parts.add(cx, link, ct)?;
    let part = parts.part(index)?;
    require(part.rels.is_empty(), "worksheet control has an independent dependency")?;
    let root = document(&part.data, |_| true)?;
    root_is(&root, ns, list)?;
    root.independent(None)?;
    require(!root.children.is_empty(), "empty native worksheet controls")?;
    for node in &root.children {
        root_is(node, ns, item)?;
        require(node.empty(), "unsupported native control extension")?;
        require(node.attr("", "style").is_none_or(|style| control.builtin_style(style)), "custom native control style cannot be remapped")?;
        let name = node.value("name")?;
        let cache = node.value("cache")?;
        require(caches.get(cache) == Some(&control), "worksheet control does not have its expected cache")?;
        require(controls.insert(name.to_string(), control).is_none(), "duplicate worksheet control name")?;
        used.insert(cache.to_string());
    }
    Ok(())
}

fn table_source(cx: &mut Ctx<'_>, parts: &mut Parts<'_>, link: &PreservedRel, cache: &Cache) -> Checked<bool> {
    content_type(parts.kept, &link.target, &standard_type("table"))?;
    let bytes = part_bytes(cx, &link.target, MAX_FRAGMENT, &mut parts.scan_left)?;
    let table = document(&bytes, |_| true)?;
    root_is(&table, NS_MAIN, "table")?;
    let name = table.value("name")?;
    let display = table.value("displayName")?;
    if !name.eq_ignore_ascii_case(&cache.source) && !display.eq_ignore_ascii_case(&cache.source) {
        return Ok(false);
    }
    require(name == cache.source && display == cache.source, "ambiguous named pivot source table")?;
    table.independent(None)?;
    require(relationships(cx, &link.target, &mut parts.scan_left)?.is_empty(), "pivot source table has an independent dependency")?;
    require(matches!(table.attr("", "tableType"), None | Some("worksheet")), "pivot source table is not a worksheet table")?;
    let range = RangeRef::parse(table.value("ref")?).ok_or_else(|| "invalid pivot source table range".to_string())?;
    let columns = table.child(NS_MAIN, "tableColumns")?;
    require(
        columns.children.len() == range.width() as usize && columns.number("count")? as usize == columns.children.len(),
        "invalid pivot source table columns",
    )?;
    let mut names = HashSet::new();
    for column in &columns.children {
        root_is(column, NS_MAIN, "tableColumn")?;
        let name = column.value("name")?;
        require(names.insert(name.to_ascii_lowercase()) && cache.fields.contains_key(name), "pivot source table column does not match the cache")?;
    }
    Ok(true)
}

fn drawing_control(node: &Node) -> Option<Control> {
    if node.is(SLICER_DRAW, "slicer") {
        Some(Control::Slicer)
    } else if node.is(TIMELINE_DRAW, "timeslicer") {
        Some(Control::Timeline)
    } else {
        None
    }
}

type DrawingPosition = ((u32, u64), (u32, u64));

struct DrawingBounds {
    from: DrawingPosition,
    to: DrawingPosition,
}

impl DrawingBounds {
    fn from_anchor(anchor: &Node) -> Checked<Self> {
        root_is(anchor, XDR, "twoCellAnchor")?;
        let from = marker(anchor.child(XDR, "from")?)?;
        let to = marker(anchor.child(XDR, "to")?)?;
        require(from.0 <= to.0 && from.1 <= to.1, "drawing anchor geometry is reversed")?;
        Ok(Self { from, to })
    }

    fn overlaps(&self, other: &Self) -> bool {
        self.from.0 < other.to.0 && other.from.0 < self.to.0 && self.from.1 < other.to.1 && other.from.1 < self.to.1
    }
}

fn marker(node: &Node) -> Checked<DrawingPosition> {
    let value = |tag| -> Checked<u64> {
        let value = node.child(XDR, tag)?;
        require(value.children.is_empty(), "invalid native anchor position")?;
        value.text.trim().parse().map_err(|_| "invalid native anchor position".into())
    };
    let col = value("col")?;
    let row = value("row")?;
    require(node.children.len() == 4 && col < u64::from(MAX_COLS) && row < u64::from(MAX_ROWS), "native anchor lies outside its worksheet")?;
    Ok(((col as u32, value("colOff")?), (row as u32, value("rowOff")?)))
}

fn anchor_identity<'a>(bytes: &[u8], drawing: &Node, anchor: &'a Node) -> Checked<Option<(&'a str, Control, DrawingBounds)>> {
    let mut candidates = 0;
    anchor.visit(&mut |node| {
        if drawing_control(node).is_some() || node.is(DRAW, "graphicData") && matches!(node.attr("", "uri"), Some(SLICER_DRAW | TIMELINE_DRAW)) {
            candidates += 1;
        }
        Ok(())
    })?;
    if candidates == 0 {
        return Ok(None);
    }
    let bounds = DrawingBounds::from_anchor(anchor)?;
    require(candidates == 2 && anchor.children.len() == 4, "invalid or mixed native control anchor")?;
    anchor.independent(None)?;
    require(anchor.child(XDR, "clientData")?.children.is_empty(), "invalid native anchor client data")?;
    let (frame, choice) = if let Some(alt) = anchor.optional(MC, "AlternateContent")? {
        let choice = alt.child(MC, "Choice")?;
        let fallback = alt.optional(MC, "Fallback")?;
        require(
            alt.children.len() == 1 + usize::from(fallback.is_some()) && choice.children.len() == 1,
            "unsupported alternate native anchor branches",
        )?;
        if let Some(fallback) = fallback {
            require(fallback.children.len() <= 1 && fallback.children.iter().all(|node| node.is(XDR, "sp")), "unsupported native anchor fallback")?;
        }
        (choice.child(XDR, "graphicFrame")?, Some((alt, choice)))
    } else {
        (anchor.child(XDR, "graphicFrame")?, None)
    };
    let data = frame.child(DRAW, "graphic")?.child(DRAW, "graphicData")?;
    let [control_node] = data.children.as_slice() else { return Err("invalid native anchor graphic data".into()) };
    let control = drawing_control(control_node).ok_or_else(|| "unknown native anchor control".to_string())?;
    let ns = match control {
        Control::Slicer => SLICER_DRAW,
        Control::Timeline => TIMELINE_DRAW,
    };
    require(data.value("uri")? == ns && control_node.children.is_empty(), "native anchor graphic data does not match its control")?;
    control_node.tag.only(&[("", "name")])?;
    if let Some((alt, choice)) = choice {
        let bindings = decls(bytes, &[drawing, anchor, alt, choice])?;
        let required = choice.value("Requires")?;
        require(
            bindings.iter().any(|(prefix, uri)| {
                prefix == required && (uri == ns || control == Control::Slicer && uri == "http://schemas.microsoft.com/office/drawing/2010/main")
            }),
            "unsupported native anchor Choice requirement",
        )?;
    }
    let properties = frame.child(XDR, "nvGraphicFramePr")?.child(XDR, "cNvPr")?;
    let name = control_node.value("name")?;
    require(properties.number("id")? != 0 && properties.value("name")? == name, "native anchor shape identity does not match its control")?;
    anchor.visit(&mut |node| {
        require(
            !matches!(node.name.local.as_str(), "hlinkClick" | "hlinkHover" | "hlinkMouseOver" | "contentPart")
                && (!node.is(DRAW, "graphicData") || std::ptr::eq(node, data)),
            "native control anchor has an independent drawing dependency",
        )
    })?;
    Ok(Some((name, control, bounds)))
}

fn read_anchors(
    cx: &mut Ctx<'_>,
    parts: &mut Parts<'_>,
    sheet: &Node,
    rels: &[PreservedRel],
    mut controls: HashMap<String, Control>,
    drawing_locations: &mut HashSet<String>,
) -> Checked<Vec<String>> {
    let mut anchors = Vec::new();
    if let Some(mount) = sheet.optional(NS_MAIN, "drawing")? {
        let link = linked(rels, mount, DRAWING)?;
        all_mounted(rels, std::slice::from_ref(link), &["drawing"])?;
        content_type(parts.kept, &link.target, "application/vnd.openxmlformats-officedocument.drawing+xml")?;
        let bytes = part_bytes(cx, &link.target, MAX_BYTES, &mut parts.scan_left)?;
        let drawing = document(&bytes, |_| true)?;
        root_is(&drawing, XDR, "wsDr")?;
        let mut drawing_ids = HashSet::new();
        let mut native_bounds: Vec<DrawingBounds> = Vec::new();
        for (index, node) in drawing.children.iter().enumerate() {
            if let Some((name, kind, bounds)) = anchor_identity(&bytes, &drawing, node)? {
                require(drawing_locations.len() < MAX_PARTS, "too many native drawing anchors")?;
                require(controls.remove(name) == Some(kind), "native drawing anchor does not match a worksheet control")?;
                let mut alternate_ids = HashSet::new();
                node.visit(&mut |child| {
                    if child.is(XDR, "cNvPr") {
                        let id = child.number("id")?;
                        if id != 0 {
                            alternate_ids.insert(id);
                        }
                    }
                    Ok(())
                })?;
                require(alternate_ids.iter().all(|id| drawing_ids.insert(*id)), "duplicate ID across native drawing anchors")?;
                require(drawing_locations.insert(format!("{}, object {}", link.target, index + 1)), "duplicate native drawing owner")?;
                anchors.push(fragment(&bytes, node, &[&drawing], parts)?);
                native_bounds.push(bounds);
            } else if !native_bounds.is_empty() {
                // The writer emits regenerated objects before these anchors. That only preserves
                // the appearance when no later original object covered a retained control.
                let bounds = DrawingBounds::from_anchor(node).map_err(|_| "cannot verify drawing order around a native control")?;
                require(!native_bounds.iter().any(|native| native.overlaps(&bounds)), "a later drawing object overlaps a native control")?;
            }
        }
    } else {
        all_mounted(rels, &[], &["drawing"])?;
    }
    require(controls.is_empty(), "worksheet native control has no drawing anchor")?;
    Ok(anchors)
}

struct SheetMount {
    id: u32,
    owner: PreservedAnalysisOwner,
    node: Node,
    rels: Vec<PreservedRel>,
}

fn worksheet(cx: &mut Ctx<'_>, parts: &mut Parts<'_>, source_part: &str, id: u32) -> Checked<SheetMount> {
    let rels = relationships(cx, source_part, &mut parts.scan_left)?;
    let bytes = part_bytes(cx, source_part, MAX_BYTES, &mut parts.scan_left)?;
    let node = document(&bytes, |name| matches!(name.local.as_str(), "extLst" | "drawing" | "tableParts"))?;
    root_is(&node, NS_MAIN, "worksheet")?;
    let owner = mounts(&bytes, &node, &rels, SHEET_MOUNTS, parts)?;
    Ok(SheetMount { id, owner, node, rels })
}

pub(super) struct CapturedAnalysis {
    pub graph: PreservedNativeAnalysis,
    pub drawing_locations: HashSet<String>,
    pub control_locations: HashSet<String>,
}

pub(super) fn capture(
    cx: &mut Ctx<'_>,
    kept: &KeptReader,
    workbook_part: &str,
    book: &xml::El,
    file_to_model: &[Option<usize>],
) -> Checked<Option<CapturedAnalysis>> {
    // Native controls are the compatibility seam for this admission path. Ordinary PivotTables
    // continue through the editable model and need no additional package reads.
    if book.child("pivotCaches").is_none()
        || !raw_rels(cx, workbook_part)?.iter().any(|r| matches!(r.rel_type.as_str(), SLICER_CACHE | TIMELINE_CACHE))
    {
        return Ok(None);
    }
    require(cx.pkg.unique_parts(), "duplicate package part name")?;
    let mut parts = Parts { kept, bytes_left: MAX_BYTES.min(kept.remaining_budget()), scan_left: MAX_BYTES, parts: vec![], names: HashSet::new() };
    let bytes = part_bytes(cx, workbook_part, MAX_FRAGMENT, &mut parts.scan_left)?;
    let rels = relationships(cx, workbook_part, &mut parts.scan_left)?;
    let root = document(&bytes, |name| matches!(name.local.as_str(), "sheets" | "pivotCaches" | "extLst"))?;
    root_is(&root, NS_MAIN, "workbook")?;
    let mut workbook = mounts(&bytes, &root, &rels, WORKBOOK_MOUNTS, &mut parts)?;
    require(!workbook.rels.is_empty(), "native workbook controls are not mounted")?;
    let mut control_locations: HashSet<String> = workbook.rels.iter().map(|rel| format!("{workbook_part}: {}", rel.target)).collect();

    // Standard workbook cache IDs and x14 cache IDs are different namespaces. Preserve the
    // complete group only when every view and control resolves to this one definition.
    let cache_list = root.child(NS_MAIN, "pivotCaches")?;
    let cache_node = cache_list.child(NS_MAIN, "pivotCache")?;
    require(
        cache_list.children.len() == 1 && cache_node.children.is_empty(),
        "multiple workbook pivot caches are outside the native analysis graph",
    )?;
    cache_list.tag.only(&[])?;
    cache_node.tag.only(&[("", "cacheId"), (NS_REL, "id")])?;
    let link = linked(&rels, cache_node, CACHE)?;
    all_mounted(&rels, std::slice::from_ref(link), &["pivotCacheDefinition", "pivotTable", "pivotCacheRecords"])?;
    let cache = read_cache(cx, &mut parts, link, cache_node.number("cacheId")?)?;
    workbook.elements.push(PreservedXml { slot: "pivotCaches".into(), xml: fragment(&bytes, cache_list, &[&root], &mut parts)? });

    let sheet_entries = root.child(NS_MAIN, "sheets")?;
    require(
        sheet_entries.children.len() == file_to_model.len() && sheet_entries.children.len() <= MAX_PARTS,
        "native analysis worksheet map is incomplete or too large",
    )?;
    let mut ids = HashSet::new();
    let mut sheet_parts = HashSet::new();
    let mut model_sheets = HashSet::new();
    let mut mounted_sheets = Vec::new();
    let mut table_parts = HashSet::new();
    let mut source_tables = 0;
    let mut pivots = PivotNames::new();
    let mut owners = Vec::new();
    for (entry, model) in sheet_entries.children.iter().zip(file_to_model) {
        root_is(entry, NS_MAIN, "sheet")?;
        let id = entry.number("sheetId")?;
        require(id != 0 && ids.insert(id), "duplicate or invalid workbook sheet ID")?;
        let Some(model) = model else {
            // Chart sheets are handled separately. They cannot own worksheet analysis parts.
            let rel = linked(&rels, entry, CHART_SHEET)?;
            require(sheet_parts.insert(normalize(&rel.target)), "duplicate workbook sheet part")?;
            all_mounted(
                &relationships(cx, &rel.target, &mut parts.scan_left)?,
                &[],
                &["pivotTable", "slicer", "slicerCache", "timeline", "timelineCache"],
            )?;
            continue;
        };
        let rel = linked(&rels, entry, SHEET)?;
        require(model_sheets.insert(*model) && sheet_parts.insert(normalize(&rel.target)), "duplicate workbook worksheet part or model index")?;
        mounted_sheets.push(rel.clone());
        let mut sheet = worksheet(cx, &mut parts, &rel.target, id)?;
        control_locations.extend(sheet.owner.rels.iter().map(|control| format!("{}: {}", rel.target, control.target)));
        let mut names = HashSet::new();
        let mut folded_names = HashSet::new();
        for link in sheet.rels.iter().filter(|r| is_kind(r, &["pivotTable"])) {
            require(link.rel_type == PIVOT && !link.external, "foreign or external worksheet PivotTable")?;
            let name = pivot_name(cx, &mut parts, link, &cache)?;
            require(folded_names.insert(name.to_ascii_lowercase()), "duplicate worksheet PivotTable name")?;
            names.insert(name);
            sheet.owner.rels.push(link.clone());
        }
        if !names.is_empty() {
            pivots.insert(id, names);
        }
        let mut mounted_tables = Vec::new();
        if let Some(tables) = sheet.node.optional(NS_MAIN, "tableParts")? {
            require(tables.number("count")? as usize == tables.children.len(), "invalid worksheet table mount count")?;
            for table in &tables.children {
                root_is(table, NS_MAIN, "tablePart")?;
                let link = linked(&sheet.rels, table, TABLE)?;
                require(table_parts.insert(normalize(&link.target)), "duplicate worksheet table part")?;
                mounted_tables.push(link.clone());
                source_tables += usize::from(table_source(cx, &mut parts, link, &cache)?);
            }
        }
        all_mounted(&sheet.rels, &mounted_tables, &["table"])?;
        owners.push(sheet);
    }
    all_mounted(&rels, &mounted_sheets, &["worksheet"])?;
    require(source_tables == 1 && !pivots.is_empty(), "native analysis has no unambiguous local source table and PivotTables")?;

    let mut caches = HashMap::new();
    let mut cache_names = HashSet::new();
    for link in &workbook.rels {
        let (name, control) = control_cache(cx, &mut parts, link, &cache, &pivots)?;
        require(cache_names.insert(name.to_ascii_lowercase()), "duplicate native control cache name")?;
        caches.insert(name, control);
    }
    workbook.rels.push(link.clone());
    let mut used = HashSet::new();
    let mut control_names_seen = HashSet::new();
    let mut drawing_locations = HashSet::new();
    let mut sheets = Vec::new();
    for sheet in owners {
        let mut controls = HashMap::new();
        for link in sheet.owner.rels.iter().filter(|r| r.rel_type != PIVOT) {
            control_names(cx, &mut parts, link, &caches, &mut controls, &mut used)?;
        }
        for (name, kind) in &controls {
            require(control_names_seen.insert((*kind, name.to_lowercase())), "duplicate workbook control name")?;
        }
        let anchors = read_anchors(cx, &mut parts, &sheet.node, &sheet.rels, controls, &mut drawing_locations)?;
        sheets.push(PreservedAnalysisSheet { sheet_id: sheet.id, owner: sheet.owner, anchors });
    }
    require(used.len() == caches.len(), "a native control cache has no mounted worksheet control")?;

    let styles: Vec<_> = rels.iter().filter(|r| is_kind(r, &["styles"])).collect();
    let [styles] = styles.as_slice() else { return Err("native analysis requires one source style part".into()) };
    require(!styles.external && styles.rel_type == STYLES, "invalid native analysis style relationship")?;
    content_type(kept, &styles.target, &standard_type("styles"))?;
    require(relationships(cx, &styles.target, &mut parts.scan_left)?.is_empty(), "native analysis style part has an independent dependency")?;
    let styles = part_bytes(cx, &styles.target, MAX_BYTES, &mut parts.bytes_left)?;
    let graph = PreservedNativeAnalysis { signature: [0; 32], styles: Arc::from(styles), parts: parts.parts, workbook, sheets };
    Ok(Some(CapturedAnalysis { graph, drawing_locations, control_locations }))
}
